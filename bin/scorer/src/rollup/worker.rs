use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Timelike, Utc};
use common::Worker;
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use storage::queries::{
    LiquidityBucketAgg, SnapshotBucketAgg, SwapBucketAgg, active_tvl_median,
    liquidity_bucket_aggregates, pool_snapshot_bucket_aggregates, scoring_universe,
    swap_bucket_aggregates,
};
use storage::types::{tier, venue};
use storage::write::{
    NewPoolMetrics10mBucket, NewPoolMetricsBucket, upsert_pool_metrics_5m, upsert_pool_metrics_10m,
};
use tokio_util::sync::CancellationToken;

use super::build_bucket_from_raw;

/// Builds `pool_metrics_5m` (tier-1 pools only) and `pool_metrics_10m` (tier-1 as a rollup
/// of the same raw window, tier-0 as its own native resolution) every tick. Never touches
/// pool_metrics_{1h,4h,24h} -- those are database continuous aggregates and refresh
/// themselves.
pub struct RollupWorker {
    pool: PgPool,
    interval: Duration,
}

impl RollupWorker {
    pub fn new(pool: PgPool, interval: Duration) -> Self {
        Self { pool, interval }
    }

    async fn tick(&self) -> eyre::Result<()> {
        let now = Utc::now();
        let bucket_end = floor_bucket(now, 5);
        let bucket_start = bucket_end - chrono::Duration::minutes(5);

        let universe = scoring_universe(&self.pool, venue::DLMM)
            .await
            .wrap_err_with(|| "Loading scoring universe for rollup")?;
        let tier1: Vec<String> = universe
            .iter()
            .filter(|p| p.tier == tier::WATCHED)
            .map(|p| p.pool_address.clone())
            .collect();
        let tier0: Vec<String> = universe
            .iter()
            .filter(|p| p.tier != tier::WATCHED)
            .map(|p| p.pool_address.clone())
            .collect();

        let rows_5m = fetch_and_build(&self.pool, &tier1, bucket_start, bucket_end).await?;
        let written = upsert_pool_metrics_5m(&self.pool, &rows_5m).await?;
        tracing::debug!(count = written, bucket = %bucket_start, "Wrote pool_metrics_5m");

        // Tier 0 is observed once per 10-minute universe scan, so 5-minute buckets are half
        // empty by construction and simply never get a row -- absence, not zero.
        if bucket_end.minute().is_multiple_of(10) {
            let bucket10_start = bucket_end - chrono::Duration::minutes(10);
            let tier1_10m = fetch_and_build(&self.pool, &tier1, bucket10_start, bucket_end).await?;
            let tier0_10m = fetch_and_build(&self.pool, &tier0, bucket10_start, bucket_end).await?;

            let wrapped: Vec<NewPoolMetrics10mBucket> = tier1_10m
                .into_iter()
                .map(|bucket| NewPoolMetrics10mBucket {
                    bucket,
                    native_resolution: false,
                })
                .chain(tier0_10m.into_iter().map(|bucket| NewPoolMetrics10mBucket {
                    bucket,
                    native_resolution: true,
                }))
                .collect();
            let written = upsert_pool_metrics_10m(&self.pool, &wrapped).await?;
            tracing::debug!(count = written, bucket = %bucket10_start, "Wrote pool_metrics_10m");
        }

        Ok(())
    }
}

#[async_trait]
impl Worker for RollupWorker {
    fn name(&self) -> &'static str {
        "rollup"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        common::tick_loop(ct, self.interval, || self.tick()).await;
        Ok(())
    }
}

async fn fetch_and_build(
    pool: &PgPool,
    pool_addresses: &[String],
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
) -> eyre::Result<Vec<NewPoolMetricsBucket>> {
    if pool_addresses.is_empty() {
        return Ok(Vec::new());
    }

    let median_since = bucket_end - chrono::Duration::minutes(60);
    let (swaps, snapshots, liquidity, medians) = tokio::try_join!(
        swap_bucket_aggregates(pool, pool_addresses, bucket_start, bucket_end),
        pool_snapshot_bucket_aggregates(pool, pool_addresses, bucket_start, bucket_end),
        liquidity_bucket_aggregates(pool, pool_addresses, bucket_start, bucket_end),
        active_tvl_median(pool, pool_addresses, median_since, bucket_end),
    )?;

    let swap_map: HashMap<&str, &SwapBucketAgg> =
        swaps.iter().map(|s| (s.pool_address.as_str(), s)).collect();
    let snapshot_map: HashMap<&str, &SnapshotBucketAgg> = snapshots
        .iter()
        .map(|s| (s.pool_address.as_str(), s))
        .collect();
    let liquidity_map: HashMap<&str, &LiquidityBucketAgg> = liquidity
        .iter()
        .map(|s| (s.pool_address.as_str(), s))
        .collect();
    let median_map: HashMap<&str, Decimal> = medians
        .iter()
        .filter_map(|m| {
            m.median_quote_value_usd
                .map(|v| (m.pool_address.as_str(), v))
        })
        .collect();

    let rows = pool_addresses
        .iter()
        .filter_map(|addr| {
            build_bucket_from_raw(
                addr,
                bucket_start,
                swap_map.get(addr.as_str()).copied(),
                snapshot_map.get(addr.as_str()).copied(),
                liquidity_map.get(addr.as_str()).copied(),
                median_map.get(addr.as_str()).copied(),
            )
        })
        .collect();

    Ok(rows)
}

fn floor_bucket(now: DateTime<Utc>, minutes: i64) -> DateTime<Utc> {
    let bucket_secs = minutes * 60;
    let floored = now.timestamp().div_euclid(bucket_secs) * bucket_secs;
    DateTime::from_timestamp(floored, 0).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_floor_bucket_rounds_down_to_five_minutes() {
        let now = DateTime::parse_from_rfc3339("2026-09-01T12:07:33Z")
            .unwrap()
            .with_timezone(&Utc);
        let floored = floor_bucket(now, 5);
        assert_eq!(floored.to_rfc3339(), "2026-09-01T12:05:00+00:00");
    }

    #[test]
    fn test_floor_bucket_on_exact_boundary_is_unchanged() {
        let now = DateTime::parse_from_rfc3339("2026-09-01T12:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(floor_bucket(now, 10), now);
    }
}
