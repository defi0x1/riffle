use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgExecutor;

#[derive(Clone, Debug)]
pub struct NewPool {
    pub pool_address: String,
    pub venue: i16,
    pub token_x: String,
    pub token_y: String,
    pub base_fee_bps: Decimal,
    pub protocol_share_bps: i32,
    pub tvl_usd: Option<Decimal>,
    pub status: i16,
    pub creator: Option<String>,
    pub activation_point: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub first_liquidity_at: Option<DateTime<Utc>>,
    pub is_blacklisted: bool,
    pub launchpad: Option<String>,
    pub tags: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct NewDlmmPoolParams {
    pub pool_address: String,
    pub bin_step: i16,
    pub base_factor: i32,
    pub filter_period: i32,
    pub decay_period: i32,
    pub reduction_factor: i32,
    pub variable_fee_control: i32,
    pub max_volatility_accumulator: i32,
    pub collect_fee_mode: i16,
    pub reward_mint_x: Option<String>,
    pub reward_mint_y: Option<String>,
}

// Metadata refresh, not tier management: tier and tier_changed_at are deliberately absent from
// the UPDATE clause and owned exclusively by write::tier, so a routine metadata sync (screening
// scan, tvl_usd cache refresh) can never accidentally undo a promotion or demotion decided
// elsewhere.
pub async fn upsert_pool<'e, E: PgExecutor<'e>>(executor: E, row: &NewPool) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO pools (
            pool_address, venue, token_x, token_y, base_fee_bps, protocol_share_bps,
            tvl_usd, status, creator, activation_point, created_at, first_liquidity_at,
            is_blacklisted, launchpad, tags, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        ON CONFLICT (pool_address) DO UPDATE SET
            token_x            = EXCLUDED.token_x,
            token_y            = EXCLUDED.token_y,
            base_fee_bps       = EXCLUDED.base_fee_bps,
            protocol_share_bps = EXCLUDED.protocol_share_bps,
            tvl_usd            = EXCLUDED.tvl_usd,
            status             = EXCLUDED.status,
            creator            = EXCLUDED.creator,
            activation_point   = EXCLUDED.activation_point,
            first_liquidity_at = COALESCE(pools.first_liquidity_at, EXCLUDED.first_liquidity_at),
            is_blacklisted     = EXCLUDED.is_blacklisted,
            launchpad          = EXCLUDED.launchpad,
            tags               = EXCLUDED.tags,
            updated_at         = EXCLUDED.updated_at
        "#,
        row.pool_address,
        row.venue,
        row.token_x,
        row.token_y,
        row.base_fee_bps,
        row.protocol_share_bps,
        row.tvl_usd,
        row.status,
        row.creator,
        row.activation_point,
        row.created_at,
        row.first_liquidity_at,
        row.is_blacklisted,
        row.launchpad,
        &row.tags,
        row.updated_at,
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Upserting pool {}", row.pool_address))?;

    Ok(())
}

pub async fn upsert_dlmm_pool_params<'e, E: PgExecutor<'e>>(
    executor: E,
    row: &NewDlmmPoolParams,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO dlmm_pool_params (
            pool_address, bin_step, base_factor, filter_period, decay_period,
            reduction_factor, variable_fee_control, max_volatility_accumulator,
            collect_fee_mode, reward_mint_x, reward_mint_y
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (pool_address) DO UPDATE SET
            bin_step                   = EXCLUDED.bin_step,
            base_factor                = EXCLUDED.base_factor,
            filter_period              = EXCLUDED.filter_period,
            decay_period               = EXCLUDED.decay_period,
            reduction_factor           = EXCLUDED.reduction_factor,
            variable_fee_control       = EXCLUDED.variable_fee_control,
            max_volatility_accumulator = EXCLUDED.max_volatility_accumulator,
            collect_fee_mode           = EXCLUDED.collect_fee_mode,
            reward_mint_x              = EXCLUDED.reward_mint_x,
            reward_mint_y              = EXCLUDED.reward_mint_y
        "#,
        row.pool_address,
        row.bin_step,
        row.base_factor,
        row.filter_period,
        row.decay_period,
        row.reduction_factor,
        row.variable_fee_control,
        row.max_volatility_accumulator,
        row.collect_fee_mode,
        row.reward_mint_x,
        row.reward_mint_y,
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Upserting dlmm_pool_params for {}", row.pool_address))?;

    Ok(())
}

// A pool should never be visible in `pools` without its DLMM satellite row, and vice versa, so
// discovery writes both inside one transaction.
pub async fn upsert_dlmm_pool(
    pool: &sqlx::PgPool,
    shared: &NewPool,
    params: &NewDlmmPoolParams,
) -> eyre::Result<()> {
    let mut tx = pool
        .begin()
        .await
        .wrap_err_with(|| "Starting dlmm pool upsert transaction")?;

    upsert_pool(&mut *tx, shared).await?;
    upsert_dlmm_pool_params(&mut *tx, params).await?;

    tx.commit()
        .await
        .wrap_err_with(|| "Committing dlmm pool upsert transaction")?;

    Ok(())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use chrono::Utc;

    fn sample(pool_address: &str) -> (NewPool, NewDlmmPoolParams) {
        let now = Utc::now();
        (
            NewPool {
                pool_address: pool_address.to_string(),
                venue: crate::types::venue::DLMM,
                token_x: "So11111111111111111111111111111111111111112".to_string(),
                token_y: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                base_fee_bps: Decimal::new(100, 2),
                protocol_share_bps: 500,
                tvl_usd: Some(Decimal::new(123_456, 2)),
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
            NewDlmmPoolParams {
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
    }

    #[tokio::test]
    async fn test_upsert_dlmm_pool_is_idempotent() {
        let pool = test_pool().await;
        let (shared, params) = sample("pool_upsert_idempotent");

        upsert_dlmm_pool(&pool, &shared, &params).await.unwrap();
        upsert_dlmm_pool(&pool, &shared, &params).await.unwrap();

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM pools WHERE pool_address = $1",
            shared.pool_address
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(count, Some(1));
    }
}
