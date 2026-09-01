//! Pure classification of a persisted indicator row into a signal-worthy kind, or nothing.
//! Reuses `engine::ranking::RankingConfig`'s own per-regime thresholds rather than
//! duplicating them, and `engine::triggers::evaluate`'s own exit decision rather than
//! reimplementing it -- this module only decides which *kind* a signal is, never whether an
//! attractiveness or exit condition holds.

use std::str::FromStr;

use engine::Regime;
use engine::ranking::RankingConfig;
use storage::write::IndicatorRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Potential,
    Degrading,
    GateFail,
}

impl SignalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalKind::Potential => "POTENTIAL",
            SignalKind::Degrading => "DEGRADING",
            SignalKind::GateFail => "GATE_FAIL",
        }
    }
}

/// Only watched (quality A) rows are eligible: a screening estimate is not something worth
/// announcing on its own, per the two-stage design.
pub fn classify(
    row: &IndicatorRow,
    cfg: &RankingConfig,
    exit_triggered: bool,
) -> Option<SignalKind> {
    if row.quality != "A" {
        return None;
    }

    let regime = row.regime.as_deref().and_then(|s| Regime::from_str(s).ok());
    let Some(regime) = regime else {
        // No committed regime (cold start) or the risk gate/regime stage rejected the pool
        // upstream -- the pipeline leaves r_org unset in both cases.
        return Some(SignalKind::GateFail);
    };
    let Some(r_org) = row.r_org else {
        return Some(SignalKind::GateFail);
    };

    if exit_triggered {
        return Some(SignalKind::Degrading);
    }
    if r_org >= cfg.r_min(regime) {
        return Some(SignalKind::Potential);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cfg() -> RankingConfig {
        RankingConfig::parse_from(["engine"])
    }

    fn row(quality: &str, regime: Option<&str>, r_org: Option<f64>) -> IndicatorRow {
        IndicatorRow {
            pool_address: "pool1".to_string(),
            venue: 0,
            bucket_start: chrono::Utc::now(),
            quality: quality.to_string(),
            regime: regime.map(|s| s.to_string()),
            vol_change: None,
            fee_change: None,
            tvl_change: None,
            price_change: None,
            active_tvl_change: None,
            holders_change: None,
            vol_tvl: None,
            fee_tvl: None,
            fee_active_tvl: None,
            tau_a: None,
            sigma_gk: None,
            sigma_fast: None,
            sigma_slow: None,
            sigma_d: None,
            sigma_jump: None,
            f_hat: None,
            phi_org: None,
            phi_mech: None,
            phi_time: None,
            phi_size: None,
            r_gross: None,
            r_org,
            y_fee: None,
            top_score: None,
        }
    }

    #[test]
    fn test_screening_quality_never_signals() {
        let row = row("B", Some("V1"), Some(100.0));
        assert_eq!(classify(&row, &cfg(), false), None);
    }

    #[test]
    fn test_unclassified_regime_is_gate_fail() {
        let row = row("A", None, None);
        assert_eq!(classify(&row, &cfg(), false), Some(SignalKind::GateFail));
    }

    #[test]
    fn test_missing_r_org_with_committed_regime_is_gate_fail() {
        let row = row("A", Some("V1"), None);
        assert_eq!(classify(&row, &cfg(), false), Some(SignalKind::GateFail));
    }

    #[test]
    fn test_exit_triggered_overrides_a_healthy_r_org_as_degrading() {
        let row = row("A", Some("V1"), Some(100.0));
        assert_eq!(classify(&row, &cfg(), true), Some(SignalKind::Degrading));
    }

    #[test]
    fn test_r_org_above_threshold_is_potential() {
        let row = row("A", Some("V1"), Some(cfg().r_min_v1 + 0.1));
        assert_eq!(classify(&row, &cfg(), false), Some(SignalKind::Potential));
    }

    #[test]
    fn test_r_org_below_threshold_is_no_signal() {
        let row = row("A", Some("V1"), Some(cfg().r_min_v1 - 0.1));
        assert_eq!(classify(&row, &cfg(), false), None);
    }
}
