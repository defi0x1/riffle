use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

// The raw materials `pool_metrics_5m`/`pool_metrics_10m` are built from -- one aggregate per
// source table, joined by the caller at write time rather than in SQL, matching the same
// three-source join the rollup tables' own migrations describe. A pool absent from a result
// simply had no observation in the window; the caller must not synthesise a row for it.

#[derive(Clone, Debug)]
pub struct SwapBucketAgg {
    pub pool_address: String,
    pub volume_usd: Option<Decimal>,
    pub buy_volume_usd: Option<Decimal>,
    pub sell_volume_usd: Option<Decimal>,
    pub trade_fee_usd: Option<Decimal>,
    pub protocol_fee_usd: Option<Decimal>,
    pub swap_count: Option<i64>,
    pub unique_traders: Option<i64>,
    pub price_open: Option<Decimal>,
    pub price_high: Option<Decimal>,
    pub price_low: Option<Decimal>,
    pub price_close: Option<Decimal>,
}

// swap_for_y = true means selling X for Y (sell-side volume); buy volume is the complement.
pub async fn swap_bucket_aggregates(
    pool: &PgPool,
    pool_addresses: &[String],
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
) -> eyre::Result<Vec<SwapBucketAgg>> {
    let rows = sqlx::query_as!(
        SwapBucketAgg,
        r#"
        SELECT
            pool_address,
            sum(volume_usd) AS volume_usd,
            sum(volume_usd) FILTER (WHERE NOT swap_for_y) AS buy_volume_usd,
            sum(volume_usd) FILTER (WHERE swap_for_y) AS sell_volume_usd,
            sum(trade_fee_usd) AS trade_fee_usd,
            sum(protocol_fee_usd) AS protocol_fee_usd,
            count(*) AS swap_count,
            count(DISTINCT signer) AS unique_traders,
            (array_agg(end_price ORDER BY ts ASC))[1] AS price_open,
            max(end_price) AS price_high,
            min(end_price) AS price_low,
            (array_agg(end_price ORDER BY ts DESC))[1] AS price_close
        FROM swaps
        WHERE pool_address = ANY($1) AND ts >= $2 AND ts < $3
        GROUP BY pool_address
        "#,
        pool_addresses,
        bucket_start,
        bucket_end,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying swap bucket aggregates")?;

    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct SnapshotBucketAgg {
    pub pool_address: String,
    pub tvl_usd: Option<Decimal>,
    pub active_tvl_usd: Option<Decimal>,
    pub active_bin_open: Option<i32>,
    pub active_bin_close: Option<i32>,
    pub total_fee_bps: Option<Decimal>,
    pub va_close: Option<i32>,
}

// A row present here is what marks a pool as having a genuine observation in the window --
// the rollup worker's absent-vs-zero decision is "does this pool appear in this result",
// nothing more elaborate.
pub async fn pool_snapshot_bucket_aggregates(
    pool: &PgPool,
    pool_addresses: &[String],
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
) -> eyre::Result<Vec<SnapshotBucketAgg>> {
    let rows = sqlx::query_as!(
        SnapshotBucketAgg,
        r#"
        SELECT
            s.pool_address,
            (array_agg(s.tvl_usd ORDER BY s.ts DESC))[1] AS tvl_usd,
            (array_agg(s.active_tvl_usd ORDER BY s.ts DESC))[1] AS active_tvl_usd,
            (array_agg(d.active_bin_id ORDER BY s.ts ASC))[1] AS active_bin_open,
            (array_agg(d.active_bin_id ORDER BY s.ts DESC))[1] AS active_bin_close,
            (array_agg(s.total_fee_bps ORDER BY s.ts DESC))[1] AS total_fee_bps,
            (array_agg(d.volatility_accumulator ORDER BY s.ts DESC))[1] AS va_close
        FROM pool_snapshots s
        LEFT JOIN dlmm_pool_state d ON d.pool_address = s.pool_address AND d.ts = s.ts
        WHERE s.pool_address = ANY($1) AND s.ts >= $2 AND s.ts < $3
        GROUP BY s.pool_address
        "#,
        pool_addresses,
        bucket_start,
        bucket_end,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying pool snapshot bucket aggregates")?;

    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct LiquidityBucketAgg {
    pub pool_address: String,
    pub net_deposit_usd: Option<Decimal>,
    pub add_count: Option<i64>,
    pub remove_count: Option<i64>,
}

pub async fn liquidity_bucket_aggregates(
    pool: &PgPool,
    pool_addresses: &[String],
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
) -> eyre::Result<Vec<LiquidityBucketAgg>> {
    let rows = sqlx::query_as!(
        LiquidityBucketAgg,
        r#"
        SELECT
            pool_address,
            sum(CASE WHEN action = 0 THEN amount_usd WHEN action = 1 THEN -amount_usd ELSE 0 END)
                AS net_deposit_usd,
            count(*) FILTER (WHERE action = 0) AS add_count,
            count(*) FILTER (WHERE action = 1) AS remove_count
        FROM liquidity_events
        WHERE pool_address = ANY($1) AND ts >= $2 AND ts < $3
        GROUP BY pool_address
        "#,
        pool_addresses,
        bucket_start,
        bucket_end,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying liquidity bucket aggregates")?;

    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct ActiveTvlMedian {
    pub pool_address: String,
    pub median_quote_value_usd: Option<Decimal>,
}

// L-bar_a: the trailing 60-minute median of active-bin quote value, not a spot reading.
pub async fn active_tvl_median(
    pool: &PgPool,
    pool_addresses: &[String],
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> eyre::Result<Vec<ActiveTvlMedian>> {
    let rows = sqlx::query_as!(
        ActiveTvlMedian,
        r#"
        SELECT pool_address,
               (percentile_cont(0.5) WITHIN GROUP (ORDER BY quote_value_usd))::numeric AS median_quote_value_usd
        FROM active_bin_snapshots
        WHERE pool_address = ANY($1) AND ts >= $2 AND ts < $3
        GROUP BY pool_address
        "#,
        pool_addresses,
        since,
        until,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying active-bin liquidity median")?;

    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct LatestActiveBin {
    pub ts: DateTime<Utc>,
    pub bin_id: i32,
    pub quote_value_usd: Option<Decimal>,
}

// A spot reading, unlike active_tvl_median above -- used to estimate a paper position's share
// of active-bin liquidity at mark time, not to feed L-bar_a into the ranking metric.
pub async fn latest_active_bin_snapshot(
    pool: &PgPool,
    pool_address: &str,
) -> eyre::Result<Option<LatestActiveBin>> {
    let row = sqlx::query_as!(
        LatestActiveBin,
        r#"
        SELECT ts, bin_id, quote_value_usd
        FROM active_bin_snapshots
        WHERE pool_address = $1
        ORDER BY ts DESC
        LIMIT 1
        "#,
        pool_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Querying latest active bin snapshot for {pool_address}"))?;

    Ok(row)
}
