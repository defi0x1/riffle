use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

use crate::types::tier;

// Pools that have never produced an indicators_10m row at all -- neither a freshly discovered
// pool nor one that has simply never been ranked highly enough to be sampled. Selecting from
// this set for promotion keeps a bad screening estimate from permanently hiding a pool the
// ranking has never actually looked at.
pub async fn never_measured_pools(pool: &PgPool, limit: i64) -> eyre::Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"
        SELECT p.pool_address
        FROM pools p
        WHERE p.tier = $1
          AND NOT EXISTS (
              SELECT 1 FROM indicators_10m i WHERE i.pool_address = p.pool_address
          )
        ORDER BY p.created_at DESC
        LIMIT $2
        "#,
        tier::UNIVERSE,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Selecting never-measured pools")?;

    Ok(rows.into_iter().map(|r| r.pool_address).collect())
}

// Only flips pools not already at tier 1, so a pool re-promoted on a later sweep keeps its
// original tier_changed_at rather than having the clock reset by every promotion tick it
// happens to still qualify for.
pub async fn promote_pools(
    pool: &PgPool,
    pool_addresses: &[String],
    at: DateTime<Utc>,
) -> eyre::Result<u64> {
    if pool_addresses.is_empty() {
        return Ok(0);
    }

    let result = sqlx::query!(
        r#"
        UPDATE pools
        SET tier = $1, tier_changed_at = $2
        WHERE pool_address = ANY($3) AND tier <> $1
        "#,
        tier::WATCHED,
        at,
        pool_addresses,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Promoting {} pools", pool_addresses.len()))?;

    Ok(result.rows_affected())
}

// A pool with an open paper position is exempt from demotion: ending its bin-state
// subscription mid-outcome would corrupt the measurement the position exists to produce. The
// exemption is enforced in the UPDATE itself via idx_paper_positions_open, not left to the
// caller to check, so a demotion sweep can never race an open position out of measurement.
// Returns the pool addresses that were actually demoted.
pub async fn demote_pools(
    pool: &PgPool,
    pool_addresses: &[String],
    at: DateTime<Utc>,
) -> eyre::Result<Vec<String>> {
    if pool_addresses.is_empty() {
        return Ok(Vec::new());
    }

    let rows = sqlx::query!(
        r#"
        UPDATE pools
        SET tier = $1, tier_changed_at = $2
        WHERE pool_address = ANY($3)
          AND tier <> $1
          AND NOT EXISTS (
              SELECT 1 FROM paper_positions pp
              WHERE pp.pool_address = pools.pool_address AND pp.closed_at IS NULL
          )
        RETURNING pool_address
        "#,
        tier::UNIVERSE,
        at,
        pool_addresses,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Demoting up to {} pools", pool_addresses.len()))?;

    Ok(rows.into_iter().map(|r| r.pool_address).collect())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use crate::write::paper_positions::{NewPaperPosition, open_paper_position};
    use crate::write::pools::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    async fn ensure_pool(pool: &PgPool, pool_address: &str, tier_value: i16) {
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

        sqlx::query!(
            "UPDATE pools SET tier = $2 WHERE pool_address = $1",
            pool_address,
            tier_value
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_demote_pools_exempts_open_position() {
        let pool = test_pool().await;
        let pool_address = "pool_tier_demote_exempt";
        ensure_pool(&pool, pool_address, tier::WATCHED).await;

        open_paper_position(
            &pool,
            &NewPaperPosition {
                id: Uuid::new_v4(),
                signal_id: None,
                pool_address: pool_address.to_string(),
                venue: crate::types::venue::DLMM,
                opened_at: Utc::now(),
                regime: Some("V1".to_string()),
                entry_price: Some(1.5),
                entry_active_bin: Some(100),
                lower_bin: Some(90),
                upper_bin: Some(110),
                shape: Some("uniform".to_string()),
                size_usd: Some(Decimal::new(1_000, 0)),
                size_per_bin: Some(Decimal::new(50, 0)),
                predicted: None,
            },
        )
        .await
        .unwrap();

        let demoted = demote_pools(&pool, &[pool_address.to_string()], Utc::now())
            .await
            .unwrap();

        assert!(demoted.is_empty());

        let row = sqlx::query!(
            "SELECT tier FROM pools WHERE pool_address = $1",
            pool_address
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(row.tier, tier::WATCHED);
    }
}
