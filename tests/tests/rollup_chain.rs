// Proves the rollup chain aggregates correctly, end to end through Postgres: raw swaps,
// liquidity events and pool state at 5-minute cadence, aggregated into `pool_metrics_5m`
// through the exact function production uses (`scorer::rollup::build_bucket_from_raw`, fed
// by the exact storage queries `scorer`'s rollup worker calls), re-aggregated at 10-minute
// width, then chained up through the real TimescaleDB continuous aggregates
// `pool_metrics_1h` -> `pool_metrics_4h` -> `pool_metrics_24h`.
//
// Includes the case that is easy to get wrong: a pool with swaps/snapshots/liquidity events
// nowhere in the bucket window must be ABSENT from every tier's output, not present with
// zeroes -- zero and unknown mean different things downstream (a real zero-volume 5-minute
// window looks identical to a pool nobody has looked at yet if absence isn't preserved).

use chrono::{DateTime, Utc};
use integration::require_database;
use rust_decimal::Decimal;
use scorer::rollup::build_bucket_from_raw;
use sqlx::Row;
use storage::types::Timeframe;
use storage::write::{
    NewActiveBinSnapshot, NewDlmmPoolState, NewLiquidityEvent, NewPoolMetrics10mBucket,
    NewPoolSnapshot, NewSwap, insert_active_bin_snapshots, insert_liquidity_events,
    insert_pool_state, insert_swaps, upsert_pool_metrics_5m, upsert_pool_metrics_10m,
};

// A fixture helper with one parameter per varying field of the worked example below, kept
// as plain arguments rather than a builder struct so each call site reads as a flat table
// of the scenario's numbers.
#[allow(clippy::too_many_arguments)]
fn swap(
    pool_address: &str,
    ts: DateTime<Utc>,
    signature: &str,
    signer: &str,
    swap_for_y: bool,
    end_price: &str,
    volume_usd: &str,
    trade_fee_usd: &str,
    protocol_fee_usd: &str,
) -> NewSwap {
    NewSwap {
        pool_address: pool_address.to_string(),
        ts,
        slot: 1,
        signature: signature.to_string(),
        ix_index: 0,
        signer: signer.to_string(),
        swap_for_y,
        amount_in_raw: Decimal::new(1_000_000, 0),
        amount_out_raw: Decimal::new(1_000_000, 0),
        amount_in: Decimal::new(1, 0),
        amount_out: Decimal::new(1, 0),
        start_bin_id: 100,
        end_bin_id: 100,
        start_price: Decimal::from_str_exact(end_price).ok(),
        end_price: Decimal::from_str_exact(end_price).ok(),
        fee_raw: Decimal::new(1, 0),
        protocol_fee_raw: Decimal::new(1, 0),
        host_fee_raw: None,
        fee_bps: Decimal::new(30, 2),
        volume_usd: Decimal::from_str_exact(volume_usd).ok(),
        trade_fee_usd: Decimal::from_str_exact(trade_fee_usd).ok(),
        protocol_fee_usd: Decimal::from_str_exact(protocol_fee_usd).ok(),
    }
}

async fn build_and_write_bucket(
    pool: &sqlx::PgPool,
    pool_addresses: &[String],
    pool_address: &str,
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
) -> Option<storage::write::NewPoolMetricsBucket> {
    let swaps =
        storage::queries::swap_bucket_aggregates(pool, pool_addresses, bucket_start, bucket_end)
            .await
            .expect("querying swap bucket aggregates");
    let snapshots = storage::queries::pool_snapshot_bucket_aggregates(
        pool,
        pool_addresses,
        bucket_start,
        bucket_end,
    )
    .await
    .expect("querying pool snapshot bucket aggregates");
    let liquidity = storage::queries::liquidity_bucket_aggregates(
        pool,
        pool_addresses,
        bucket_start,
        bucket_end,
    )
    .await
    .expect("querying liquidity bucket aggregates");
    let median = storage::queries::active_tvl_median(
        pool,
        pool_addresses,
        bucket_end - chrono::Duration::minutes(60),
        bucket_end,
    )
    .await
    .expect("querying active tvl median");

    build_bucket_from_raw(
        pool_address,
        bucket_start,
        swaps.iter().find(|r| r.pool_address == pool_address),
        snapshots.iter().find(|r| r.pool_address == pool_address),
        liquidity.iter().find(|r| r.pool_address == pool_address),
        median
            .iter()
            .find(|r| r.pool_address == pool_address)
            .and_then(|r| r.median_quote_value_usd),
    )
}

#[tokio::test]
async fn test_rollup_chain_aggregates_sums_opens_closes_highs_lows_and_omits_idle_pools() {
    let pool = require_database!();
    let active = "pool_rollup_active";
    let idle = "pool_rollup_idle";
    integration::ensure_pool(&pool, active).await;
    integration::ensure_pool(&pool, idle).await;
    integration::reset_pool_fixture(&pool, active).await;
    integration::reset_pool_fixture(&pool, idle).await;

    let hour_start: DateTime<Utc> = "2024-04-01T00:00:00Z".parse().unwrap();
    let bucket0_start = hour_start;
    let bucket0_end = bucket0_start + chrono::Duration::minutes(5);
    let bucket1_start = bucket0_end;
    let bucket1_end = bucket1_start + chrono::Duration::minutes(10 - 5);
    let ten_min_end = bucket1_end;

    // Bucket 0 (00:00-00:05): two swaps, one buy and one sell.
    insert_swaps(
        &pool,
        &[
            swap(
                active,
                bucket0_start + chrono::Duration::minutes(1),
                "sig_rollup_a",
                "signerA",
                false, // buy
                "1.50",
                "1000",
                "3",
                "0.5",
            ),
            swap(
                active,
                bucket0_start + chrono::Duration::minutes(3),
                "sig_rollup_b",
                "signerB",
                true, // sell
                "1.55",
                "500",
                "1.5",
                "0.25",
            ),
        ],
    )
    .await
    .unwrap();

    // Bucket 1 (00:05-00:10): one more swap, same signer as bucket 0's first swap.
    insert_swaps(
        &pool,
        &[swap(
            active,
            bucket1_start + chrono::Duration::minutes(2),
            "sig_rollup_c",
            "signerA",
            true,
            "1.60",
            "300",
            "0.9",
            "0.15",
        )],
    )
    .await
    .unwrap();

    // One pool_snapshot/dlmm_pool_state observation per 5-minute bucket -- this is what
    // marks a bucket as "observed" at all; without one, build_bucket_from_raw must return
    // None regardless of swap/liquidity activity.
    insert_pool_state(
        &pool,
        &[NewPoolSnapshot {
            pool_address: active.to_string(),
            ts: bucket0_start + chrono::Duration::minutes(4) + chrono::Duration::seconds(30),
            slot: 10,
            price: 1.5,
            reserve_x_raw: None,
            reserve_y_raw: None,
            tvl_usd: Some(Decimal::new(1_000_000, 0)),
            active_tvl_usd: Some(Decimal::new(50_000, 0)),
            total_fee_bps: Decimal::new(30, 2),
        }],
        &[NewDlmmPoolState {
            pool_address: active.to_string(),
            ts: bucket0_start + chrono::Duration::minutes(4) + chrono::Duration::seconds(30),
            active_bin_id: 100,
            volatility_accumulator: 1_000,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 1_700_000_000,
            base_fee_bps: Decimal::new(30, 2),
            dynamic_fee_bps: Decimal::new(0, 0),
        }],
    )
    .await
    .unwrap();
    insert_pool_state(
        &pool,
        &[NewPoolSnapshot {
            pool_address: active.to_string(),
            ts: bucket1_start + chrono::Duration::minutes(4) + chrono::Duration::seconds(30),
            slot: 11,
            price: 1.6,
            reserve_x_raw: None,
            reserve_y_raw: None,
            tvl_usd: Some(Decimal::new(1_010_000, 0)),
            active_tvl_usd: Some(Decimal::new(51_000, 0)),
            total_fee_bps: Decimal::new(30, 2),
        }],
        &[NewDlmmPoolState {
            pool_address: active.to_string(),
            ts: bucket1_start + chrono::Duration::minutes(4) + chrono::Duration::seconds(30),
            active_bin_id: 105,
            volatility_accumulator: 1_200,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 1_700_000_300,
            base_fee_bps: Decimal::new(30, 2),
            dynamic_fee_bps: Decimal::new(0, 0),
        }],
    )
    .await
    .unwrap();

    // One add in bucket 0, one remove in bucket 1.
    insert_liquidity_events(
        &pool,
        &[NewLiquidityEvent {
            pool_address: active.to_string(),
            ts: bucket0_start + chrono::Duration::minutes(2),
            slot: 20,
            signature: "sig_rollup_liq_add".to_string(),
            ix_index: 0,
            position_address: None,
            owner: "owner_rollup_1111111111111111111111111111".to_string(),
            action: storage::types::liquidity_action::ADD,
            active_bin_id: 100,
            amount_x_raw: None,
            amount_y_raw: None,
            amount_usd: Some(Decimal::new(2_000, 0)),
        }],
    )
    .await
    .unwrap();
    insert_liquidity_events(
        &pool,
        &[NewLiquidityEvent {
            pool_address: active.to_string(),
            ts: bucket1_start + chrono::Duration::minutes(2),
            slot: 21,
            signature: "sig_rollup_liq_remove".to_string(),
            ix_index: 0,
            position_address: None,
            owner: "owner_rollup_1111111111111111111111111111".to_string(),
            action: storage::types::liquidity_action::REMOVE,
            active_bin_id: 105,
            amount_x_raw: None,
            amount_y_raw: None,
            amount_usd: Some(Decimal::new(500, 0)),
        }],
    )
    .await
    .unwrap();

    // A single active-bin observation: the trailing-60-minute median window of every bucket
    // in this test overlaps it, so every bucket's active_tvl_median comes out the same --
    // that is the intended (slow-moving) behaviour of a trailing statistic, not a bug in the
    // test.
    insert_active_bin_snapshots(
        &pool,
        &[NewActiveBinSnapshot {
            pool_address: active.to_string(),
            ts: bucket0_start + chrono::Duration::minutes(2),
            slot: 30,
            bin_id: 100,
            amount_x: Decimal::new(1, 0),
            amount_y: Decimal::new(1, 0),
            liquidity_supply: Decimal::new(1, 0),
            quote_value_usd: Some(Decimal::new(45_000, 0)),
        }],
    )
    .await
    .unwrap();

    let pool_addresses = vec![active.to_string(), idle.to_string()];

    // --- 5-minute tier ---
    let bucket0 =
        build_and_write_bucket(&pool, &pool_addresses, active, bucket0_start, bucket0_end)
            .await
            .expect("bucket 0 has an observation and must not be absent");
    assert_eq!(bucket0.volume_usd, Some(Decimal::new(1_500, 0)));
    assert_eq!(
        bucket0.trade_fee_usd,
        Some(Decimal::from_str_exact("4.5").unwrap())
    );
    assert_eq!(bucket0.swap_count, Some(2));
    assert_eq!(bucket0.unique_traders, Some(2));
    assert_eq!(bucket0.price_open, Some(1.50));
    assert_eq!(bucket0.price_close, Some(1.55));
    assert_eq!(bucket0.price_high, Some(1.55));
    assert_eq!(bucket0.price_low, Some(1.50));
    assert_eq!(bucket0.tvl_close, Some(Decimal::new(1_000_000, 0)));
    assert_eq!(bucket0.active_bin_open, Some(100));
    assert_eq!(bucket0.active_bin_close, Some(100));
    assert_eq!(bucket0.net_deposit_usd, Some(Decimal::new(2_000, 0)));
    assert_eq!(bucket0.add_count, Some(1));
    assert_eq!(bucket0.remove_count, Some(0));
    assert_eq!(bucket0.active_tvl_median, Some(Decimal::new(45_000, 0)));
    upsert_pool_metrics_5m(&pool, &[bucket0]).await.unwrap();

    let bucket1 =
        build_and_write_bucket(&pool, &pool_addresses, active, bucket1_start, bucket1_end)
            .await
            .expect("bucket 1 has an observation and must not be absent");
    assert_eq!(bucket1.volume_usd, Some(Decimal::new(300, 0)));
    assert_eq!(bucket1.swap_count, Some(1));
    assert_eq!(bucket1.unique_traders, Some(1));
    assert_eq!(bucket1.price_open, Some(1.60));
    assert_eq!(bucket1.price_close, Some(1.60));
    assert_eq!(bucket1.active_bin_close, Some(105));
    assert_eq!(bucket1.net_deposit_usd, Some(Decimal::new(-500, 0)));
    assert_eq!(bucket1.remove_count, Some(1));
    upsert_pool_metrics_5m(&pool, &[bucket1]).await.unwrap();

    // The idle pool has no swaps, no snapshot, no liquidity events anywhere -- it must be
    // absent from the aggregate query results and build_bucket_from_raw must refuse to
    // synthesise a row for it.
    let idle_bucket =
        build_and_write_bucket(&pool, &pool_addresses, idle, bucket0_start, bucket0_end).await;
    assert!(
        idle_bucket.is_none(),
        "a pool with no observation in the bucket must be absent, not a zeroed row"
    );

    let idle_row_count: i64 =
        sqlx::query("SELECT count(*) AS c FROM pool_metrics_5m WHERE pool_address = $1")
            .bind(idle)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("c");
    assert_eq!(idle_row_count, 0);

    // --- 10-minute tier: re-aggregates the raw tables over the wider window, not a sum of
    // the two 5-minute rows already written -- see scorer::rollup::worker for why.
    let ten_min =
        build_and_write_bucket(&pool, &pool_addresses, active, bucket0_start, ten_min_end)
            .await
            .expect("the 10-minute window has observations and must not be absent");
    assert_eq!(ten_min.volume_usd, Some(Decimal::new(1_800, 0)));
    assert_eq!(ten_min.swap_count, Some(3));
    assert_eq!(ten_min.unique_traders, Some(2));
    assert_eq!(ten_min.price_open, Some(1.50));
    assert_eq!(ten_min.price_close, Some(1.60));
    assert_eq!(ten_min.price_high, Some(1.60));
    assert_eq!(ten_min.price_low, Some(1.50));
    assert_eq!(ten_min.tvl_close, Some(Decimal::new(1_010_000, 0)));
    assert_eq!(ten_min.active_bin_open, Some(100));
    assert_eq!(ten_min.active_bin_close, Some(105));
    assert_eq!(ten_min.net_deposit_usd, Some(Decimal::new(1_500, 0)));
    assert_eq!(ten_min.add_count, Some(1));
    assert_eq!(ten_min.remove_count, Some(1));
    upsert_pool_metrics_10m(
        &pool,
        &[NewPoolMetrics10mBucket {
            bucket: ten_min,
            native_resolution: true,
        }],
    )
    .await
    .unwrap();

    let idle_10m =
        build_and_write_bucket(&pool, &pool_addresses, idle, bucket0_start, ten_min_end).await;
    assert!(idle_10m.is_none());

    // --- Continuous-aggregate tiers: 1h <- 10m, 4h <- 1h, 24h <- 4h. All three are
    // materialized_only, so nothing appears until refreshed -- production refreshes on a
    // wall-clock schedule; a test refreshes explicitly for a deterministic result.
    integration::refresh_continuous_aggregate(&pool, "pool_metrics_1h").await;
    integration::refresh_continuous_aggregate(&pool, "pool_metrics_4h").await;
    integration::refresh_continuous_aggregate(&pool, "pool_metrics_24h").await;

    for (timeframe, view) in [
        (Timeframe::H1, "pool_metrics_1h"),
        (Timeframe::H4, "pool_metrics_4h"),
        (Timeframe::H24, "pool_metrics_24h"),
    ] {
        let rows = storage::queries::pool_metrics_recent(&pool, timeframe, active, ten_min_end, 10)
            .await
            .unwrap_or_else(|e| panic!("querying {view} history: {e}"));
        let row = rows
            .iter()
            .find(|r| r.bucket_start == hour_start)
            .unwrap_or_else(|| panic!("{view} should hold a bucket starting at {hour_start}"));

        assert_eq!(
            row.volume_usd,
            Some(Decimal::new(1_800, 0)),
            "{view} volume_usd"
        );
        assert_eq!(row.swap_count, Some(3), "{view} swap_count");
        assert_eq!(row.price_open, Some(1.50), "{view} price_open");
        assert_eq!(row.price_close, Some(1.60), "{view} price_close");
        assert_eq!(row.price_high, Some(1.60), "{view} price_high");
        assert_eq!(row.price_low, Some(1.50), "{view} price_low");
        assert_eq!(
            row.tvl_close,
            Some(Decimal::new(1_010_000, 0)),
            "{view} tvl_close"
        );
        assert_eq!(row.active_bin_close, Some(105), "{view} active_bin_close");

        let idle_rows =
            storage::queries::pool_metrics_recent(&pool, timeframe, idle, ten_min_end, 10)
                .await
                .unwrap_or_else(|e| panic!("querying {view} history for the idle pool: {e}"));
        assert!(
            idle_rows.is_empty(),
            "{view} must have no row for a pool with no upstream activity, not a zeroed row"
        );
    }
}
