use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::write::IndicatorRow;

#[derive(Clone, Debug)]
pub struct PoolSummary {
    pub pool_address: String,
    pub venue: i16,
    pub token_x: String,
    pub token_y: String,
    pub base_fee_bps: Decimal,
    pub protocol_share_bps: i32,
    pub tvl_usd: Option<Decimal>,
    pub status: i16,
    pub tier: i16,
    pub tier_changed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub first_liquidity_at: Option<DateTime<Utc>>,
    pub is_blacklisted: bool,
    pub launchpad: Option<String>,
    pub bin_step: i16,
    pub base_factor: i32,
    pub collect_fee_mode: i16,
}

// Every timeframe a pool has, in one struct -- the shape /pool_detail and a future HTTP handler
// both render, built once here rather than reassembled by every consumer.
#[derive(Clone, Debug)]
pub struct PoolDetail {
    pub pool: PoolSummary,
    pub m5: Option<IndicatorRow>,
    pub m10: Option<IndicatorRow>,
    pub h1: Option<IndicatorRow>,
    pub h4: Option<IndicatorRow>,
    pub h24: Option<IndicatorRow>,
}

async fn fetch_pool_summary(
    pool: &PgPool,
    pool_address: &str,
) -> eyre::Result<Option<PoolSummary>> {
    let row = sqlx::query_as!(
        PoolSummary,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.base_fee_bps, p.protocol_share_bps,
            p.tvl_usd, p.status, p.tier, p.tier_changed_at, p.created_at, p.first_liquidity_at,
            p.is_blacklisted, p.launchpad,
            d.bin_step, d.base_factor, d.collect_fee_mode
        FROM pools p
        JOIN dlmm_pool_params d ON d.pool_address = p.pool_address
        WHERE p.pool_address = $1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching pool summary for {pool_address}"))?;

    Ok(row)
}

async fn fetch_latest_m5(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<IndicatorRow>> {
    let row = sqlx::query_as!(
        IndicatorRow,
        r#"
        SELECT
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        FROM indicators_5m
        WHERE pool_address = $1
        ORDER BY bucket_start DESC
        LIMIT 1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching latest indicators_5m row for {pool_address}"))?;

    Ok(row)
}

async fn fetch_latest_m10(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<IndicatorRow>> {
    let row = sqlx::query_as!(
        IndicatorRow,
        r#"
        SELECT
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        FROM indicators_10m
        WHERE pool_address = $1
        ORDER BY bucket_start DESC
        LIMIT 1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching latest indicators_10m row for {pool_address}"))?;

    Ok(row)
}

async fn fetch_latest_h1(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<IndicatorRow>> {
    let row = sqlx::query_as!(
        IndicatorRow,
        r#"
        SELECT
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        FROM indicators_1h
        WHERE pool_address = $1
        ORDER BY bucket_start DESC
        LIMIT 1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching latest indicators_1h row for {pool_address}"))?;

    Ok(row)
}

async fn fetch_latest_h4(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<IndicatorRow>> {
    let row = sqlx::query_as!(
        IndicatorRow,
        r#"
        SELECT
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        FROM indicators_4h
        WHERE pool_address = $1
        ORDER BY bucket_start DESC
        LIMIT 1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching latest indicators_4h row for {pool_address}"))?;

    Ok(row)
}

async fn fetch_latest_h24(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<IndicatorRow>> {
    let row = sqlx::query_as!(
        IndicatorRow,
        r#"
        SELECT
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        FROM indicators_24h
        WHERE pool_address = $1
        ORDER BY bucket_start DESC
        LIMIT 1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching latest indicators_24h row for {pool_address}"))?;

    Ok(row)
}

pub async fn pool_detail(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<PoolDetail>> {
    let Some(summary) = fetch_pool_summary(pool, pool_address).await? else {
        return Ok(None);
    };

    let (m5, m10, h1, h4, h24) = tokio::try_join!(
        fetch_latest_m5(pool, pool_address),
        fetch_latest_m10(pool, pool_address),
        fetch_latest_h1(pool, pool_address),
        fetch_latest_h4(pool, pool_address),
        fetch_latest_h24(pool, pool_address),
    )?;

    Ok(Some(PoolDetail {
        pool: summary,
        m5,
        m10,
        h1,
        h4,
        h24,
    }))
}
