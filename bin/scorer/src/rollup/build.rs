//! Pure bucket-building logic, kept separate from the storage calls that supply its inputs
//! so the absent-vs-zero rule is testable without a database.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use storage::queries::{LiquidityBucketAgg, SnapshotBucketAgg, SwapBucketAgg};
use storage::write::NewPoolMetricsBucket;

fn dec_to_f64(d: Option<Decimal>) -> Option<f64> {
    d.and_then(|v| v.to_f64())
}

// reserve_x_close/reserve_y_close and lp_count_delta are not populated by this pass: the
// aggregates this worker reads do not carry reserves or per-owner LP counts, and both are
// display-only columns nothing downstream in the pipeline reads. Left null rather than
// approximated.

/// Build one `pool_metrics_{5m,10m}` row from the three raw aggregates plus the trailing
/// active-liquidity median, or `None` if the pool has no state observation in this window.
/// Presence of `snapshot` is the whole rule: a pool with swaps but no pool state reading in
/// the bucket has nothing a rollup row can anchor `tvl_close`/`active_bin_close` to, so this
/// returns absence rather than a row with those columns null but the rest populated -- the
/// zero-vs-absent distinction is about the *row*, not individual columns within it.
pub fn build_bucket_from_raw(
    pool_address: &str,
    bucket_start: DateTime<Utc>,
    swap: Option<&SwapBucketAgg>,
    snapshot: Option<&SnapshotBucketAgg>,
    liquidity: Option<&LiquidityBucketAgg>,
    active_tvl_median: Option<Decimal>,
) -> Option<NewPoolMetricsBucket> {
    let snapshot = snapshot?;

    Some(NewPoolMetricsBucket {
        pool_address: pool_address.to_string(),
        bucket_start,
        volume_usd: swap.and_then(|s| s.volume_usd),
        buy_volume_usd: swap.and_then(|s| s.buy_volume_usd),
        sell_volume_usd: swap.and_then(|s| s.sell_volume_usd),
        trade_fee_usd: swap.and_then(|s| s.trade_fee_usd),
        protocol_fee_usd: swap.and_then(|s| s.protocol_fee_usd),
        swap_count: swap.and_then(|s| s.swap_count).map(|n| n as i32),
        unique_traders: swap.and_then(|s| s.unique_traders).map(|n| n as i32),
        price_open: swap.and_then(|s| dec_to_f64(s.price_open)),
        price_high: swap.and_then(|s| dec_to_f64(s.price_high)),
        price_low: swap.and_then(|s| dec_to_f64(s.price_low)),
        price_close: swap.and_then(|s| dec_to_f64(s.price_close)),
        tvl_close: snapshot.tvl_usd,
        active_tvl_close: snapshot.active_tvl_usd,
        active_tvl_median,
        active_bin_open: snapshot.active_bin_open,
        active_bin_close: snapshot.active_bin_close,
        va_close: snapshot.va_close,
        total_fee_bps_close: snapshot.total_fee_bps,
        reserve_x_close: None,
        reserve_y_close: None,
        net_deposit_usd: liquidity.and_then(|l| l.net_deposit_usd),
        add_count: liquidity.and_then(|l| l.add_count).map(|n| n as i32),
        remove_count: liquidity.and_then(|l| l.remove_count).map(|n| n as i32),
        lp_count_delta: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn snapshot() -> SnapshotBucketAgg {
        SnapshotBucketAgg {
            pool_address: "pool1".to_string(),
            tvl_usd: Some(Decimal::new(1_000_000, 0)),
            active_tvl_usd: Some(Decimal::new(50_000, 0)),
            active_bin_open: Some(100),
            active_bin_close: Some(101),
            total_fee_bps: Some(Decimal::new(30, 2)),
            va_close: Some(12_000),
        }
    }

    #[test]
    fn test_absent_when_no_state_observation_this_bucket() {
        // A pool with genuine swap flow but no forced 5-minute state sample -- e.g. a tier-0
        // pool observed only on a 10-minute universe scan -- must not get a fabricated row.
        let swap = SwapBucketAgg {
            pool_address: "pool1".to_string(),
            volume_usd: Some(Decimal::new(1_000, 0)),
            buy_volume_usd: Some(Decimal::new(600, 0)),
            sell_volume_usd: Some(Decimal::new(400, 0)),
            trade_fee_usd: Some(Decimal::new(3, 0)),
            protocol_fee_usd: Some(Decimal::new(1, 0)),
            swap_count: Some(5),
            unique_traders: Some(3),
            price_open: Some(Decimal::new(150, 2)),
            price_high: Some(Decimal::new(151, 2)),
            price_low: Some(Decimal::new(149, 2)),
            price_close: Some(Decimal::new(150, 2)),
        };

        let row = build_bucket_from_raw("pool1", t(), Some(&swap), None, None, None);
        assert!(
            row.is_none(),
            "a bucket with no state observation must be absent, not a zero-filled row"
        );
    }

    #[test]
    fn test_present_with_state_observation_even_without_swaps() {
        // A quiet pool with a forced 5-minute state sample but zero trades this bucket: the
        // row exists (state was genuinely observed), flow columns are simply null, not zero.
        let row = build_bucket_from_raw("pool1", t(), None, Some(&snapshot()), None, None);
        let row = row.expect("state observation present -> row must exist");
        assert_eq!(row.tvl_close, Some(Decimal::new(1_000_000, 0)));
        assert_eq!(row.volume_usd, None, "no swaps -> null, not zero");
        assert_eq!(row.swap_count, None, "no swaps -> null, not zero");
    }

    #[test]
    fn test_full_bucket_carries_every_source_through() {
        let swap = SwapBucketAgg {
            pool_address: "pool1".to_string(),
            volume_usd: Some(Decimal::new(2_000, 0)),
            buy_volume_usd: Some(Decimal::new(1_200, 0)),
            sell_volume_usd: Some(Decimal::new(800, 0)),
            trade_fee_usd: Some(Decimal::new(6, 0)),
            protocol_fee_usd: Some(Decimal::new(2, 0)),
            swap_count: Some(10),
            unique_traders: Some(4),
            price_open: Some(Decimal::new(150, 2)),
            price_high: Some(Decimal::new(152, 2)),
            price_low: Some(Decimal::new(149, 2)),
            price_close: Some(Decimal::new(151, 2)),
        };
        let liquidity = LiquidityBucketAgg {
            pool_address: "pool1".to_string(),
            net_deposit_usd: Some(Decimal::new(5_000, 0)),
            add_count: Some(2),
            remove_count: Some(1),
        };

        let row = build_bucket_from_raw(
            "pool1",
            t(),
            Some(&swap),
            Some(&snapshot()),
            Some(&liquidity),
            Some(Decimal::new(48_000, 0)),
        )
        .unwrap();

        assert_eq!(row.pool_address, "pool1");
        assert_eq!(row.volume_usd, Some(Decimal::new(2_000, 0)));
        assert_eq!(row.swap_count, Some(10));
        assert_eq!(row.unique_traders, Some(4));
        assert_eq!(row.price_close, Some(1.51));
        assert_eq!(row.active_tvl_median, Some(Decimal::new(48_000, 0)));
        assert_eq!(row.net_deposit_usd, Some(Decimal::new(5_000, 0)));
        assert_eq!(row.add_count, Some(2));
        assert_eq!(row.remove_count, Some(1));
    }
}
