use clap::Parser;
use dlmm_math::RationaleItem;

use crate::rationale;

/// Organic-flow blend and shrinkage inputs. `c_fill` is per-regime and supplied by the
/// caller; the class prior (`mu_c`, `tau_c_sq`) is per regime x age-bucket and
/// re-estimated weekly outside this crate.
#[derive(Parser, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[group(id = "organic_flow")]
pub struct OrganicFlowConfig {
    /// Assumed sample variance of the observed organic share, used to size the shrinkage
    /// weight (`B = (s²/n) / (s²/n + τ_c²)`) when the class's own dispersion has not yet
    /// been estimated.
    #[arg(long, env, default_value_t = 0.05)]
    pub default_sample_variance: f64,
}

pub struct OrganicFlowInput {
    pub sigma_d: f64,
    pub bin_step: f64,
    pub active_bin_liquidity: f64,
    pub c_fill: f64,
    pub vol_24h: f64,
    pub trade_sizes: Vec<f64>,
    pub phi_time: Option<f64>,
    pub n_trades: u32,
    pub class_prior_mu: f64,
    pub class_prior_tau_sq: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct OrganicFlowOutput {
    pub phi_mech: f64,
    pub phi_size: Option<f64>,
    pub phi_time: Option<f64>,
    pub phi_org: f64,
}

/// Organic-flow stage: blend the three estimators, then shrink toward the class prior.
/// `phi_time` degrades to the `n_trades = 0` case when the ingestion backend has no
/// swap-level data -- that degradation happens in [`dlmm_math::phi_org_blend`] itself;
/// this stage just records whether it was available this tick.
pub fn evaluate(
    input: &OrganicFlowInput,
    cfg: &OrganicFlowConfig,
) -> (OrganicFlowOutput, Vec<RationaleItem>) {
    let phi_mech = dlmm_math::phi_mech(
        input.sigma_d,
        input.bin_step,
        input.active_bin_liquidity,
        input.c_fill,
        input.vol_24h,
    );
    let phi_size = dlmm_math::phi_size(&input.trade_sizes);

    let phi_obs = dlmm_math::phi_org_blend(
        input.phi_time,
        input.n_trades,
        phi_mech,
        phi_size.unwrap_or(phi_mech),
    );
    let phi_org = dlmm_math::shrink_phi_org(
        phi_obs,
        input.n_trades,
        cfg.default_sample_variance,
        input.class_prior_mu,
        input.class_prior_tau_sq,
    );

    let rationale = vec![
        rationale::info("phi_mech", phi_mech),
        rationale::info(
            "phi_size_fit_available",
            if phi_size.is_some() { 1.0 } else { 0.0 },
        ),
        rationale::info(
            "phi_time_available",
            if input.phi_time.is_some() { 1.0 } else { 0.0 },
        ),
        rationale::info("phi_org_observed", phi_obs),
        rationale::info("phi_org_shrunk", phi_org),
    ];

    (
        OrganicFlowOutput {
            phi_mech,
            phi_size,
            phi_time: input.phi_time,
            phi_org,
        },
        rationale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OrganicFlowConfig {
        OrganicFlowConfig::parse_from(["engine"])
    }

    #[test]
    fn test_degrades_to_mech_size_blend_when_phi_time_unavailable() {
        let input = OrganicFlowInput {
            sigma_d: 0.18,
            bin_step: 0.01,
            active_bin_liquidity: 12_000.0,
            c_fill: 0.5,
            vol_24h: 4_500_000.0,
            trade_sizes: Vec::new(),
            phi_time: None,
            n_trades: 0,
            class_prior_mu: 0.6,
            class_prior_tau_sq: 0.01,
        };
        let (out, rationale) = evaluate(&input, &cfg());
        assert!(out.phi_time.is_none());
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "phi_time_available" && r.observed == 0.0)
        );
    }
}
