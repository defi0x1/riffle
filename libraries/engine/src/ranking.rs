use clap::Parser;
use dlmm_math::{Comparator, PoolState, RationaleItem, Venue, VolEstimate};

use crate::indicators::Regime;
use crate::rationale;

/// Attractiveness-gate thresholds, one triple per regime. `R_min` sits well above the
/// derived breakeven deliberately -- a model-error budget, not a claim about the true
/// breakeven; the real calibrated values live outside this repo, these are neutral
/// placeholders.
#[derive(Parser, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[group(id = "ranking")]
pub struct RankingConfig {
    #[arg(long, env, default_value_t = 1.5)]
    pub r_min_s: f64,
    #[arg(long, env, default_value_t = 2.0)]
    pub r_min_v1: f64,
    #[arg(long, env, default_value_t = 3.0)]
    pub r_min_v2: f64,

    #[arg(long, env, default_value_t = 1.0)]
    pub vol_tvl_min_s: f64,
    #[arg(long, env, default_value_t = 1.5)]
    pub vol_tvl_min_v1: f64,
    #[arg(long, env, default_value_t = 4.0)]
    pub vol_tvl_min_v2: f64,

    /// Organic floor -- locked, not tuned.
    #[arg(long, env, default_value_t = 0.50)]
    pub phi_org_min_s: f64,
    #[arg(long, env, default_value_t = 0.40)]
    pub phi_org_min_v1: f64,
    #[arg(long, env, default_value_t = 0.50)]
    pub phi_org_min_v2: f64,

    /// Annualised `Y_fee` floor (fraction, e.g. 0.08 = 8%/yr).
    #[arg(long, env, default_value_t = 0.08)]
    pub y_fee_annual_min_s: f64,
    #[arg(long, env, default_value_t = 0.25)]
    pub y_fee_annual_min_v1: f64,
    #[arg(long, env, default_value_t = 1.50)]
    pub y_fee_annual_min_v2: f64,

    #[arg(long, env, default_value_t = 1_000_000.0)]
    pub tvl_min_s: f64,
    #[arg(long, env, default_value_t = 500_000.0)]
    pub tvl_min_v1: f64,
    #[arg(long, env, default_value_t = 150_000.0)]
    pub tvl_min_v2: f64,

    #[arg(long, env, default_value_t = 2_000_000.0)]
    pub vol24h_min_s: f64,
    #[arg(long, env, default_value_t = 1_000_000.0)]
    pub vol24h_min_v1: f64,
    #[arg(long, env, default_value_t = 500_000.0)]
    pub vol24h_min_v2: f64,

    /// S and V1: 7-day volume trend must not be down more than this (fraction, negative).
    #[arg(long, env, default_value_t = -0.50)]
    pub volume_trend_min_wk_wk: f64,
    /// V2, age < 7d: 24h volume must reach this fraction of the trailing-72h average.
    #[arg(long, env, default_value_t = 0.35)]
    pub volume_trend_min_v2_young: f64,
    /// V2, age >= 7d: 24h volume must reach this fraction of the trailing-72h average.
    #[arg(long, env, default_value_t = 0.50)]
    pub volume_trend_min_v2_mature: f64,

    /// JIT haircut applied inside `R_org`.
    #[arg(long, env, default_value_t = 0.05)]
    pub h_jit_s: f64,
    #[arg(long, env, default_value_t = 0.10)]
    pub h_jit_v1: f64,
    #[arg(long, env, default_value_t = 0.15)]
    pub h_jit_v2: f64,

    /// Annual hurdle used to size the ranking key's dilution-adjusted position cap, via
    /// the same self-dilution cap the sizing stage uses. No calibrated value ships with
    /// the repo; this is a neutral placeholder.
    #[arg(long, env, default_value_t = 0.20)]
    pub ranking_key_hurdle_annual: f64,

    /// Multi-window consistency filter: the minimum of the 1h/24h/7d fee/TVL windows must
    /// reach this fraction of the 24h window, or the pool is judged inconsistent across
    /// horizons. Our own choice of coefficient, not a calibrated value.
    #[arg(long, env, default_value_t = 0.5)]
    pub consistency_min_ratio: f64,
}

impl RankingConfig {
    pub fn r_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.r_min_s,
            Regime::V1 => self.r_min_v1,
            Regime::V2 => self.r_min_v2,
        }
    }
    pub fn vol_tvl_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.vol_tvl_min_s,
            Regime::V1 => self.vol_tvl_min_v1,
            Regime::V2 => self.vol_tvl_min_v2,
        }
    }
    pub fn phi_org_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.phi_org_min_s,
            Regime::V1 => self.phi_org_min_v1,
            Regime::V2 => self.phi_org_min_v2,
        }
    }
    pub fn y_fee_annual_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.y_fee_annual_min_s,
            Regime::V1 => self.y_fee_annual_min_v1,
            Regime::V2 => self.y_fee_annual_min_v2,
        }
    }
    pub fn tvl_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.tvl_min_s,
            Regime::V1 => self.tvl_min_v1,
            Regime::V2 => self.tvl_min_v2,
        }
    }
    pub fn vol24h_min(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.vol24h_min_s,
            Regime::V1 => self.vol24h_min_v1,
            Regime::V2 => self.vol24h_min_v2,
        }
    }
    pub fn h_jit(&self, regime: Regime) -> f64 {
        match regime {
            Regime::S => self.h_jit_s,
            Regime::V1 => self.h_jit_v1,
            Regime::V2 => self.h_jit_v2,
        }
    }
}

pub struct RankingInput {
    pub pool: PoolState,
    pub vol: VolEstimate,
    pub vol_24h: f64,
    pub phi_org: f64,
    pub tvl_usd: f64,
    /// 7-day wk/wk trend for S/V1 (fraction change, negative = down); for V2 this is
    /// `vol_24h / avg_vol_72h`.
    pub volume_trend: f64,
    /// V2 only: whether the pool is younger than 7 days (picks the young/mature volume
    /// trend threshold).
    pub v2_is_young: bool,
    /// The 1h/24h/7d fee/TVL windows, when available, for the consistency filter.
    pub fee_tvl_1h: Option<f64>,
    pub fee_tvl_24h: Option<f64>,
    pub fee_tvl_7d: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub struct RankingOutput {
    pub r_gross: f64,
    pub r_org: f64,
    pub y_fee_daily: f64,
    pub y_fee_annual: f64,
    pub vol_tvl_24h: f64,
    pub tau_a: f64,
    pub attractive: bool,
    pub ranking_key: f64,
}

/// Ranking stage: `R_org`, `Y_fee`, the attractiveness gate, and the ranking key. Written
/// once against the `Venue` trait, so a second venue costs nothing here beyond its own
/// `fee_rate`/`turnover_base`/`lvr_geometry`.
pub fn evaluate<V: Venue>(
    venue: &V,
    input: &RankingInput,
    regime: Regime,
    cfg: &RankingConfig,
) -> (RankingOutput, Vec<RationaleItem>) {
    let mut rationale = Vec::new();

    let fee = venue.fee_rate(&input.pool, &input.vol).ok();
    let f_hat = fee.map(|f| f.forecast).unwrap_or(0.0);
    let l_a = venue.turnover_base(&input.pool).unwrap_or(0.0);
    let tau_a = if l_a > 0.0 { input.vol_24h / l_a } else { 0.0 };
    let geometry = venue.lvr_geometry(&input.pool);

    let h_jit = cfg.h_jit(regime);
    let r_gross = dlmm_math::r_ratio(
        f_hat,
        tau_a,
        geometry,
        input.pool.protocol_share,
        input.vol.sigma_d,
    );
    let r_org = dlmm_math::r_org(r_gross, input.phi_org, h_jit);

    let y_fee_daily = dlmm_math::y_fee(input.pool.protocol_share, f_hat, tau_a, h_jit, l_a, 0.0);
    let y_fee_annual = y_fee_daily * 365.0;

    let vol_tvl_24h = if input.tvl_usd > 0.0 {
        input.vol_24h / input.tvl_usd
    } else {
        0.0
    };

    rationale.push(rationale::check(
        "r_org",
        r_org,
        Comparator::Ge,
        cfg.r_min(regime),
    ));
    rationale.push(rationale::check(
        "vol_tvl_24h",
        vol_tvl_24h,
        Comparator::Ge,
        cfg.vol_tvl_min(regime),
    ));
    rationale.push(rationale::check(
        "phi_org",
        input.phi_org,
        Comparator::Ge,
        cfg.phi_org_min(regime),
    ));
    rationale.push(rationale::check(
        "y_fee_annual",
        y_fee_annual,
        Comparator::Ge,
        cfg.y_fee_annual_min(regime),
    ));
    rationale.push(rationale::check(
        "tvl_usd",
        input.tvl_usd,
        Comparator::Ge,
        cfg.tvl_min(regime),
    ));
    rationale.push(rationale::check(
        "vol_24h",
        input.vol_24h,
        Comparator::Ge,
        cfg.vol24h_min(regime),
    ));

    let volume_trend_threshold = match regime {
        Regime::S | Regime::V1 => cfg.volume_trend_min_wk_wk,
        Regime::V2 if input.v2_is_young => cfg.volume_trend_min_v2_young,
        Regime::V2 => cfg.volume_trend_min_v2_mature,
    };
    rationale.push(rationale::check(
        "volume_trend",
        input.volume_trend,
        Comparator::Ge,
        volume_trend_threshold,
    ));

    if let (Some(h1), Some(h24), Some(d7)) = (input.fee_tvl_1h, input.fee_tvl_24h, input.fee_tvl_7d)
    {
        let consistency_min = (h1 * 24.0).min(h24).min(d7 / 7.0);
        let threshold = h24 * cfg.consistency_min_ratio;
        rationale.push(rationale::check(
            "multi_window_consistency",
            consistency_min,
            Comparator::Ge,
            threshold,
        ));
    }

    let attractive = rationale.iter().all(|r| r.passed);

    let m_star = dlmm_math::self_dilution_cap(
        l_a,
        input.pool.protocol_share,
        f_hat,
        tau_a,
        cfg.ranking_key_hurdle_annual,
    );
    let ranking_key = if m_star > 0.0 {
        r_org * (input.vol_24h / (5.0 * m_star)).min(1.0)
    } else {
        r_org
    };

    (
        RankingOutput {
            r_gross,
            r_org,
            y_fee_daily,
            y_fee_annual,
            vol_tvl_24h,
            tau_a,
            attractive,
            ranking_key,
        },
        rationale,
    )
}
