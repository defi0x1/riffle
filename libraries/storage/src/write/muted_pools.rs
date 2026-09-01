use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

// Re-muting the same pool for the same chat always means "reset the clock", never "stack a
// second mute" -- an upsert keyed on (pool_address, chat_id) is the whole implementation.
pub async fn mute_pool(
    pool: &PgPool,
    pool_address: &str,
    chat_id: i64,
    until: DateTime<Utc>,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO muted_pools (pool_address, chat_id, until)
        VALUES ($1, $2, $3)
        ON CONFLICT (pool_address, chat_id) DO UPDATE SET until = EXCLUDED.until
        "#,
        pool_address,
        chat_id,
        until,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Muting {pool_address} for chat {chat_id}"))?;

    Ok(())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::queries::muted_pool_addresses;
    use crate::test_support::test_pool;
    use crate::write::pools::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};
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

    #[tokio::test]
    async fn test_mute_pool_is_visible_before_expiry() {
        let pool = test_pool().await;
        let pool_address = "pool_mute_active";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let chat_id = -1001;
        mute_pool(
            &pool,
            pool_address,
            chat_id,
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

        let muted = muted_pool_addresses(&pool, chat_id).await.unwrap();
        assert!(muted.contains(&pool_address.to_string()));
    }

    #[tokio::test]
    async fn test_expired_mute_falls_out_of_the_query() {
        let pool = test_pool().await;
        let pool_address = "pool_mute_expired";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let chat_id = -1002;
        mute_pool(
            &pool,
            pool_address,
            chat_id,
            Utc::now() - chrono::Duration::seconds(1),
        )
        .await
        .unwrap();

        let muted = muted_pool_addresses(&pool, chat_id).await.unwrap();
        assert!(!muted.contains(&pool_address.to_string()));
    }

    #[tokio::test]
    async fn test_remuting_extends_the_expiry() {
        let pool = test_pool().await;
        let pool_address = "pool_mute_remute";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let chat_id = -1003;
        mute_pool(
            &pool,
            pool_address,
            chat_id,
            Utc::now() - chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
        mute_pool(
            &pool,
            pool_address,
            chat_id,
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

        let muted = muted_pool_addresses(&pool, chat_id).await.unwrap();
        assert!(muted.contains(&pool_address.to_string()));
    }

    #[tokio::test]
    async fn test_mute_is_scoped_to_its_own_chat() {
        let pool = test_pool().await;
        let pool_address = "pool_mute_other_chat";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let muting_chat = -1004;
        let other_chat = -1005;
        mute_pool(
            &pool,
            pool_address,
            muting_chat,
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

        let muted = muted_pool_addresses(&pool, other_chat).await.unwrap();
        assert!(!muted.contains(&pool_address.to_string()));
    }
}
