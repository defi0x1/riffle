use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewPoolMetricsBucket {
    pub pool_address: String,
    pub bucket_start: DateTime<Utc>,
    pub volume_usd: Option<Decimal>,
    pub buy_volume_usd: Option<Decimal>,
    pub sell_volume_usd: Option<Decimal>,
    pub trade_fee_usd: Option<Decimal>,
    pub protocol_fee_usd: Option<Decimal>,
    pub swap_count: Option<i32>,
    pub unique_traders: Option<i32>,
    pub price_open: Option<f64>,
    pub price_high: Option<f64>,
    pub price_low: Option<f64>,
    pub price_close: Option<f64>,
    pub tvl_close: Option<Decimal>,
    pub active_tvl_close: Option<Decimal>,
    // Trailing 60-minute median of active-bin liquidity, not a spot reading.
    pub active_tvl_median: Option<Decimal>,
    pub active_bin_open: Option<i32>,
    pub active_bin_close: Option<i32>,
    pub va_close: Option<i32>,
    pub total_fee_bps_close: Option<Decimal>,
    pub reserve_x_close: Option<Decimal>,
    pub reserve_y_close: Option<Decimal>,
    pub net_deposit_usd: Option<Decimal>,
    pub add_count: Option<i32>,
    pub remove_count: Option<i32>,
    pub lp_count_delta: Option<i32>,
}

// A bucket is rewritten as more of the interval's data arrives before its boundary closes, so
// this upserts rather than inserting once -- unlike the raw event tables, `bucket_start` alone
// does not identify an immutable fact.
pub async fn upsert_pool_metrics_5m(
    pool: &PgPool,
    rows: &[NewPoolMetricsBucket],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket_start).collect();
    let volume_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.volume_usd).collect();
    let buy_volume_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.buy_volume_usd).collect();
    let sell_volume_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.sell_volume_usd).collect();
    let trade_fee_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.trade_fee_usd).collect();
    let protocol_fee_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.protocol_fee_usd).collect();
    let swap_count: Vec<Option<i32>> = rows.iter().map(|r| r.swap_count).collect();
    let unique_traders: Vec<Option<i32>> = rows.iter().map(|r| r.unique_traders).collect();
    let price_open: Vec<Option<f64>> = rows.iter().map(|r| r.price_open).collect();
    let price_high: Vec<Option<f64>> = rows.iter().map(|r| r.price_high).collect();
    let price_low: Vec<Option<f64>> = rows.iter().map(|r| r.price_low).collect();
    let price_close: Vec<Option<f64>> = rows.iter().map(|r| r.price_close).collect();
    let tvl_close: Vec<Option<Decimal>> = rows.iter().map(|r| r.tvl_close).collect();
    let active_tvl_close: Vec<Option<Decimal>> = rows.iter().map(|r| r.active_tvl_close).collect();
    let active_tvl_median: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.active_tvl_median).collect();
    let active_bin_open: Vec<Option<i32>> = rows.iter().map(|r| r.active_bin_open).collect();
    let active_bin_close: Vec<Option<i32>> = rows.iter().map(|r| r.active_bin_close).collect();
    let va_close: Vec<Option<i32>> = rows.iter().map(|r| r.va_close).collect();
    let total_fee_bps_close: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.total_fee_bps_close).collect();
    let reserve_x_close: Vec<Option<Decimal>> = rows.iter().map(|r| r.reserve_x_close).collect();
    let reserve_y_close: Vec<Option<Decimal>> = rows.iter().map(|r| r.reserve_y_close).collect();
    let net_deposit_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.net_deposit_usd).collect();
    let add_count: Vec<Option<i32>> = rows.iter().map(|r| r.add_count).collect();
    let remove_count: Vec<Option<i32>> = rows.iter().map(|r| r.remove_count).collect();
    let lp_count_delta: Vec<Option<i32>> = rows.iter().map(|r| r.lp_count_delta).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO pool_metrics_5m (
            pool_address, bucket_start, volume_usd, buy_volume_usd, sell_volume_usd,
            trade_fee_usd, protocol_fee_usd, swap_count, unique_traders,
            price_open, price_high, price_low, price_close,
            tvl_close, active_tvl_close, active_tvl_median, active_bin_open, active_bin_close,
            va_close, total_fee_bps_close, reserve_x_close, reserve_y_close,
            net_deposit_usd, add_count, remove_count, lp_count_delta
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::numeric[], $4::numeric[], $5::numeric[],
            $6::numeric[], $7::numeric[], $8::int[], $9::int[],
            $10::float8[], $11::float8[], $12::float8[], $13::float8[],
            $14::numeric[], $15::numeric[], $16::numeric[], $17::int[], $18::int[],
            $19::int[], $20::numeric[], $21::numeric[], $22::numeric[],
            $23::numeric[], $24::int[], $25::int[], $26::int[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            volume_usd          = EXCLUDED.volume_usd,
            buy_volume_usd       = EXCLUDED.buy_volume_usd,
            sell_volume_usd      = EXCLUDED.sell_volume_usd,
            trade_fee_usd        = EXCLUDED.trade_fee_usd,
            protocol_fee_usd      = EXCLUDED.protocol_fee_usd,
            swap_count          = EXCLUDED.swap_count,
            unique_traders       = EXCLUDED.unique_traders,
            price_open          = EXCLUDED.price_open,
            price_high          = EXCLUDED.price_high,
            price_low           = EXCLUDED.price_low,
            price_close          = EXCLUDED.price_close,
            tvl_close           = EXCLUDED.tvl_close,
            active_tvl_close      = EXCLUDED.active_tvl_close,
            active_tvl_median      = EXCLUDED.active_tvl_median,
            active_bin_open       = EXCLUDED.active_bin_open,
            active_bin_close      = EXCLUDED.active_bin_close,
            va_close            = EXCLUDED.va_close,
            total_fee_bps_close     = EXCLUDED.total_fee_bps_close,
            reserve_x_close       = EXCLUDED.reserve_x_close,
            reserve_y_close       = EXCLUDED.reserve_y_close,
            net_deposit_usd       = EXCLUDED.net_deposit_usd,
            add_count           = EXCLUDED.add_count,
            remove_count         = EXCLUDED.remove_count,
            lp_count_delta        = EXCLUDED.lp_count_delta
        "#,
        &pool_address as &[&str],
        &bucket_start,
        &volume_usd as &[Option<Decimal>],
        &buy_volume_usd as &[Option<Decimal>],
        &sell_volume_usd as &[Option<Decimal>],
        &trade_fee_usd as &[Option<Decimal>],
        &protocol_fee_usd as &[Option<Decimal>],
        &swap_count as &[Option<i32>],
        &unique_traders as &[Option<i32>],
        &price_open as &[Option<f64>],
        &price_high as &[Option<f64>],
        &price_low as &[Option<f64>],
        &price_close as &[Option<f64>],
        &tvl_close as &[Option<Decimal>],
        &active_tvl_close as &[Option<Decimal>],
        &active_tvl_median as &[Option<Decimal>],
        &active_bin_open as &[Option<i32>],
        &active_bin_close as &[Option<i32>],
        &va_close as &[Option<i32>],
        &total_fee_bps_close as &[Option<Decimal>],
        &reserve_x_close as &[Option<Decimal>],
        &reserve_y_close as &[Option<Decimal>],
        &net_deposit_usd as &[Option<Decimal>],
        &add_count as &[Option<i32>],
        &remove_count as &[Option<i32>],
        &lp_count_delta as &[Option<i32>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} pool_metrics_5m buckets", rows.len()))?;

    Ok(result.rows_affected())
}

#[derive(Clone, Debug)]
pub struct NewPoolMetrics10mBucket {
    pub bucket: NewPoolMetricsBucket,
    // true = written directly from a 10-minute universe scan (tier 0, its native resolution).
    // false = rolled up from four pool_metrics_5m buckets (tier 1). Downstream indicators use
    // this to know whether a pool's finest data point for the window is genuinely 10 minutes
    // wide or an aggregate of finer samples.
    pub native_resolution: bool,
}

pub async fn upsert_pool_metrics_10m(
    pool: &PgPool,
    rows: &[NewPoolMetrics10mBucket],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows
        .iter()
        .map(|r| r.bucket.pool_address.as_str())
        .collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket.bucket_start).collect();
    let native_resolution: Vec<bool> = rows.iter().map(|r| r.native_resolution).collect();
    let volume_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.bucket.volume_usd).collect();
    let buy_volume_usd: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.buy_volume_usd).collect();
    let sell_volume_usd: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.sell_volume_usd).collect();
    let trade_fee_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.bucket.trade_fee_usd).collect();
    let protocol_fee_usd: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.protocol_fee_usd).collect();
    let swap_count: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.swap_count).collect();
    let unique_traders: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.unique_traders).collect();
    let price_open: Vec<Option<f64>> = rows.iter().map(|r| r.bucket.price_open).collect();
    let price_high: Vec<Option<f64>> = rows.iter().map(|r| r.bucket.price_high).collect();
    let price_low: Vec<Option<f64>> = rows.iter().map(|r| r.bucket.price_low).collect();
    let price_close: Vec<Option<f64>> = rows.iter().map(|r| r.bucket.price_close).collect();
    let tvl_close: Vec<Option<Decimal>> = rows.iter().map(|r| r.bucket.tvl_close).collect();
    let active_tvl_close: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.active_tvl_close).collect();
    let active_tvl_median: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.active_tvl_median).collect();
    let active_bin_open: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.active_bin_open).collect();
    let active_bin_close: Vec<Option<i32>> =
        rows.iter().map(|r| r.bucket.active_bin_close).collect();
    let va_close: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.va_close).collect();
    let total_fee_bps_close: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.total_fee_bps_close).collect();
    let reserve_x_close: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.reserve_x_close).collect();
    let reserve_y_close: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.reserve_y_close).collect();
    let net_deposit_usd: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.bucket.net_deposit_usd).collect();
    let add_count: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.add_count).collect();
    let remove_count: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.remove_count).collect();
    let lp_count_delta: Vec<Option<i32>> = rows.iter().map(|r| r.bucket.lp_count_delta).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO pool_metrics_10m (
            pool_address, bucket_start, native_resolution,
            volume_usd, buy_volume_usd, sell_volume_usd,
            trade_fee_usd, protocol_fee_usd, swap_count, unique_traders,
            price_open, price_high, price_low, price_close,
            tvl_close, active_tvl_close, active_tvl_median, active_bin_open, active_bin_close,
            va_close, total_fee_bps_close, reserve_x_close, reserve_y_close,
            net_deposit_usd, add_count, remove_count, lp_count_delta
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bool[],
            $4::numeric[], $5::numeric[], $6::numeric[],
            $7::numeric[], $8::numeric[], $9::int[], $10::int[],
            $11::float8[], $12::float8[], $13::float8[], $14::float8[],
            $15::numeric[], $16::numeric[], $17::numeric[], $18::int[], $19::int[],
            $20::int[], $21::numeric[], $22::numeric[], $23::numeric[],
            $24::numeric[], $25::int[], $26::int[], $27::int[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            native_resolution     = EXCLUDED.native_resolution,
            volume_usd          = EXCLUDED.volume_usd,
            buy_volume_usd       = EXCLUDED.buy_volume_usd,
            sell_volume_usd      = EXCLUDED.sell_volume_usd,
            trade_fee_usd        = EXCLUDED.trade_fee_usd,
            protocol_fee_usd      = EXCLUDED.protocol_fee_usd,
            swap_count          = EXCLUDED.swap_count,
            unique_traders       = EXCLUDED.unique_traders,
            price_open          = EXCLUDED.price_open,
            price_high          = EXCLUDED.price_high,
            price_low           = EXCLUDED.price_low,
            price_close          = EXCLUDED.price_close,
            tvl_close           = EXCLUDED.tvl_close,
            active_tvl_close      = EXCLUDED.active_tvl_close,
            active_tvl_median      = EXCLUDED.active_tvl_median,
            active_bin_open       = EXCLUDED.active_bin_open,
            active_bin_close      = EXCLUDED.active_bin_close,
            va_close            = EXCLUDED.va_close,
            total_fee_bps_close     = EXCLUDED.total_fee_bps_close,
            reserve_x_close       = EXCLUDED.reserve_x_close,
            reserve_y_close       = EXCLUDED.reserve_y_close,
            net_deposit_usd       = EXCLUDED.net_deposit_usd,
            add_count           = EXCLUDED.add_count,
            remove_count         = EXCLUDED.remove_count,
            lp_count_delta        = EXCLUDED.lp_count_delta
        "#,
        &pool_address as &[&str],
        &bucket_start,
        &native_resolution,
        &volume_usd as &[Option<Decimal>],
        &buy_volume_usd as &[Option<Decimal>],
        &sell_volume_usd as &[Option<Decimal>],
        &trade_fee_usd as &[Option<Decimal>],
        &protocol_fee_usd as &[Option<Decimal>],
        &swap_count as &[Option<i32>],
        &unique_traders as &[Option<i32>],
        &price_open as &[Option<f64>],
        &price_high as &[Option<f64>],
        &price_low as &[Option<f64>],
        &price_close as &[Option<f64>],
        &tvl_close as &[Option<Decimal>],
        &active_tvl_close as &[Option<Decimal>],
        &active_tvl_median as &[Option<Decimal>],
        &active_bin_open as &[Option<i32>],
        &active_bin_close as &[Option<i32>],
        &va_close as &[Option<i32>],
        &total_fee_bps_close as &[Option<Decimal>],
        &reserve_x_close as &[Option<Decimal>],
        &reserve_y_close as &[Option<Decimal>],
        &net_deposit_usd as &[Option<Decimal>],
        &add_count as &[Option<i32>],
        &remove_count as &[Option<i32>],
        &lp_count_delta as &[Option<i32>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} pool_metrics_10m buckets", rows.len()))?;

    Ok(result.rows_affected())
}
