//! Property 2: the write path round-trips. Insert through storage's public write
//! functions, read back through its query functions (and, where no typed query exists for
//! a raw table, a direct `SELECT *`), and assert equality -- especially the fixed-point and
//! high-precision decimal columns, which are the easiest to get wrong silently (a `NUMERIC`
//! truncation or an `f64` cast does not raise an error, it just loses digits).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use storage::queries::{
    active_tvl_median, latest_active_bin_snapshot, liquidity_bucket_aggregates,
    pool_snapshot_bucket_aggregates, scoring_universe, swap_bucket_aggregates,
};
use storage::types::{liquidity_action, venue};
use storage::write::{
    NewActiveBinSnapshot, NewBinState, NewDlmmPoolState, NewLiquidityEvent, NewPoolSnapshot,
    NewSwap, NewToken, insert_active_bin_snapshots, insert_bin_states, insert_liquidity_events,
    insert_pool_state, insert_swaps, upsert_token,
};

fn t() -> DateTime<Utc> {
    integration::fixture_time()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

#[derive(sqlx::FromRow, Debug)]
struct RawSwapRow {
    pool_address: String,
    ts: DateTime<Utc>,
    slot: i64,
    signature: String,
    ix_index: i32,
    signer: String,
    swap_for_y: bool,
    amount_in_raw: Decimal,
    amount_out_raw: Decimal,
    amount_in: Decimal,
    amount_out: Decimal,
    start_bin_id: i32,
    end_bin_id: i32,
    start_price: Option<Decimal>,
    end_price: Option<Decimal>,
    fee_raw: Decimal,
    protocol_fee_raw: Decimal,
    host_fee_raw: Option<Decimal>,
    fee_bps: Decimal,
    volume_usd: Option<Decimal>,
    trade_fee_usd: Option<Decimal>,
    protocol_fee_usd: Option<Decimal>,
}

#[tokio::test]
async fn test_pool_metadata_round_trips_through_scoring_universe() {
    let pool = integration::require_database!();
    let pool_address = "roundtrip_pool_metadata";
    integration::reset_pool_fixture(&pool, pool_address).await;

    let now = t();
    integration::ensure_pool_with(&pool, pool_address, |shared, params| {
        shared.base_fee_bps = dec("0.123456");
        shared.protocol_share_bps = 777;
        shared.tvl_usd = Some(dec("1234567.123456789012345678"));
        shared.tags = vec!["meme".to_string(), "verified".to_string()];
        params.bin_step = 25;
        params.base_factor = 12_345;
        params.variable_fee_control = 98_765;
    })
    .await;

    upsert_token(
        &pool,
        &NewToken {
            mint: integration::WRAPPED_SOL.to_string(),
            symbol: Some("SOL".to_string()),
            name: Some("Wrapped SOL".to_string()),
            decimals: 9,
            mint_authority: None,
            freeze_authority: None,
            token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            extensions: None,
            supply: Some(dec("581012345678901234")),
            holder_count: Some(1_200_000),
            top10_share: Some(0.021),
            top1_share: Some(0.004),
            is_verified: Some(true),
            rugcheck_score: None,
            rugcheck_flags: None,
            rugcheck_at: None,
            updated_at: now,
        },
    )
    .await
    .unwrap();

    upsert_token(
        &pool,
        &NewToken {
            mint: integration::USDC.to_string(),
            symbol: Some("USDC".to_string()),
            name: Some("USD Coin".to_string()),
            decimals: 6,
            mint_authority: Some("some_mint_authority_11111111111111111111111".to_string()),
            freeze_authority: Some("some_freeze_authority_1111111111111111111111".to_string()),
            token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            extensions: None,
            supply: None,
            holder_count: None,
            top10_share: Some(0.512340987),
            top1_share: Some(0.198765432),
            is_verified: None,
            rugcheck_score: Some(87),
            rugcheck_flags: None,
            rugcheck_at: None,
            updated_at: now,
        },
    )
    .await
    .unwrap();

    let universe = scoring_universe(&pool, venue::DLMM).await.unwrap();
    let row = universe
        .iter()
        .find(|p| p.pool_address == pool_address)
        .expect("fixture pool present in scoring universe");

    assert_eq!(row.base_fee_bps, dec("0.123456"));
    assert_eq!(row.protocol_share_bps, 777);
    assert_eq!(row.tvl_usd, Some(dec("1234567.123456789012345678")));
    assert_eq!(row.bin_step, 25);
    assert_eq!(row.base_factor, 12_345);
    assert_eq!(row.variable_fee_control, 98_765);
    assert_eq!(row.tier, storage::types::tier::UNIVERSE);
    assert_eq!(row.x_mint_authority, None, "SOL has no mint authority");
    assert_eq!(
        row.y_mint_authority,
        Some("some_mint_authority_11111111111111111111111".to_string())
    );
    assert!(row.y_freeze_authority.is_some());
    assert_eq!(row.x_top10_share, Some(0.021));
    assert_eq!(row.y_top10_share, Some(0.512340987));
    assert_eq!(row.y_top1_share, Some(0.198765432));
}

#[tokio::test]
async fn test_swap_round_trips_exactly_including_high_precision_decimals() {
    let pool = integration::require_database!();
    let pool_address = "roundtrip_swap";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let swap = NewSwap {
        pool_address: pool_address.to_string(),
        ts: t(),
        slot: 123_456_789,
        signature: "sig_roundtrip_swap".to_string(),
        ix_index: 2,
        signer: "signer1111111111111111111111111111111111111".to_string(),
        swap_for_y: true,
        amount_in_raw: dec("123456789012345678901234"),
        amount_out_raw: dec("987654321098765432109876"),
        amount_in: dec("123456.789012345678901234"),
        amount_out: dec("987654.321098765432109876"),
        start_bin_id: 8_388_608,
        end_bin_id: 8_388_610,
        start_price: Some(dec("1.500000000000000001")),
        end_price: Some(dec("1.510000000000000002")),
        fee_raw: dec("1000000000000000000"),
        protocol_fee_raw: dec("100000000000000000"),
        host_fee_raw: Some(dec("5000000000000000")),
        fee_bps: dec("30.5"),
        volume_usd: Some(dec("1500.123456789012345678")),
        trade_fee_usd: Some(dec("9.001234567890123456")),
        protocol_fee_usd: Some(dec("0.900123456789012345")),
    };

    insert_swaps(&pool, std::slice::from_ref(&swap))
        .await
        .unwrap();

    let raw: RawSwapRow = sqlx::query_as(
        "SELECT * FROM swaps WHERE pool_address = $1 AND signature = $2 AND ix_index = $3",
    )
    .bind(pool_address)
    .bind(&swap.signature)
    .bind(swap.ix_index)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(raw.pool_address, swap.pool_address);
    assert_eq!(raw.ts, swap.ts);
    assert_eq!(raw.slot, swap.slot);
    assert_eq!(raw.signature, swap.signature);
    assert_eq!(raw.ix_index, swap.ix_index);
    assert_eq!(raw.signer, swap.signer);
    assert_eq!(raw.swap_for_y, swap.swap_for_y);
    assert_eq!(raw.amount_in_raw, swap.amount_in_raw);
    assert_eq!(raw.amount_out_raw, swap.amount_out_raw);
    assert_eq!(raw.amount_in, swap.amount_in);
    assert_eq!(raw.amount_out, swap.amount_out);
    assert_eq!(raw.start_bin_id, swap.start_bin_id);
    assert_eq!(raw.end_bin_id, swap.end_bin_id);
    assert_eq!(raw.start_price, swap.start_price);
    assert_eq!(raw.end_price, swap.end_price);
    assert_eq!(raw.fee_raw, swap.fee_raw);
    assert_eq!(raw.protocol_fee_raw, swap.protocol_fee_raw);
    assert_eq!(raw.host_fee_raw, swap.host_fee_raw);
    assert_eq!(raw.fee_bps, swap.fee_bps);
    assert_eq!(raw.volume_usd, swap.volume_usd);
    assert_eq!(raw.trade_fee_usd, swap.trade_fee_usd);
    assert_eq!(raw.protocol_fee_usd, swap.protocol_fee_usd);

    // And through the real query path: the bucket aggregate over a window containing
    // exactly this one swap must reproduce its volume/fee/price fields exactly.
    let bucket_start = swap.ts - chrono::Duration::minutes(1);
    let bucket_end = swap.ts + chrono::Duration::minutes(1);
    let agg = swap_bucket_aggregates(&pool, &[pool_address.to_string()], bucket_start, bucket_end)
        .await
        .unwrap();
    let row = agg
        .iter()
        .find(|r| r.pool_address == pool_address)
        .expect("swap present in the bucket aggregate");
    assert_eq!(row.volume_usd, swap.volume_usd);
    assert_eq!(row.trade_fee_usd, swap.trade_fee_usd);
    assert_eq!(row.price_open, swap.end_price);
    assert_eq!(row.price_close, swap.end_price);
    assert_eq!(row.price_high, swap.end_price);
    assert_eq!(row.price_low, swap.end_price);
    assert_eq!(row.swap_count, Some(1));
}

#[tokio::test]
async fn test_liquidity_event_round_trips_exactly() {
    let pool = integration::require_database!();
    let pool_address = "roundtrip_liquidity_event";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let event = NewLiquidityEvent {
        pool_address: pool_address.to_string(),
        ts: t(),
        slot: 42,
        signature: "sig_roundtrip_liquidity".to_string(),
        ix_index: 0,
        position_address: Some("position11111111111111111111111111111111111".to_string()),
        owner: "owner111111111111111111111111111111111111111".to_string(),
        action: liquidity_action::ADD,
        active_bin_id: 8_388_608,
        amount_x_raw: Some(dec("42000000000000000000")),
        amount_y_raw: Some(dec("9000000000")),
        amount_usd: Some(dec("18000.123456789012345678")),
    };

    insert_liquidity_events(&pool, std::slice::from_ref(&event))
        .await
        .unwrap();

    let raw = sqlx::query(
        "SELECT amount_x_raw, amount_y_raw, amount_usd, owner, position_address, active_bin_id \
         FROM liquidity_events WHERE pool_address = $1 AND signature = $2 AND ix_index = $3",
    )
    .bind(pool_address)
    .bind(&event.signature)
    .bind(event.ix_index)
    .fetch_one(&pool)
    .await
    .unwrap();
    use sqlx::Row;
    let amount_x_raw: Option<Decimal> = raw.get("amount_x_raw");
    let amount_y_raw: Option<Decimal> = raw.get("amount_y_raw");
    let amount_usd: Option<Decimal> = raw.get("amount_usd");
    assert_eq!(amount_x_raw, event.amount_x_raw);
    assert_eq!(amount_y_raw, event.amount_y_raw);
    assert_eq!(amount_usd, event.amount_usd);
    let owner: String = raw.get("owner");
    assert_eq!(owner, event.owner);
    let active_bin_id: i32 = raw.get("active_bin_id");
    assert_eq!(active_bin_id, event.active_bin_id);

    let bucket_start = event.ts - chrono::Duration::minutes(1);
    let bucket_end = event.ts + chrono::Duration::minutes(1);
    let agg =
        liquidity_bucket_aggregates(&pool, &[pool_address.to_string()], bucket_start, bucket_end)
            .await
            .unwrap();
    let row = agg.iter().find(|r| r.pool_address == pool_address).unwrap();
    assert_eq!(row.net_deposit_usd, event.amount_usd);
    assert_eq!(row.add_count, Some(1));
    assert_eq!(row.remove_count, Some(0));
}

#[tokio::test]
async fn test_bin_state_round_trips_exactly() {
    let pool = integration::require_database!();
    let pool_address = "roundtrip_bin_state";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let state = NewBinState {
        pool_address: pool_address.to_string(),
        ts: t(),
        slot: 555,
        bin_id: 8_388_609,
        amount_x: dec("123456789012345678"),
        amount_y: dec("987654321098765432"),
        liquidity_supply: dec("111222333444555666"),
        price_q64: dec("18446744073709551617"),
        ui_price: 1.0000123456789,
        fee_x_per_token_stored: dec("1000000000000000000"),
        fee_y_per_token_stored: dec("2000000000000000000"),
    };

    insert_bin_states(&pool, std::slice::from_ref(&state))
        .await
        .unwrap();

    let raw = sqlx::query(
        "SELECT amount_x, amount_y, liquidity_supply, price_q64, ui_price, \
                fee_x_per_token_stored, fee_y_per_token_stored \
         FROM bin_states WHERE pool_address = $1 AND bin_id = $2 AND ts = $3",
    )
    .bind(pool_address)
    .bind(state.bin_id)
    .bind(state.ts)
    .fetch_one(&pool)
    .await
    .unwrap();
    use sqlx::Row;
    let amount_x: Decimal = raw.get("amount_x");
    let amount_y: Decimal = raw.get("amount_y");
    let liquidity_supply: Decimal = raw.get("liquidity_supply");
    let price_q64: Decimal = raw.get("price_q64");
    let ui_price: f64 = raw.get("ui_price");
    let fee_x: Decimal = raw.get("fee_x_per_token_stored");
    let fee_y: Decimal = raw.get("fee_y_per_token_stored");

    assert_eq!(amount_x, state.amount_x);
    assert_eq!(amount_y, state.amount_y);
    assert_eq!(liquidity_supply, state.liquidity_supply);
    assert_eq!(price_q64, state.price_q64);
    assert_eq!(
        ui_price, state.ui_price,
        "f64 must round-trip exactly through DOUBLE PRECISION"
    );
    assert_eq!(fee_x, state.fee_x_per_token_stored);
    assert_eq!(fee_y, state.fee_y_per_token_stored);
}

#[tokio::test]
async fn test_active_bin_snapshot_round_trips_and_feeds_active_tvl_median() {
    let pool = integration::require_database!();
    let pool_address = "roundtrip_active_bin_snapshot";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let snapshot = NewActiveBinSnapshot {
        pool_address: pool_address.to_string(),
        ts: t(),
        slot: 777,
        bin_id: 8_388_608,
        amount_x: dec("50000000000000000000"),
        amount_y: dec("60000000000000000000"),
        liquidity_supply: dec("110000000000000000000"),
        quote_value_usd: Some(dec("42000.123456789012345678")),
    };

    insert_active_bin_snapshots(&pool, std::slice::from_ref(&snapshot))
        .await
        .unwrap();

    let latest = latest_active_bin_snapshot(&pool, pool_address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.ts, snapshot.ts);
    assert_eq!(latest.bin_id, snapshot.bin_id);
    assert_eq!(latest.quote_value_usd, snapshot.quote_value_usd);

    let median = active_tvl_median(
        &pool,
        &[pool_address.to_string()],
        snapshot.ts - chrono::Duration::minutes(1),
        snapshot.ts + chrono::Duration::minutes(1),
    )
    .await
    .unwrap();
    let row = median
        .iter()
        .find(|r| r.pool_address == pool_address)
        .unwrap();
    // NOT an exact match, and that is itself the finding: Postgres has no NUMERIC-typed
    // `percentile_cont` overload (only `percentile_cont(double precision) WITHIN GROUP
    // (ORDER BY double precision)`), so `active_tvl_median`'s `percentile_cont(0.5) WITHIN
    // GROUP (ORDER BY quote_value_usd)` -- `quote_value_usd` is NUMERIC(38,18) -- silently
    // casts through `double precision` before the `::numeric` cast the query applies to its
    // result. The median of a single observation is exact math (no interpolation needed)
    // yet still comes back float64-truncated: this is a real precision defect in
    // `storage::queries::rollup_source::active_tvl_median`, out of scope for this suite to
    // fix (`libraries/storage` is not owned by this task) and reported instead of silently
    // asserted around -- see the suite's summary report.
    let expected_f64_precision = Decimal::from_str(&snapshot.quote_value_usd.unwrap().to_string())
        .unwrap()
        .to_string()
        .parse::<f64>()
        .unwrap();
    let observed: f64 = row
        .median_quote_value_usd
        .and_then(|d| d.to_string().parse::<f64>().ok())
        .expect("median must be present for a single observation");
    assert!(
        (observed - expected_f64_precision).abs() < 1e-6,
        "median {observed} should match the single observation {expected_f64_precision} to \
         float64 precision, even though `active_tvl_median` cannot preserve full NUMERIC \
         precision"
    );
}

#[tokio::test]
async fn test_pool_state_round_trips_through_rollup_source() {
    let pool = integration::require_database!();
    let pool_address = "roundtrip_pool_state";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let ts = t();
    let snapshot = NewPoolSnapshot {
        pool_address: pool_address.to_string(),
        ts,
        slot: 999,
        price: 1.2345678901234,
        reserve_x_raw: Some(dec("1000000000000000000000")),
        reserve_y_raw: Some(dec("2000000000000000000000")),
        tvl_usd: Some(dec("3456789.123456789012345678")),
        active_tvl_usd: Some(dec("123456.123456789012345678")),
        total_fee_bps: dec("30.5"),
    };
    let dlmm_state = NewDlmmPoolState {
        pool_address: pool_address.to_string(),
        ts,
        active_bin_id: 8_388_612,
        volatility_accumulator: 123_456,
        volatility_reference: 12_345,
        index_reference: 4,
        last_update_timestamp: 1_735_689_600,
        base_fee_bps: dec("20"),
        dynamic_fee_bps: dec("10.5"),
    };

    insert_pool_state(
        &pool,
        std::slice::from_ref(&snapshot),
        std::slice::from_ref(&dlmm_state),
    )
    .await
    .unwrap();

    let agg = pool_snapshot_bucket_aggregates(
        &pool,
        &[pool_address.to_string()],
        ts - chrono::Duration::minutes(1),
        ts + chrono::Duration::minutes(1),
    )
    .await
    .unwrap();
    let row = agg.iter().find(|r| r.pool_address == pool_address).unwrap();
    assert_eq!(row.tvl_usd, snapshot.tvl_usd);
    assert_eq!(row.active_tvl_usd, snapshot.active_tvl_usd);
    assert_eq!(row.total_fee_bps, Some(snapshot.total_fee_bps));
    assert_eq!(row.active_bin_open, Some(dlmm_state.active_bin_id));
    assert_eq!(row.active_bin_close, Some(dlmm_state.active_bin_id));
    assert_eq!(row.va_close, Some(dlmm_state.volatility_accumulator));
}
