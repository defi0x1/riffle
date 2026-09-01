use chrono::{DateTime, Utc};
use dlmm_math::{PoolState, RationaleItem, Venue, VenueId, VolEstimate};
use rust_decimal::Decimal;

use crate::config::EngineConfig;
use crate::indicators::{Indicators, Quality, Regime};
use crate::regime::RegimeState;
use crate::risk_gate::RiskGateInputs;
use crate::triggers::HistoryPoint;
use crate::volatility::{OhlcBar, VolatilityState};
use crate::{fee_forecast, organic_flow, ranking, regime, risk_gate, sizing, triggers};

/// Active-bin liquidity as a fraction of TVL, used only when real bin state has not been
/// measured. Chosen per regime: stable pairs concentrate liquidity in a handful of bins
/// at a 1 bp step, so a much larger share of TVL sits at the active bin than for a
/// volatile pair spread across a wide range.
pub fn phi_shape(regime: Regime) -> f64 {
    match regime {
        Regime::S => 0.16,
        Regime::V1 => 0.04,
        Regime::V2 => 0.02,
    }
}

/// The previous bucket's raw readings, carried alongside this bucket's row so a renderer
/// can show both endpoints without a self-join.
#[derive(Debug, Clone, Copy, Default)]
pub struct PreviousBucket {
    pub vol: Option<f64>,
    pub fee: Option<f64>,
    pub tvl: Option<f64>,
    pub price: Option<f64>,
    pub active_tvl: Option<f64>,
    pub holders: Option<f64>,
}

/// Everything one pipeline evaluation needs, grouped roughly by the stage that consumes
/// it. `measured_active_bin_liquidity` is only read when `quality` is [`Quality::A`];
/// `tvl_usd` is always read (it feeds both the screening estimate and the ranking gate).
pub struct PipelineInput {
    pub pool_address: String,
    pub venue: VenueId,
    pub bucket_start: DateTime<Utc>,
    pub now: DateTime<Utc>,

    pub latest_bar: OhlcBar,
    pub autocorrelations: Vec<f64>,
    pub log_returns_24h: Vec<f64>,
    pub decay_window_secs: f64,

    pub dev_peg: Option<f64>,
    pub is_pegged_whitelisted: bool,
    pub is_major: bool,
    pub age_days: f64,
    pub kill_switch: bool,

    pub risk: RiskGateInputs,

    pub bin_step_bps: u16,
    pub base_factor: u16,
    pub base_fee_power_factor: u8,
    pub variable_fee_control: u32,
    pub protocol_share: f64,
    pub tvl_usd: f64,
    pub measured_active_bin_liquidity: Option<f64>,
    pub kappa_c: f64,

    pub trade_sizes: Vec<f64>,
    pub phi_time: Option<f64>,
    pub n_trades: u32,
    pub c_fill: f64,
    pub vol_24h: f64,
    pub organic_class_prior_mu: f64,
    pub organic_class_prior_tau_sq: f64,

    pub volume_trend: f64,
    pub v2_is_young: bool,
    pub fee_tvl_1h: Option<f64>,
    pub fee_tvl_24h: Option<f64>,
    pub fee_tvl_7d: Option<f64>,

    pub regime_capital: f64,
    pub mu_fee: f64,
    pub mu_arb: f64,
    pub free_capital: f64,

    pub trigger_history: Vec<HistoryPoint>,
    pub fee_jack_multiplier: Option<f64>,
    pub is_weekend_utc: bool,

    pub previous: PreviousBucket,
}

/// One evaluation's full result: the row to persist, and every check that ran on the way
/// to it, in stage order, whether or not any of them changed the outcome.
pub struct EvaluationResult {
    pub indicators: Indicators,
    pub rationale: Vec<RationaleItem>,
}

/// Screen a pool: active-bin liquidity is estimated from TVL, and the row is tagged as
/// the weaker, unmeasured kind of evidence. Runs over every pool in the universe.
pub fn screen<V: Venue>(
    input: PipelineInput,
    venue: &V,
    vol_state: &mut VolatilityState,
    regime_state: &mut RegimeState,
    cfg: &EngineConfig,
) -> EvaluationResult {
    evaluate(input, Quality::B, venue, vol_state, regime_state, cfg)
}

/// Rank a pool: active-bin liquidity is read from measured bin state, and the row counts
/// toward outcome scoring. Runs only over the pools actually being watched.
pub fn rank<V: Venue>(
    input: PipelineInput,
    venue: &V,
    vol_state: &mut VolatilityState,
    regime_state: &mut RegimeState,
    cfg: &EngineConfig,
) -> EvaluationResult {
    evaluate(input, Quality::A, venue, vol_state, regime_state, cfg)
}

fn evaluate<V: Venue>(
    input: PipelineInput,
    quality: Quality,
    venue: &V,
    vol_state: &mut VolatilityState,
    regime_state: &mut RegimeState,
    cfg: &EngineConfig,
) -> EvaluationResult {
    let mut rationale = Vec::new();
    let mut indicators = Indicators::empty(
        input.pool_address.clone(),
        input.venue,
        input.bucket_start,
        quality,
    );

    indicators.vol_change = input.previous.vol;
    indicators.fee_change = input.previous.fee;
    indicators.tvl_change = input.previous.tvl;
    indicators.price_change = input.previous.price;
    indicators.active_tvl_change = input.previous.active_tvl;
    indicators.holders_change = input.previous.holders;

    let (vol_out, vol_rationale) = crate::volatility::evaluate(
        vol_state,
        input.latest_bar,
        &input.autocorrelations,
        &input.log_returns_24h,
        input.decay_window_secs,
        input.now,
    );
    rationale.extend(vol_rationale);
    indicators.sigma_gk = Some(vol_out.sigma_gk);
    indicators.sigma_fast = Some(vol_out.sigma_fast);
    indicators.sigma_slow = Some(vol_out.sigma_slow);
    indicators.sigma_d = Some(vol_out.sigma_d);
    indicators.sigma_jump = Some(vol_out.sigma_jump);

    let candidate = if vol_out.sufficient_history {
        regime::classify_candidate(
            regime_state.regime,
            vol_out.sigma_slow,
            vol_out.sigma_fast,
            input.dev_peg,
            input.age_days,
            input.is_pegged_whitelisted,
            input.is_major,
            &cfg.regime,
        )
    } else {
        None
    };
    let regime_rationale =
        regime_state.update(candidate, input.now, &cfg.regime, input.kill_switch);
    rationale.push(regime_rationale);
    indicators.regime = regime_state.regime;

    let Some(regime) = regime_state.regime else {
        return EvaluationResult {
            indicators,
            rationale,
        };
    };

    let (risk_out, risk_rationale) = risk_gate::evaluate(&input.risk, regime, &cfg.risk_gate);
    rationale.extend(risk_rationale);
    if !risk_out.passed {
        return EvaluationResult {
            indicators,
            rationale,
        };
    }

    let active_bin_liquidity = match quality {
        Quality::A => input
            .measured_active_bin_liquidity
            .unwrap_or_else(|| input.tvl_usd * phi_shape(regime)),
        Quality::B => input.tvl_usd * phi_shape(regime),
    };

    let pool = PoolState {
        bin_step_bps: input.bin_step_bps,
        base_factor: input.base_factor,
        base_fee_power_factor: input.base_fee_power_factor,
        variable_fee_control: input.variable_fee_control,
        active_bin_liquidity,
        protocol_share: input.protocol_share,
    };
    let vol_estimate = VolEstimate {
        sigma_d: vol_out.sigma_d,
        sigma_d_bps: vol_out.sigma_d_bps,
        kappa_c: input.kappa_c,
    };

    let (organic_out, organic_rationale) = organic_flow::evaluate(
        &organic_flow::OrganicFlowInput {
            sigma_d: vol_out.sigma_d,
            bin_step: input.bin_step_bps as f64 / 10_000.0,
            active_bin_liquidity,
            c_fill: input.c_fill,
            vol_24h: input.vol_24h,
            trade_sizes: input.trade_sizes,
            phi_time: input.phi_time,
            n_trades: input.n_trades,
            class_prior_mu: input.organic_class_prior_mu,
            class_prior_tau_sq: input.organic_class_prior_tau_sq,
        },
        &cfg.organic_flow,
    );
    rationale.extend(organic_rationale);
    indicators.phi_mech = Some(organic_out.phi_mech);
    indicators.phi_size = organic_out.phi_size;
    indicators.phi_time = organic_out.phi_time;
    indicators.phi_org = Some(organic_out.phi_org);

    let fee_result = fee_forecast::evaluate(venue, &pool, &vol_estimate);
    let Ok((fee, fee_rationale)) = fee_result else {
        rationale.push(dlmm_math::RationaleItem {
            signal: "fee_forecast_computable".to_string(),
            observed: 0.0,
            cmp: dlmm_math::Comparator::Ge,
            threshold: 1.0,
            passed: false,
        });
        return EvaluationResult {
            indicators,
            rationale,
        };
    };
    rationale.extend(fee_rationale);
    indicators.f_hat = Decimal::try_from(fee.forecast).ok();

    let (ranking_out, ranking_rationale) = ranking::evaluate(
        venue,
        &ranking::RankingInput {
            pool,
            vol: vol_estimate,
            vol_24h: input.vol_24h,
            phi_org: organic_out.phi_org,
            tvl_usd: input.tvl_usd,
            volume_trend: input.volume_trend,
            v2_is_young: input.v2_is_young,
            fee_tvl_1h: input.fee_tvl_1h,
            fee_tvl_24h: input.fee_tvl_24h,
            fee_tvl_7d: input.fee_tvl_7d,
        },
        regime,
        &cfg.ranking,
    );
    rationale.extend(ranking_rationale);
    indicators.r_gross = Some(ranking_out.r_gross);
    indicators.r_org = Some(ranking_out.r_org);
    indicators.y_fee = Some(ranking_out.y_fee_daily);
    indicators.vol_tvl = Some(ranking_out.vol_tvl_24h);
    indicators.tau_a = Some(ranking_out.tau_a);
    if input.tvl_usd > 0.0 {
        indicators.fee_tvl = Some(fee.current * ranking_out.tau_a);
        indicators.fee_active_tvl =
            Some(fee.current * ranking_out.tau_a * input.tvl_usd / active_bin_liquidity.max(1.0));
    }

    if !ranking_out.attractive || quality == Quality::B {
        return EvaluationResult {
            indicators,
            rationale,
        };
    }

    let (_sizing_out, sizing_rationale) = sizing::evaluate(
        &sizing::SizingInput {
            active_bin_liquidity,
            protocol_share: input.protocol_share,
            fee_rate: fee.forecast,
            tau_a: ranking_out.tau_a,
            tvl_usd: input.tvl_usd,
            regime_capital: input.regime_capital,
            mu_fee: input.mu_fee,
            mu_arb: input.mu_arb,
            sigma_hat: vol_out.sigma_d,
            free_capital: input.free_capital,
        },
        regime,
        &cfg.sizing,
    );
    rationale.extend(sizing_rationale);

    let (_triggers_out, triggers_rationale) = triggers::evaluate(
        &triggers::TriggersInput {
            r_org: ranking_out.r_org,
            vol_tvl: ranking_out.vol_tvl_24h,
            volume_decay_metric: input.volume_trend,
            v2_is_young: input.v2_is_young,
            fee_jack_multiplier: input.fee_jack_multiplier,
            history: input.trigger_history,
            is_weekend_utc: input.is_weekend_utc,
        },
        regime,
        &cfg.triggers,
    );
    rationale.extend(triggers_rationale);

    EvaluationResult {
        indicators,
        rationale,
    }
}
