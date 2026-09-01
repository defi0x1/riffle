use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::types::Timeframe;

// The rollup-layer inputs the pipeline needs per timeframe: this bucket's own OHLC (fed to
// volatility as `latest_bar`) plus enough trailing history to derive log returns, lag
// autocorrelation and the volume/fee trend windows -- all in the caller, since none of that is
// expressible as a single row. Returned newest-first; callers that need chronological order
// reverse it themselves.
#[derive(Clone, Debug)]
pub struct PoolMetricsHistoryRow {
    pub bucket_start: DateTime<Utc>,
    pub volume_usd: Option<Decimal>,
    pub trade_fee_usd: Option<Decimal>,
    pub swap_count: Option<i32>,
    pub unique_traders: Option<i32>,
    pub price_open: Option<f64>,
    pub price_high: Option<f64>,
    pub price_low: Option<f64>,
    pub price_close: Option<f64>,
    pub tvl_close: Option<Decimal>,
    pub active_tvl_close: Option<Decimal>,
    pub active_tvl_median: Option<Decimal>,
    pub active_bin_close: Option<i32>,
    pub total_fee_bps_close: Option<Decimal>,
}

async fn pool_metrics_recent_5m(
    pool: &PgPool,
    pool_address: &str,
    until: DateTime<Utc>,
    limit: i64,
) -> eyre::Result<Vec<PoolMetricsHistoryRow>> {
    let rows = sqlx::query_as!(
        PoolMetricsHistoryRow,
        r#"
        SELECT bucket_start, volume_usd, trade_fee_usd, swap_count, unique_traders,
               price_open, price_high, price_low, price_close,
               tvl_close, active_tvl_close, active_tvl_median, active_bin_close, total_fee_bps_close
        FROM pool_metrics_5m
        WHERE pool_address = $1 AND bucket_start <= $2
        ORDER BY bucket_start DESC
        LIMIT $3
        "#,
        pool_address,
        until,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying pool_metrics_5m history for {pool_address}"))?;

    Ok(rows)
}

async fn pool_metrics_recent_10m(
    pool: &PgPool,
    pool_address: &str,
    until: DateTime<Utc>,
    limit: i64,
) -> eyre::Result<Vec<PoolMetricsHistoryRow>> {
    let rows = sqlx::query_as!(
        PoolMetricsHistoryRow,
        r#"
        SELECT bucket_start, volume_usd, trade_fee_usd, swap_count, unique_traders,
               price_open, price_high, price_low, price_close,
               tvl_close, active_tvl_close, active_tvl_median, active_bin_close, total_fee_bps_close
        FROM pool_metrics_10m
        WHERE pool_address = $1 AND bucket_start <= $2
        ORDER BY bucket_start DESC
        LIMIT $3
        "#,
        pool_address,
        until,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying pool_metrics_10m history for {pool_address}"))?;

    Ok(rows)
}

async fn pool_metrics_recent_1h(
    pool: &PgPool,
    pool_address: &str,
    until: DateTime<Utc>,
    limit: i64,
) -> eyre::Result<Vec<PoolMetricsHistoryRow>> {
    let rows = sqlx::query_as!(
        PoolMetricsHistoryRow,
        r#"
        SELECT bucket_start AS "bucket_start!", volume_usd, trade_fee_usd,
               swap_count::int4 AS "swap_count", unique_traders::int4 AS "unique_traders",
               price_open, price_high, price_low, price_close,
               tvl_close, active_tvl_close, active_tvl_median, active_bin_close, total_fee_bps_close
        FROM pool_metrics_1h
        WHERE pool_address = $1 AND bucket_start <= $2
        ORDER BY bucket_start DESC
        LIMIT $3
        "#,
        pool_address,
        until,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying pool_metrics_1h history for {pool_address}"))?;

    Ok(rows)
}

async fn pool_metrics_recent_4h(
    pool: &PgPool,
    pool_address: &str,
    until: DateTime<Utc>,
    limit: i64,
) -> eyre::Result<Vec<PoolMetricsHistoryRow>> {
    let rows = sqlx::query_as!(
        PoolMetricsHistoryRow,
        r#"
        SELECT bucket_start AS "bucket_start!", volume_usd, trade_fee_usd,
               swap_count::int4 AS "swap_count", unique_traders::int4 AS "unique_traders",
               price_open, price_high, price_low, price_close,
               tvl_close, active_tvl_close, active_tvl_median, active_bin_close, total_fee_bps_close
        FROM pool_metrics_4h
        WHERE pool_address = $1 AND bucket_start <= $2
        ORDER BY bucket_start DESC
        LIMIT $3
        "#,
        pool_address,
        until,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying pool_metrics_4h history for {pool_address}"))?;

    Ok(rows)
}

async fn pool_metrics_recent_24h(
    pool: &PgPool,
    pool_address: &str,
    until: DateTime<Utc>,
    limit: i64,
) -> eyre::Result<Vec<PoolMetricsHistoryRow>> {
    let rows = sqlx::query_as!(
        PoolMetricsHistoryRow,
        r#"
        SELECT bucket_start AS "bucket_start!", volume_usd, trade_fee_usd,
               swap_count::int4 AS "swap_count", unique_traders::int4 AS "unique_traders",
               price_open, price_high, price_low, price_close,
               tvl_close, active_tvl_close, active_tvl_median, active_bin_close, total_fee_bps_close
        FROM pool_metrics_24h
        WHERE pool_address = $1 AND bucket_start <= $2
        ORDER BY bucket_start DESC
        LIMIT $3
        "#,
        pool_address,
        until,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying pool_metrics_24h history for {pool_address}"))?;

    Ok(rows)
}

pub async fn pool_metrics_recent(
    pool: &PgPool,
    timeframe: Timeframe,
    pool_address: &str,
    until: DateTime<Utc>,
    limit: i64,
) -> eyre::Result<Vec<PoolMetricsHistoryRow>> {
    match timeframe {
        Timeframe::M5 => pool_metrics_recent_5m(pool, pool_address, until, limit).await,
        Timeframe::M10 => pool_metrics_recent_10m(pool, pool_address, until, limit).await,
        Timeframe::H1 => pool_metrics_recent_1h(pool, pool_address, until, limit).await,
        Timeframe::H4 => pool_metrics_recent_4h(pool, pool_address, until, limit).await,
        Timeframe::H24 => pool_metrics_recent_24h(pool, pool_address, until, limit).await,
    }
}
