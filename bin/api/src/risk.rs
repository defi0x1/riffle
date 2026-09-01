//! The refusal checks item 6 of this task asks for: an unknown pool, a pool failing the risk
//! gate, or an amount beyond the configured cap. Wallet ownership is enforced by
//! `wallet_resolve` (every action always acts as the caller's own resolved wallet, never a
//! wallet named in the request) and by an explicit ownership check wherever a request also
//! names an existing position (see `tx_build`).

use storage::queries::PoolDetail;
use storage::types::tier;

use crate::error::ApiError;
use crate::state::AppState;

/// A pool is farmable only if this service already tracks it and the scorer's own screening
/// has promoted it to the watched tier without it being blacklisted -- the same set
/// `queries::watch_set` (bin/bot, bin/indexer) already treats as "safe enough to act on",
/// reused here rather than inventing a second notion of "risky".
pub async fn pool_risk_gate(state: &AppState, pool_address: &str) -> Result<PoolDetail, ApiError> {
    let detail = storage::queries::pool_detail(&state.db, pool_address)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| {
            ApiError::refused(
                "unknown_pool",
                format!("{pool_address} is not a pool this service tracks"),
            )
        })?;

    if detail.pool.is_blacklisted {
        return Err(ApiError::refused(
            "pool_blacklisted",
            "This pool is blacklisted and cannot be farmed",
        ));
    }

    if detail.pool.tier != tier::WATCHED {
        return Err(ApiError::refused(
            "pool_not_watched",
            "This pool has not passed the risk screening required before farming",
        ));
    }

    Ok(detail)
}

pub fn check_amount_cap(cap: u64, amount_x: u64, amount_y: u64) -> Result<(), ApiError> {
    if amount_x > cap || amount_y > cap {
        return Err(ApiError::refused(
            "amount_exceeds_cap",
            format!("Deposit amount exceeds the configured per-side cap of {cap} raw base units"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amount_within_cap_is_allowed() {
        assert!(check_amount_cap(1_000, 500, 999).is_ok());
    }

    #[test]
    fn test_amount_x_over_cap_is_refused() {
        let err = check_amount_cap(1_000, 1_001, 0).unwrap_err();
        assert!(matches!(err, ApiError::Refused { code: "amount_exceeds_cap", .. }));
    }

    #[test]
    fn test_amount_y_over_cap_is_refused() {
        let err = check_amount_cap(1_000, 0, 1_001).unwrap_err();
        assert!(matches!(err, ApiError::Refused { code: "amount_exceeds_cap", .. }));
    }

    #[test]
    fn test_amount_exactly_at_cap_is_allowed() {
        assert!(check_amount_cap(1_000, 1_000, 1_000).is_ok());
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use chrono::Utc;
    use rust_decimal::Decimal;

    use super::*;
    use crate::test_support::{test_pool, test_state};

    async fn ensure_pool(pool: &sqlx::PgPool, pool_address: &str, is_blacklisted: bool) {
        let now = Utc::now();
        storage::write::upsert_dlmm_pool(
            pool,
            &storage::write::NewPool {
                pool_address: pool_address.to_string(),
                venue: storage::types::venue::DLMM,
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
                is_blacklisted,
                launchpad: None,
                tags: vec![],
                updated_at: now,
            },
            &storage::write::NewDlmmPoolParams {
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
    async fn test_unknown_pool_is_refused() {
        let pool = test_pool().await;
        let state = test_state(pool);

        let err = pool_risk_gate(&state, "pool_risk_gate_unknown_1111111111111111")
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Refused { code: "unknown_pool", .. }));
    }

    #[tokio::test]
    async fn test_pool_not_yet_watched_is_refused() {
        let pool_address = "pool_risk_gate_not_watched_111111111111";
        let pool = test_pool().await;
        ensure_pool(&pool, pool_address, false).await;
        let state = test_state(pool);

        let err = pool_risk_gate(&state, pool_address).await.unwrap_err();
        assert!(matches!(err, ApiError::Refused { code: "pool_not_watched", .. }));
    }

    #[tokio::test]
    async fn test_blacklisted_pool_is_refused_even_if_watched() {
        let pool_address = "pool_risk_gate_blacklisted_1111111111111";
        let pool = test_pool().await;
        ensure_pool(&pool, pool_address, true).await;
        storage::write::promote_pools(&pool, &[pool_address.to_string()], Utc::now())
            .await
            .unwrap();
        let state = test_state(pool);

        let err = pool_risk_gate(&state, pool_address).await.unwrap_err();
        assert!(matches!(err, ApiError::Refused { code: "pool_blacklisted", .. }));
    }

    #[tokio::test]
    async fn test_watched_unblacklisted_pool_passes_the_gate() {
        let pool_address = "pool_risk_gate_watched_1111111111111111";
        let pool = test_pool().await;
        ensure_pool(&pool, pool_address, false).await;
        storage::write::promote_pools(&pool, &[pool_address.to_string()], Utc::now())
            .await
            .unwrap();
        let state = test_state(pool);

        let detail = pool_risk_gate(&state, pool_address).await.unwrap();
        assert_eq!(detail.pool.pool_address, pool_address);
    }
}
