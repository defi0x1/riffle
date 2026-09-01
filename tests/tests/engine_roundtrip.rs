// Proves the ranking engine reproduces its decisions end to end through Postgres, not just
// in memory: pool/token/rollup/regime/volatility state is written through storage's public
// write functions, read back through the real production queries and the same
// `bin/scorer::pipeline`/`bin/scorer::state` glue `IndicatorsWorker` uses (not a
// reimplementation of that glue), and fed into the exact `engine::rank` entry point
// `libraries/engine`'s own worked examples call.
//
// Two pools, identical in every respect except one token field: `test_healthy_pool_is_ranked`
// clears the risk gate and reaches the ranking stage (`r_org` is populated);
// `test_pool_with_a_live_mint_authority_is_rejected_by_the_risk_gate` sets that same token's
// mint authority and is rejected before ranking ever runs (`r_org` stays `None`) -- the
// database-backed counterpart of `libraries::engine::risk_gate`'s in-memory
// `test_mint_authority_present_fails_gate`, carried all the way through the real pipeline
// entry point instead of stopping at the risk gate in isolation.

use chrono::{DateTime, Utc};
use clap::Parser;
use dlmm_math::{Dlmm, VenueId};
use engine::regime::RegimeState;
use engine::volatility::VolatilityState;
use engine::{EngineConfig, PipelineInput, Regime, rank};
use integration::require_database;
use rust_decimal::Decimal;
use scorer::config::PipelineDefaultsConfig;
use scorer::indicators::to_indicator_row;
use scorer::pipeline;
use scorer::state::{regime_state_from_row, volatility_state_from_row};
use storage::queries::{
    load_regime_state, load_volatility_state, pool_metrics_recent, scoring_universe,
};
use storage::types::{Timeframe, venue};
use storage::write::{
    NewPoolMetricsBucket, NewRegimeStateRow, NewToken, NewVolatilityStateRow, upsert_indicators,
    upsert_pool_metrics_5m, upsert_regime_state, upsert_token, upsert_volatility_state,
};

fn meme_token(mint: &str, mint_authority: Option<&str>, updated_at: DateTime<Utc>) -> NewToken {
    NewToken {
        mint: mint.to_string(),
        symbol: Some("MEME".to_string()),
        name: Some("Integration Test Meme".to_string()),
        decimals: 6,
        mint_authority: mint_authority.map(|a| a.to_string()),
        freeze_authority: None,
        token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
        extensions: None,
        supply: Some(Decimal::new(1_000_000_000, 0)),
        holder_count: Some(500),
        top10_share: Some(0.10),
        top1_share: Some(0.05),
        is_verified: Some(true),
        rugcheck_score: None,
        rugcheck_flags: None,
        rugcheck_at: None,
        updated_at,
    }
}

/// Writes pool metadata, a token, one `pool_metrics_5m` bucket and committed regime/
/// volatility state through storage's public write functions, then reads all of it back
/// through the real production queries and `bin/scorer` glue and assembles the
/// `PipelineInput` `IndicatorsWorker::evaluate_pool` would have built from the same rows.
/// Returns the input plus the mutable state `rank` needs, so the caller runs the actual
/// pipeline entry point itself rather than this helper hiding it.
async fn build_input_from_database(
    pool: &sqlx::PgPool,
    pool_address: &str,
    meme_mint: &str,
) -> (PipelineInput, VolatilityState, RegimeState, EngineConfig) {
    let now = integration::fixture_time() + chrono::Duration::days(200);

    // Already committed, well past both the 30-minute persistence window and the 2-hour
    // cooldown -- the same "settled, long-running instance" convention
    // `libraries::engine::pipeline`'s own worked examples use, so this test is about the
    // stages downstream of regime classification, not regime classification itself.
    upsert_regime_state(
        pool,
        &NewRegimeStateRow {
            pool_address: pool_address.to_string(),
            venue: venue::DLMM,
            timeframe: Timeframe::M5.as_str().to_string(),
            regime: Some(Regime::V1.to_string()),
            since: now - chrono::Duration::days(10),
            pending: None,
            pending_since: None,
            last_transition: Some(now - chrono::Duration::days(10)),
            updated_at: now,
        },
    )
    .await
    .expect("seeding regime state");

    upsert_volatility_state(
        pool,
        &NewVolatilityStateRow {
            pool_address: pool_address.to_string(),
            venue: venue::DLMM,
            timeframe: Timeframe::M5.as_str().to_string(),
            sigma_fast_variance: 0.0,
            sigma_slow_variance: 0.0,
            first_observed_at: now - chrono::Duration::days(10),
            updated_at: now,
        },
    )
    .await
    .expect("seeding volatility state");

    // One 5-minute bucket is enough: `sufficient_history` above is time-based
    // (`first_observed_at`), not row-count based, and `pipeline::assemble` only needs a
    // current row to produce a real OHLC bar and TVL reading.
    upsert_pool_metrics_5m(
        pool,
        &[NewPoolMetricsBucket {
            pool_address: pool_address.to_string(),
            bucket_start: now,
            volume_usd: Some(Decimal::new(50_000, 0)),
            buy_volume_usd: Some(Decimal::new(30_000, 0)),
            sell_volume_usd: Some(Decimal::new(20_000, 0)),
            trade_fee_usd: Some(Decimal::new(150, 0)),
            protocol_fee_usd: Some(Decimal::new(15, 0)),
            swap_count: Some(40),
            unique_traders: Some(12),
            price_open: Some(100.0),
            price_high: Some(100.5),
            price_low: Some(99.6),
            price_close: Some(100.1),
            tvl_close: Some(Decimal::new(5_000_000, 0)),
            active_tvl_close: Some(Decimal::new(300_000, 0)),
            active_tvl_median: Some(Decimal::new(300_000, 0)),
            active_bin_open: Some(8_388_608),
            active_bin_close: Some(8_388_610),
            va_close: Some(12_000),
            total_fee_bps_close: Some(Decimal::new(30, 2)),
            reserve_x_close: None,
            reserve_y_close: None,
            net_deposit_usd: Some(Decimal::new(0, 0)),
            add_count: Some(0),
            remove_count: Some(0),
            lp_count_delta: Some(0),
        }],
    )
    .await
    .expect("seeding pool_metrics_5m");

    let _ = meme_mint; // token_y already points at meme_mint; kept for call-site clarity.

    // Everything from here on is read back through the real production queries and the same
    // `bin/scorer` conversion functions `IndicatorsWorker::evaluate_pool` calls -- not a
    // reimplementation of any of them.
    let universe = scoring_universe(pool, venue::DLMM)
        .await
        .expect("querying scoring universe");
    let pool_meta = universe
        .iter()
        .find(|p| p.pool_address == pool_address)
        .expect("fixture pool must appear in the scoring universe");

    let history = pool_metrics_recent(pool, Timeframe::M5, pool_address, now, 300)
        .await
        .expect("querying pool_metrics_5m history");
    let assembled = pipeline::assemble(&history, Timeframe::M5).expect("history must assemble");

    let regime_row = load_regime_state(pool, pool_address, venue::DLMM, Timeframe::M5.as_str())
        .await
        .expect("loading regime state")
        .expect("regime state was just seeded");
    let loaded_regime_state = regime_state_from_row(&regime_row);
    let previous_regime = loaded_regime_state.regime;

    let vol_row = load_volatility_state(pool, pool_address, venue::DLMM, Timeframe::M5.as_str())
        .await
        .expect("loading volatility state")
        .expect("volatility state was just seeded");
    let loaded_volatility_state = volatility_state_from_row(&vol_row);

    let defaults = PipelineDefaultsConfig::parse_from(["integration"]);
    let engine_cfg = EngineConfig::parse_from(["integration"]);
    let age_days = pipeline::age_days(pool_meta, now);

    let input = PipelineInput {
        pool_address: pool_meta.pool_address.clone(),
        venue: VenueId::Dlmm,
        bucket_start: history[0].bucket_start,
        now,
        latest_bar: assembled.latest_bar,
        autocorrelations: assembled.autocorrelations,
        log_returns_24h: assembled.log_returns_24h,
        decay_window_secs: defaults.decay_window_secs,
        dev_peg: None,
        is_pegged_whitelisted: false,
        is_major: false,
        age_days,
        kill_switch: false,
        risk: pipeline::risk_gate_inputs(pool_meta, now),
        bin_step_bps: pool_meta.bin_step as u16,
        base_factor: pool_meta.base_factor.clamp(0, i32::from(u16::MAX)) as u16,
        base_fee_power_factor: 0,
        variable_fee_control: pool_meta.variable_fee_control.max(0) as u32,
        protocol_share: pool_meta.protocol_share_bps as f64 / 10_000.0,
        tvl_usd: assembled.tvl_usd,
        measured_active_bin_liquidity: Some(300_000.0),
        kappa_c: defaults.kappa_c,
        trade_sizes: Vec::new(),
        phi_time: None,
        n_trades: assembled.n_trades,
        c_fill: pipeline::c_fill_for(previous_regime),
        vol_24h: assembled.vol_24h,
        organic_class_prior_mu: defaults.organic_class_prior_mu,
        organic_class_prior_tau_sq: defaults.organic_class_prior_tau_sq,
        volume_trend: assembled.volume_trend,
        v2_is_young: age_days < 7.0,
        fee_tvl_1h: assembled.fee_tvl_1h,
        fee_tvl_24h: assembled.fee_tvl_24h,
        fee_tvl_7d: assembled.fee_tvl_7d,
        regime_capital: defaults.regime_capital,
        mu_fee: defaults.mu_fee,
        mu_arb: defaults.mu_arb,
        free_capital: defaults.free_capital,
        trigger_history: Vec::new(),
        fee_jack_multiplier: None,
        is_weekend_utc: false,
        previous: assembled.previous,
    };

    (
        input,
        loaded_volatility_state,
        loaded_regime_state,
        engine_cfg,
    )
}

#[tokio::test]
async fn test_healthy_pool_clears_the_risk_gate_and_is_ranked() {
    let pool = require_database!();
    let pool_address = "pool_engine_roundtrip_accept";
    let meme_mint = "MemeAcceptMint11111111111111111111111111111";
    integration::reset_pool_fixture(&pool, pool_address).await;

    integration::ensure_pool_with(&pool, pool_address, |shared, _params| {
        shared.token_y = meme_mint.to_string();
    })
    .await;
    upsert_token(
        &pool,
        &meme_token(meme_mint, None, integration::fixture_time()),
    )
    .await
    .expect("writing the clean meme token");

    let (input, mut vol, mut regime, cfg) =
        build_input_from_database(&pool, pool_address, meme_mint).await;

    let result = rank(input, &Dlmm, &mut vol, &mut regime, &cfg);

    assert_eq!(
        result.indicators.regime,
        Some(Regime::V1),
        "regime classification is unaffected by the risk gate"
    );
    assert!(
        result.indicators.r_org.is_some(),
        "a clean pool must clear the risk gate and reach the ranking stage"
    );
    assert!(
        result.indicators.f_hat.is_some(),
        "the fee forecast must have run for an accepted pool"
    );

    // Persist and read the decision back the same way the write-path tests do, closing the
    // loop: the numbers asserted above are not just what `rank` returned in memory, they are
    // what a fresh query over the row it wrote reports too.
    let row = to_indicator_row(&result.indicators);
    upsert_indicators(&pool, Timeframe::M5, std::slice::from_ref(&row))
        .await
        .expect("persisting the indicators row");
    let detail = storage::queries::pool_detail(&pool, pool_address)
        .await
        .expect("querying pool detail")
        .expect("pool must exist");
    let persisted = detail.m5.expect("indicators_5m row must exist");
    assert_eq!(persisted.r_org, result.indicators.r_org);
    assert_eq!(persisted.regime, Some(Regime::V1.to_string()));
}

#[tokio::test]
async fn test_pool_with_a_live_mint_authority_is_rejected_by_the_risk_gate() {
    let pool = require_database!();
    let pool_address = "pool_engine_roundtrip_reject";
    let meme_mint = "MemeRejectMint11111111111111111111111111111";
    integration::reset_pool_fixture(&pool, pool_address).await;

    integration::ensure_pool_with(&pool, pool_address, |shared, _params| {
        shared.token_y = meme_mint.to_string();
    })
    .await;
    // The one difference from the accepted case: a live mint authority on the non-quote
    // token -- the database-backed input to `mint_authority_null`, the exact check
    // `libraries::engine::risk_gate::test_mint_authority_present_fails_gate` exercises in
    // memory.
    upsert_token(
        &pool,
        &meme_token(
            meme_mint,
            Some("RiskyMintAuthority111111111111111111111111"),
            integration::fixture_time(),
        ),
    )
    .await
    .expect("writing the mintable meme token");

    let (input, mut vol, mut regime, cfg) =
        build_input_from_database(&pool, pool_address, meme_mint).await;

    let result = rank(input, &Dlmm, &mut vol, &mut regime, &cfg);

    assert_eq!(
        result.indicators.regime,
        Some(Regime::V1),
        "regime classification runs before the risk gate and is unaffected by it"
    );
    assert!(
        result.indicators.r_org.is_none(),
        "a pool with a live mint authority must be rejected before the ranking stage runs"
    );
    assert!(
        result.indicators.f_hat.is_none(),
        "the fee forecast must not run for a risk-gated-out pool"
    );
    assert!(
        result
            .rationale
            .iter()
            .any(|item| item.signal == "mint_authority_null" && !item.passed),
        "the rationale trail must record which check failed, not just that one did"
    );

    let row = to_indicator_row(&result.indicators);
    upsert_indicators(&pool, Timeframe::M5, std::slice::from_ref(&row))
        .await
        .expect("persisting the indicators row");
    let detail = storage::queries::pool_detail(&pool, pool_address)
        .await
        .expect("querying pool detail")
        .expect("pool must exist");
    let persisted = detail
        .m5
        .expect("indicators_5m row must exist even for a rejected pool");
    assert_eq!(persisted.r_org, None);
}
