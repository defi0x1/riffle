use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct LatestConfig {
    pub config_hash: String,
    pub ts: DateTime<Utc>,
}

// config_hash is stamped on every signal at write time, and a scoring tick reuses the same
// process-wide hash for every pool it evaluates -- so the newest row across all pools and
// kinds is the best available answer to "what configuration is currently applied", without
// a dedicated config-versions table of its own.
pub async fn latest_config(pool: &PgPool) -> eyre::Result<Option<LatestConfig>> {
    let row = sqlx::query_as!(
        LatestConfig,
        r#"
        SELECT config_hash, ts
        FROM signals
        ORDER BY ts DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| "Querying latest config hash")?;

    Ok(row)
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use crate::types::venue;
    use crate::write::{NewDlmmPoolParams, NewPool, NewSignal, insert_signal, upsert_dlmm_pool};
    use rust_decimal::Decimal;
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

    // Other suites in this crate write to `signals` concurrently against the same shared
    // database, so "the newest row" cannot be asserted against a fixed fixture -- this signal
    // is timestamped far enough in the future that it is guaranteed to be newest regardless of
    // what else is running.
    #[tokio::test]
    async fn test_latest_config_returns_the_newest_signal_hash() {
        let pool = test_pool().await;
        let pool_address = "pool_latest_config";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        insert_signal(
            &pool,
            &NewSignal {
                id: Uuid::new_v4(),
                ts: Utc::now() + chrono::Duration::days(3650),
                pool_address: pool_address.to_string(),
                venue: venue::DLMM,
                timeframe: "5m".to_string(),
                kind: "INFO".to_string(),
                regime: None,
                numbers: None,
                config_hash: "hash_latest_config_test".to_string(),
                expires_at: None,
            },
        )
        .await
        .unwrap();

        let latest = latest_config(&pool).await.unwrap().expect("expected a row");
        assert_eq!(latest.config_hash, "hash_latest_config_test");
    }
}
