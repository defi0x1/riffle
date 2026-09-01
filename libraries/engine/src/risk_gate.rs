use std::time::Duration;

use clap::Parser;
use dlmm_math::{Comparator, RationaleItem};

use crate::indicators::Regime;
use crate::rationale;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteAsset {
    Sol,
    Usdc,
    Usdt,
    Other,
}

/// Risk-gate thresholds. Runs before any attractiveness metric; a failing pool is not
/// scored at all.
#[derive(Parser, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[group(id = "risk_gate")]
pub struct RiskGateConfig {
    /// Top-10 holder share must be below this to pass, when holder data is available.
    #[arg(long, env, default_value_t = 0.35)]
    pub top10_holder_share_max: f64,
    /// Single-wallet holder share must be below this to pass, when holder data is available.
    #[arg(long, env, default_value_t = 0.15)]
    pub top1_holder_share_max: f64,
    /// Token-2022 transfer fee must be at or below this (bps) to pass.
    #[arg(long, env, default_value_t = 100)]
    pub transfer_fee_bps_max: u16,
    /// Depth on another venue (or a CEX listing) must reach this fraction of DLMM depth
    /// to pass the liquidity-elsewhere check.
    #[arg(long, env, default_value_t = 0.20)]
    pub other_venue_min_depth_ratio: f64,
    /// A base-fee change within this window fails the creator-behaviour check.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "7d")]
    pub creator_fee_change_window: Duration,
    /// Share of 24h volume from the top wash-screen signers must be below this to pass.
    #[arg(long, env, default_value_t = 0.40)]
    pub wash_signer_volume_share_max: f64,
    /// Number of signers the wash-screen volume share is measured over.
    #[arg(long, env, default_value_t = 5)]
    pub wash_signer_count: u32,
    /// Round-trip volume ratio must be below this to pass the wash screen.
    #[arg(long, env, default_value_t = 0.30)]
    pub wash_round_trip_ratio_max: f64,
    /// Minimum pool age for V2, since first liquidity.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "72h")]
    pub v2_min_age: Duration,
}

/// Raw signals the risk gate needs, assembled by the caller from chain data and the
/// public API. `Option` fields that are `None` mean the signal is not available to us at
/// all, not that it was checked and found absent.
#[derive(Debug, Clone)]
pub struct RiskGateInputs {
    pub mint_authority_present: bool,
    pub freeze_authority_present: bool,
    /// True if a present freeze authority is a documented multisig for a major, the one
    /// exception this gate allows.
    pub freeze_authority_is_documented_multisig: bool,
    pub token2022_has_permanent_delegate: bool,
    pub token2022_has_transfer_hook: bool,
    pub token2022_transfer_fee_bps: u16,
    /// Not available for free -- needs a holder scan or a paid provider.
    pub top10_holder_share: Option<f64>,
    pub top1_holder_share: Option<f64>,
    /// Not available for free -- RugCheck's insider/bundle danger tier.
    pub insider_bundle_flagged: Option<bool>,
    /// From Jupiter routes; `None` if no route data could be fetched.
    pub other_venue_depth_ratio: Option<f64>,
    pub cex_listed: bool,
    pub days_since_last_fee_param_change: f64,
    pub pool_status_enabled: bool,
    pub activation_passed: bool,
    pub quote_asset: QuoteAsset,
    pub signer_top_n_share_of_24h_volume: f64,
    pub round_trip_ratio: f64,
    pub age_hours: f64,
}

// NOTE: two more checks belong here and are not implemented yet -- sellability (round-
// trip Jupiter quote, catches honeypots no static authority check would) and copycat-
// ticker detection (homoglyph-fold a symbol and test membership against verified
// symbols). Both are reproducible from public APIs; when built, they slot in as
// additional fields on `RiskGateInputs` and additional checks below.

#[derive(Debug, Clone)]
pub struct RiskGateOutput {
    pub passed: bool,
}

/// The risk gate. Every row runs, and every row is recorded, even after the gate has
/// already failed -- that is what lets `/why` explain a rejection in full rather than at
/// the first failing check.
pub fn evaluate(
    inputs: &RiskGateInputs,
    regime: Regime,
    cfg: &RiskGateConfig,
) -> (RiskGateOutput, Vec<RationaleItem>) {
    let mut rationale = Vec::new();

    rationale.push(rationale::check(
        "mint_authority_null",
        b2f(inputs.mint_authority_present),
        Comparator::Le,
        0.0,
    ));

    let freeze_ok =
        !inputs.freeze_authority_present || inputs.freeze_authority_is_documented_multisig;
    rationale.push(rationale::check(
        "freeze_authority_null_or_documented",
        b2f(!freeze_ok),
        Comparator::Le,
        0.0,
    ));

    rationale.push(rationale::check(
        "token2022_no_permanent_delegate",
        b2f(inputs.token2022_has_permanent_delegate),
        Comparator::Le,
        0.0,
    ));
    rationale.push(rationale::check(
        "token2022_no_transfer_hook",
        b2f(inputs.token2022_has_transfer_hook),
        Comparator::Le,
        0.0,
    ));
    rationale.push(rationale::check(
        "token2022_transfer_fee_bps",
        inputs.token2022_transfer_fee_bps as f64,
        Comparator::Le,
        cfg.transfer_fee_bps_max as f64,
    ));

    rationale.push(match inputs.top10_holder_share {
        Some(share) => rationale::check(
            "top10_holder_share",
            share,
            Comparator::Lt,
            cfg.top10_holder_share_max,
        ),
        None => rationale::unavailable(
            "top10_holder_share",
            Comparator::Lt,
            cfg.top10_holder_share_max,
        ),
    });
    rationale.push(match inputs.top1_holder_share {
        Some(share) => rationale::check(
            "top1_holder_share",
            share,
            Comparator::Lt,
            cfg.top1_holder_share_max,
        ),
        None => rationale::unavailable(
            "top1_holder_share",
            Comparator::Lt,
            cfg.top1_holder_share_max,
        ),
    });
    rationale.push(match inputs.insider_bundle_flagged {
        Some(flagged) => rationale::check(
            "insider_bundle_below_danger_tier",
            b2f(flagged),
            Comparator::Le,
            0.0,
        ),
        None => rationale::unavailable("insider_bundle_below_danger_tier", Comparator::Le, 0.0),
    });

    rationale.push(match inputs.other_venue_depth_ratio {
        Some(ratio) => {
            // A CEX listing satisfies this check on its own, regardless of the measured
            // route depth, so fold it into the observed value rather than the threshold.
            let effective = if inputs.cex_listed {
                ratio.max(cfg.other_venue_min_depth_ratio)
            } else {
                ratio
            };
            rationale::check(
                "liquidity_elsewhere",
                effective,
                Comparator::Ge,
                cfg.other_venue_min_depth_ratio,
            )
        }
        None if inputs.cex_listed => rationale::check(
            "liquidity_elsewhere",
            cfg.other_venue_min_depth_ratio,
            Comparator::Ge,
            cfg.other_venue_min_depth_ratio,
        ),
        None => rationale::unavailable(
            "liquidity_elsewhere",
            Comparator::Ge,
            cfg.other_venue_min_depth_ratio,
        ),
    });

    let fee_change_window_days = cfg.creator_fee_change_window.as_secs_f64() / 86_400.0;
    rationale.push(rationale::check(
        "creator_no_recent_fee_change",
        inputs.days_since_last_fee_param_change,
        Comparator::Ge,
        fee_change_window_days,
    ));
    rationale.push(rationale::check(
        "creator_status_enabled",
        b2f(!inputs.pool_status_enabled),
        Comparator::Le,
        0.0,
    ));
    rationale.push(rationale::check(
        "creator_activation_passed",
        b2f(!inputs.activation_passed),
        Comparator::Le,
        0.0,
    ));

    let quote_ok = matches!(
        inputs.quote_asset,
        QuoteAsset::Sol | QuoteAsset::Usdc | QuoteAsset::Usdt
    );
    rationale.push(rationale::check(
        "quote_asset_allowed",
        b2f(!quote_ok),
        Comparator::Le,
        0.0,
    ));

    rationale.push(rationale::check(
        "wash_signer_volume_share",
        inputs.signer_top_n_share_of_24h_volume,
        Comparator::Le,
        cfg.wash_signer_volume_share_max,
    ));
    rationale.push(rationale::check(
        "wash_round_trip_ratio",
        inputs.round_trip_ratio,
        Comparator::Lt,
        cfg.wash_round_trip_ratio_max,
    ));

    if regime == Regime::V2 {
        let min_age_hours = cfg.v2_min_age.as_secs_f64() / 3_600.0;
        rationale.push(rationale::check(
            "v2_minimum_age_hours",
            inputs.age_hours,
            Comparator::Ge,
            min_age_hours,
        ));
    }

    let passed = rationale.iter().all(|r| r.passed);
    (RiskGateOutput { passed }, rationale)
}

fn b2f(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RiskGateConfig {
        RiskGateConfig::parse_from(["engine"])
    }

    fn clean_inputs() -> RiskGateInputs {
        RiskGateInputs {
            mint_authority_present: false,
            freeze_authority_present: false,
            freeze_authority_is_documented_multisig: false,
            token2022_has_permanent_delegate: false,
            token2022_has_transfer_hook: false,
            token2022_transfer_fee_bps: 0,
            top10_holder_share: None,
            top1_holder_share: None,
            insider_bundle_flagged: None,
            other_venue_depth_ratio: Some(0.5),
            cex_listed: false,
            days_since_last_fee_param_change: 30.0,
            pool_status_enabled: true,
            activation_passed: true,
            quote_asset: QuoteAsset::Sol,
            signer_top_n_share_of_24h_volume: 0.1,
            round_trip_ratio: 0.05,
            age_hours: 1000.0,
        }
    }

    #[test]
    fn test_clean_pool_passes_v1() {
        let (out, rationale) = evaluate(&clean_inputs(), Regime::V1, &cfg());
        assert!(out.passed);
        assert!(rationale.iter().all(|r| r.passed));
    }

    #[test]
    fn test_missing_holder_data_degrades_gracefully_not_silently() {
        let (out, rationale) = evaluate(&clean_inputs(), Regime::V1, &cfg());
        let item = rationale
            .iter()
            .find(|r| r.signal == "top10_holder_share")
            .expect("holder rationale present");
        assert!(
            item.observed.is_nan(),
            "unavailable check must be marked, not silently passed"
        );
        assert!(item.passed);
        assert!(out.passed);
    }

    #[test]
    fn test_mint_authority_present_fails_gate() {
        let mut inputs = clean_inputs();
        inputs.mint_authority_present = true;
        let (out, rationale) = evaluate(&inputs, Regime::V1, &cfg());
        assert!(!out.passed);
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "mint_authority_null" && !r.passed)
        );
    }

    #[test]
    fn test_v2_age_gate_applies_only_to_v2() {
        let mut inputs = clean_inputs();
        inputs.age_hours = 1.0;
        let (out_v1, _) = evaluate(&inputs, Regime::V1, &cfg());
        let (out_v2, rationale_v2) = evaluate(&inputs, Regime::V2, &cfg());
        assert!(out_v1.passed);
        assert!(!out_v2.passed);
        assert!(
            rationale_v2
                .iter()
                .any(|r| r.signal == "v2_minimum_age_hours" && !r.passed)
        );
    }

    #[test]
    fn test_every_check_recorded_even_after_a_failure() {
        let mut inputs = clean_inputs();
        inputs.mint_authority_present = true;
        inputs.round_trip_ratio = 0.9;
        let (out, rationale) = evaluate(&inputs, Regime::V1, &cfg());
        assert!(!out.passed);
        // Both failing checks are present -- the gate does not stop recording at the
        // first failure.
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "mint_authority_null" && !r.passed)
        );
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "wash_round_trip_ratio" && !r.passed)
        );
    }
}
