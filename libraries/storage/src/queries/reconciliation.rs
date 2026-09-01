use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StalePoolState {
    pub pool_address: String,
    pub state_slot: i64,
    pub observed_slot: i64,
}

// Splits two facts that are easy to conflate: the slot at which we last *observed* a pool
// change (cheap -- already decoded from a swap, a liquidity event or a fee-param update, no
// extra fetch) against the slot our own full account snapshot is stamped with (expensive -- a
// getMultipleAccounts round trip). Wherever state_slot < observed_slot, our snapshot has fallen
// behind something we already know happened, and that pool needs a repair fetch.
//
// `since` bounds every side of the query to recent chunks so a repair sweep only ever touches
// the hot tail of each hypertable, not the full retained history -- each per-table lookup
// matches an existing (pool_address, ts DESC) index, and `since` gives the planner a chunk
// exclusion boundary on top of it.
pub async fn stale_pool_state(
    pool: &PgPool,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<StalePoolState>> {
    let rows = sqlx::query_as!(
        StalePoolState,
        r#"
        WITH state AS (
            SELECT DISTINCT ON (pool_address) pool_address, slot AS state_slot
            FROM pool_snapshots
            WHERE ts >= $1
            ORDER BY pool_address, ts DESC
        ),
        observed AS (
            SELECT pool_address, max(slot) AS observed_slot
            FROM (
                SELECT pool_address, slot FROM swaps WHERE ts >= $1
                UNION ALL
                SELECT pool_address, slot FROM liquidity_events WHERE ts >= $1
                UNION ALL
                SELECT pool_address, slot FROM fee_param_updates WHERE ts >= $1
            ) recent
            GROUP BY pool_address
        )
        SELECT
            o.pool_address AS "pool_address!",
            s.state_slot AS "state_slot!",
            o.observed_slot AS "observed_slot!"
        FROM observed o
        JOIN state s USING (pool_address)
        WHERE s.state_slot < o.observed_slot
        ORDER BY (o.observed_slot - s.state_slot) DESC
        "#,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying stale pool state")?;

    Ok(rows)
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use crate::write::{
        NewDlmmPoolParams, NewPool, NewPoolSnapshot, NewSwap, insert_pool_snapshots, insert_swaps,
        upsert_dlmm_pool,
    };
    use rust_decimal::Decimal;

    async fn ensure_pool(pool: &PgPool, pool_address: &str) {
        let now = Utc::now();
        upsert_dlmm_pool(
            pool,
            &NewPool {
                pool_address: pool_address.to_string(),
                venue: crate::types::venue::DLMM,
                token_x: "So11111111111111111111111111111111111111112".to_string(),
                token_y: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                base_fee_bps: Decimal::new(100, 2),
                protocol_share_bps: 500,
                tvl_usd: None,
                status: 0,
                creator: None,
                activation_point: None,
                created_at: now,
                first_liquidity_at: None,
                is_blacklisted: false,
                launchpad: None,
                tags: vec![],
                updated_at: now,
            },
            &NewDlmmPoolParams {
                pool_address: pool_address.to_string(),
                bin_step: 20,
                base_factor: 10_000,
                filter_period: 30,
                decay_period: 600,
                reduction_factor: 5_000,
                variable_fee_control: 40_000,
                max_volatility_accumulator: 350_000,
                collect_fee_mode: 0,
                reward_mint_x: None,
                reward_mint_y: None,
            },
        )
        .await
        .unwrap();
    }

    fn sample_swap(pool_address: &str, ts: DateTime<Utc>, slot: i64, signature: &str) -> NewSwap {
        NewSwap {
            pool_address: pool_address.to_string(),
            ts,
            slot,
            signature: signature.to_string(),
            ix_index: 0,
            signer: "signer1111111111111111111111111111111111".to_string(),
            swap_for_y: true,
            amount_in_raw: Decimal::new(1_000_000, 0),
            amount_out_raw: Decimal::new(2_000_000, 0),
            amount_in: Decimal::new(1, 0),
            amount_out: Decimal::new(2, 0),
            start_bin_id: 100,
            end_bin_id: 101,
            start_price: Some(Decimal::new(150, 2)),
            end_price: Some(Decimal::new(151, 2)),
            fee_raw: Decimal::new(1_000, 0),
            protocol_fee_raw: Decimal::new(100, 0),
            host_fee_raw: None,
            fee_bps: Decimal::new(30, 2),
            volume_usd: Some(Decimal::new(15, 1)),
            trade_fee_usd: Some(Decimal::new(9, 3)),
            protocol_fee_usd: Some(Decimal::new(1, 3)),
        }
    }

    #[tokio::test]
    async fn test_stale_pool_state_flags_a_snapshot_behind_an_observed_swap() {
        let pool = test_pool().await;
        let pool_address = "pool_reconciliation_stale";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let base = Utc::now();

        insert_pool_snapshots(
            &pool,
            &[NewPoolSnapshot {
                pool_address: pool_address.to_string(),
                ts: base,
                slot: 100,
                price: 1.5,
                reserve_x_raw: None,
                reserve_y_raw: None,
                tvl_usd: None,
                active_tvl_usd: None,
                total_fee_bps: Decimal::new(30, 2),
            }],
        )
        .await
        .unwrap();

        insert_swaps(
            &pool,
            &[sample_swap(
                pool_address,
                base + chrono::Duration::seconds(1),
                150,
                "sig_reconciliation_stale",
            )],
        )
        .await
        .unwrap();

        let since = base - chrono::Duration::minutes(5);
        let stale = stale_pool_state(&pool, since).await.unwrap();

        let found = stale
            .iter()
            .find(|s| s.pool_address == pool_address)
            .expect("pool should be reported stale");
        assert_eq!(found.state_slot, 100);
        assert_eq!(found.observed_slot, 150);
    }

    #[tokio::test]
    async fn test_stale_pool_state_omits_a_snapshot_already_current() {
        let pool = test_pool().await;
        let pool_address = "pool_reconciliation_current";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let base = Utc::now();

        insert_pool_snapshots(
            &pool,
            &[NewPoolSnapshot {
                pool_address: pool_address.to_string(),
                ts: base,
                slot: 200,
                price: 1.5,
                reserve_x_raw: None,
                reserve_y_raw: None,
                tvl_usd: None,
                active_tvl_usd: None,
                total_fee_bps: Decimal::new(30, 2),
            }],
        )
        .await
        .unwrap();

        insert_swaps(
            &pool,
            &[sample_swap(
                pool_address,
                base - chrono::Duration::seconds(1),
                150,
                "sig_reconciliation_current",
            )],
        )
        .await
        .unwrap();

        let since = base - chrono::Duration::minutes(5);
        let stale = stale_pool_state(&pool, since).await.unwrap();

        assert!(stale.iter().all(|s| s.pool_address != pool_address));
    }
}
