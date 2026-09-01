use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

// The signals worker's cooldown key is (pool_address, timeframe, kind), and every broadcast
// is already a row in this table -- so surviving a restart needs no dedicated state table,
// just a query over what is already persisted, unlike regime_state/volatility_state (0023)
// which round-trip values the signals table has no other place to carry.
pub async fn last_signal_broadcast(
    pool: &PgPool,
    pool_address: &str,
    timeframe: &str,
    kind: &str,
) -> eyre::Result<Option<DateTime<Utc>>> {
    let row = sqlx::query!(
        r#"
        SELECT ts
        FROM signals
        WHERE pool_address = $1 AND timeframe = $2 AND kind = $3
        ORDER BY ts DESC
        LIMIT 1
        "#,
        pool_address,
        timeframe,
        kind,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| {
        format!("Loading last signal broadcast for {pool_address}/{timeframe}/{kind}")
    })?;

    Ok(row.map(|r| r.ts))
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use crate::types::venue;
    use crate::write::{NewDlmmPoolParams, NewPool, NewSignal, insert_signal, upsert_dlmm_pool};
    use rust_decimal::Decimal;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

    async fn ensure_pool(pool: &PgPool, pool_address: &str) {
        let now = Utc::now();
        upsert_dlmm_pool(
            pool,
            &NewPool {
                pool_address: pool_address.to_string(),
                venue: venue::DLMM,
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

    fn sample_signal(
        pool_address: &str,
        ts: DateTime<Utc>,
        timeframe: &str,
        kind: &str,
    ) -> NewSignal {
        NewSignal {
            id: Uuid::new_v4(),
            ts,
            pool_address: pool_address.to_string(),
            venue: venue::DLMM,
            timeframe: timeframe.to_string(),
            kind: kind.to_string(),
            regime: Some("V1".to_string()),
            numbers: None,
            config_hash: "hash_cooldown_test".to_string(),
            expires_at: None,
        }
    }

    async fn fresh_connection() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@localhost:55432/feefarming".to_string()
        });
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap_or_else(|e| panic!("opening a fresh connection pool: {e}"))
    }

    #[tokio::test]
    async fn test_last_signal_broadcast_is_none_with_no_matching_row() {
        let pool = test_pool().await;
        let pool_address = "pool_cooldown_none";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let last = last_signal_broadcast(&pool, pool_address, "1h", "POTENTIAL")
            .await
            .unwrap();
        assert!(last.is_none());
    }

    #[tokio::test]
    async fn test_last_signal_broadcast_filters_by_timeframe_and_kind() {
        let pool = test_pool().await;
        let pool_address = "pool_cooldown_filter";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let now = Utc::now();
        // A different kind and a different timeframe at the same pool, both newer than the
        // row the test actually cares about -- neither may leak into the answer.
        insert_signal(&pool, &sample_signal(pool_address, now, "1h", "INFO"))
            .await
            .unwrap();
        insert_signal(&pool, &sample_signal(pool_address, now, "4h", "POTENTIAL"))
            .await
            .unwrap();
        let wanted_ts = now - chrono::Duration::minutes(10);
        insert_signal(
            &pool,
            &sample_signal(pool_address, wanted_ts, "1h", "POTENTIAL"),
        )
        .await
        .unwrap();

        let last = last_signal_broadcast(&pool, pool_address, "1h", "POTENTIAL")
            .await
            .unwrap()
            .expect("expected a matching row");
        assert_eq!(last, wanted_ts);
    }

    #[tokio::test]
    async fn test_last_signal_broadcast_returns_the_newest_of_several_matches() {
        let pool = test_pool().await;
        let pool_address = "pool_cooldown_newest";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let now = Utc::now();
        let older = now - chrono::Duration::hours(2);
        insert_signal(
            &pool,
            &sample_signal(pool_address, older, "5m", "DEGRADING"),
        )
        .await
        .unwrap();
        insert_signal(&pool, &sample_signal(pool_address, now, "5m", "DEGRADING"))
            .await
            .unwrap();

        let last = last_signal_broadcast(&pool, pool_address, "5m", "DEGRADING")
            .await
            .unwrap()
            .expect("expected a matching row");
        assert_eq!(last, now);
    }

    // The actual requirement: a scorer restart must not forget a recent broadcast. Write the
    // cooldown-relevant row, drop every handle this test has held, then open a connection
    // that has never seen this process's state -- the same thing a restarted worker does --
    // and confirm the broadcast is still visible and still inside the cooldown window.
    #[tokio::test]
    async fn test_cooldown_state_survives_a_restart() {
        let pool = test_pool().await;
        let pool_address = "pool_cooldown_restart";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let broadcast_at = Utc::now();
        insert_signal(
            &pool,
            &sample_signal(pool_address, broadcast_at, "1h", "POTENTIAL"),
        )
        .await
        .unwrap();

        drop(pool);
        let restarted = fresh_connection().await;

        let last = last_signal_broadcast(&restarted, pool_address, "1h", "POTENTIAL")
            .await
            .expect("loading cooldown state after restart")
            .expect("broadcast row must still exist after restart");

        assert_eq!(last, broadcast_at);
        let cooldown_window = chrono::Duration::hours(1);
        assert!(
            Utc::now().signed_duration_since(last) < cooldown_window,
            "broadcast must still be inside the cooldown window after a restart"
        );
    }
}
