//! Pure conversions between `engine`'s typed results and the plain rows `storage` persists.

use dlmm_math::RationaleItem;
use engine::{Indicators, venue_smallint};
use storage::write::{IndicatorRow, NewRationaleItem};
use uuid::Uuid;

pub fn to_indicator_row(indicators: &Indicators) -> IndicatorRow {
    IndicatorRow {
        pool_address: indicators.pool_address.clone(),
        venue: venue_smallint(indicators.venue),
        bucket_start: indicators.bucket_start,
        quality: indicators.quality.as_char().to_string(),
        regime: indicators.regime.map(|r| r.to_string()),
        vol_change: indicators.vol_change,
        fee_change: indicators.fee_change,
        tvl_change: indicators.tvl_change,
        price_change: indicators.price_change,
        active_tvl_change: indicators.active_tvl_change,
        holders_change: indicators.holders_change,
        vol_tvl: indicators.vol_tvl,
        fee_tvl: indicators.fee_tvl,
        fee_active_tvl: indicators.fee_active_tvl,
        tau_a: indicators.tau_a,
        sigma_gk: indicators.sigma_gk,
        sigma_fast: indicators.sigma_fast,
        sigma_slow: indicators.sigma_slow,
        sigma_d: indicators.sigma_d,
        sigma_jump: indicators.sigma_jump,
        f_hat: indicators.f_hat,
        phi_org: indicators.phi_org,
        phi_mech: indicators.phi_mech,
        phi_time: indicators.phi_time,
        phi_size: indicators.phi_size,
        r_gross: indicators.r_gross,
        r_org: indicators.r_org,
        y_fee: indicators.y_fee,
        top_score: indicators.top_score,
    }
}

// observed = NaN marks a check that was not evaluated (engine::rationale::unavailable);
// stored as the literal string so a renderer can distinguish it from a real zero.
pub fn to_rationale_rows(
    signal_id: Uuid,
    venue: i16,
    items: &[RationaleItem],
) -> Vec<NewRationaleItem> {
    items
        .iter()
        .enumerate()
        .map(|(seq, item)| NewRationaleItem {
            signal_id,
            seq: seq as i32,
            venue,
            signal: item.signal.clone(),
            observed: Some(format!("{}", item.observed)),
            cmp: Some(format!("{:?}", item.cmp)),
            threshold: Some(format!("{}", item.threshold)),
            passed: item.passed,
            note: None,
        })
        .collect()
}

pub fn config_hash(cfg: &engine::EngineConfig) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    format!("{cfg:?}").hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use dlmm_math::{Comparator, VenueId};
    use engine::{Quality, Regime};

    fn t() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn test_to_indicator_row_carries_quality_regime_and_venue() {
        let mut indicators = Indicators::empty("pool1".to_string(), VenueId::Dlmm, t(), Quality::A);
        indicators.regime = Some(Regime::V1);
        indicators.r_org = Some(0.42);

        let row = to_indicator_row(&indicators);
        assert_eq!(row.pool_address, "pool1");
        assert_eq!(row.venue, 0);
        assert_eq!(row.quality, "A");
        assert_eq!(row.regime, Some("V1".to_string()));
        assert_eq!(row.r_org, Some(0.42));
    }

    #[test]
    fn test_to_indicator_row_with_no_committed_regime_stores_none() {
        let indicators = Indicators::empty("pool1".to_string(), VenueId::Dlmm, t(), Quality::B);
        let row = to_indicator_row(&indicators);
        assert_eq!(row.quality, "B");
        assert_eq!(row.regime, None);
    }

    #[test]
    fn test_rationale_rows_preserve_order_via_sequence_number() {
        let items = vec![
            engine_rationale_check("first", 1.0, Comparator::Ge, 0.5),
            engine_rationale_check("second", 0.1, Comparator::Lt, 0.5),
        ];
        let rows = to_rationale_rows(Uuid::nil(), 0, &items);
        assert_eq!(rows[0].seq, 0);
        assert_eq!(rows[0].signal, "first");
        assert_eq!(rows[1].seq, 1);
        assert_eq!(rows[1].signal, "second");
    }

    #[test]
    fn test_unavailable_check_is_stored_as_the_nan_marker_not_a_real_zero() {
        // engine::rationale::unavailable sets observed = NaN and passed = true so a
        // missing input never silently blocks the gate; the renderer distinguishes this
        // from a real observed-zero by the literal "NaN" text.
        let item = RationaleItem {
            signal: "insider_bundle_flagged".to_string(),
            observed: f64::NAN,
            cmp: Comparator::Ge,
            threshold: 0.0,
            passed: true,
        };
        let rows = to_rationale_rows(Uuid::nil(), 0, std::slice::from_ref(&item));
        assert_eq!(rows[0].observed, Some("NaN".to_string()));
        assert_ne!(rows[0].observed, Some("0".to_string()));
        assert!(rows[0].passed);
    }

    #[test]
    fn test_config_hash_is_stable_and_sensitive_to_change() {
        let cfg_a = engine::EngineConfig::parse_from(["engine"]);
        let cfg_b = engine::EngineConfig::parse_from(["engine"]);
        assert_eq!(
            config_hash(&cfg_a),
            config_hash(&cfg_b),
            "identical config must hash identically"
        );

        let mut cfg_c = engine::EngineConfig::parse_from(["engine"]);
        cfg_c.ranking.r_min_v1 += 1.0;
        assert_ne!(
            config_hash(&cfg_a),
            config_hash(&cfg_c),
            "changing a config value must change the hash"
        );
    }

    fn engine_rationale_check(
        signal: &str,
        observed: f64,
        cmp: Comparator,
        threshold: f64,
    ) -> RationaleItem {
        RationaleItem {
            signal: signal.to_string(),
            observed,
            cmp,
            threshold,
            passed: match cmp {
                Comparator::Ge => observed >= threshold,
                Comparator::Le => observed <= threshold,
                Comparator::Gt => observed > threshold,
                Comparator::Lt => observed < threshold,
            },
        }
    }
}
