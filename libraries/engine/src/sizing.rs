use clap::Parser;
use dlmm_math::{Comparator, RationaleItem};

use crate::indicators::Regime;
use crate::rationale;

/// Sizing caps: position-share, TVL-share and capital-at-risk caps per regime, plus the
/// quarter-Kelly haircut and the minimum position size below which a pool is skipped
/// entirely. No calibrated value ships with the repo; these are neutral placeholders.
#[derive(Parser, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[group(id = "sizing")]
pub struct SizingConfig {
    #[arg(long, env, default_value_t = 0.15)]
    pub theta_max_s: f64,
    #[arg(long, env, default_value_t = 0.10)]
    pub theta_max_v1: f64,
    #[arg(long, env, default_value_t = 0.08)]
    pub theta_max_v2: f64,

    #[arg(long, env, default_value_t = 0.10)]
    pub pi_max_s: f64,
    #[arg(long, env, default_value_t = 0.05)]
    pub pi_max_v1: f64,
    #[arg(long, env, default_value_t = 0.03)]
    pub pi_max_v2: f64,

    #[arg(long, env, default_value_t = 0.40)]
    pub car_fraction_s: f64,
    #[arg(long, env, default_value_t = 0.20)]
    pub car_fraction_v1: f64,
    #[arg(long, env, default_value_t = 0.10)]
    pub car_fraction_v2: f64,

    #[arg(long, env, default_value_t = 5_000.0)]
    pub v_min_s: f64,
    #[arg(long, env, default_value_t = 3_000.0)]
    pub v_min_v1: f64,
    #[arg(long, env, default_value_t = 1_000.0)]
    pub v_min_v2: f64,

    /// Annual hurdle used to compute the self-dilution cap `m*`.
    #[arg(long, env, default_value_t = 0.20)]
    pub annual_hurdle: f64,
    /// Position count `N` for Spot: `V = N·m`.
    #[arg(long, env, default_value_t = 5)]
    pub position_count: u32,
}

impl SizingConfig {
    pub fn theta_max(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.theta_max_s,
            Regime::V1 => self.theta_max_v1,
            Regime::V2 => self.theta_max_v2,
        }
    }
    pub fn pi_max(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.pi_max_s,
            Regime::V1 => self.pi_max_v1,
            Regime::V2 => self.pi_max_v2,
        }
    }
    pub fn car_fraction(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.car_fraction_s,
            Regime::V1 => self.car_fraction_v1,
            Regime::V2 => self.car_fraction_v2,
        }
    }
    pub fn v_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.v_min_s,
            Regime::V1 => self.v_min_v1,
            Regime::V2 => self.v_min_v2,
        }
    }
}

pub struct SizingInput {
    pub active_bin_liquidity: f64,
    pub protocol_share: f64,
    pub fee_rate: f64,
    pub tau_a: f64,
    pub tvl_usd: f64,
    pub regime_capital: f64,
    pub mu_fee: f64,
    pub mu_arb: f64,
    pub sigma_hat: f64,
    pub free_capital: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct SizingOutput {
    pub m_dilution: f64,
    pub m_share: f64,
    pub v_tvl: f64,
    pub v_car: f64,
    pub quarter_kelly_fraction: f64,
    pub v_star: Option<f64>,
}

/// Sizing stage: four caps, then quarter-Kelly, then the minimum of everything
/// (including free capital) -- skip the pool if that minimum is below `v_min`.
pub fn evaluate(
    input: &SizingInput,
    regime: Regime,
    cfg: &SizingConfig,
) -> (SizingOutput, Vec<RationaleItem>) {
    let m_dilution = dlmm_math::self_dilution_cap(
        input.active_bin_liquidity,
        input.protocol_share,
        input.fee_rate,
        input.tau_a,
        cfg.annual_hurdle,
    );
    let m_share = dlmm_math::share_cap(input.active_bin_liquidity, cfg.theta_max(regime));
    let v_tvl = dlmm_math::tvl_cap(input.tvl_usd, cfg.pi_max(regime));
    let v_car = dlmm_math::car_cap(input.regime_capital, cfg.car_fraction(regime));
    let quarter_kelly_fraction =
        dlmm_math::quarter_kelly(input.mu_fee, input.mu_arb, input.sigma_hat);
    let kelly_cap = quarter_kelly_fraction * input.regime_capital;

    let n_m_share = cfg.position_count as f64 * m_share;
    let caps = [
        n_m_share,
        v_tvl,
        v_car,
        kelly_cap.max(0.0),
        input.free_capital,
    ];
    let v_min = cfg.v_min(regime);
    let v_star = dlmm_math::position_size(&caps, v_min);

    let binding_min = caps.iter().cloned().fold(f64::INFINITY, f64::min);
    let rationale = vec![
        rationale::info("m_dilution", m_dilution),
        rationale::info("m_share_per_bin", m_share),
        rationale::info("v_tvl_cap", v_tvl),
        rationale::info("v_car_cap", v_car),
        rationale::info("quarter_kelly_fraction", quarter_kelly_fraction),
        rationale::check("position_size", binding_min, Comparator::Ge, v_min),
    ];

    (
        SizingOutput {
            m_dilution,
            m_share,
            v_tvl,
            v_car,
            quarter_kelly_fraction,
            v_star,
        },
        rationale,
    )
}
