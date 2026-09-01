use std::time::Duration;

use chrono::{DateTime, Utc};
use clap::Parser;
use dlmm_math::{Comparator, RationaleItem};

use crate::indicators::Regime;
use crate::rationale;

/// Exit-trigger thresholds and persistence windows. Each persistence window is a lookback
/// over the timeframe's own indicator history rather than a stateful accumulator, so it
/// survives restarts by construction; the values below are neutral placeholders.
#[derive(Parser, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[group(id = "triggers")]
pub struct TriggersConfig {
    #[arg(long, env, default_value_t = 1.0)]
    pub r_org_exit_s: f64,
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "24h")]
    pub r_org_exit_persistence_s: Duration,
    #[arg(long, env, default_value_t = 1.5)]
    pub r_org_exit_v1: f64,
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "6h")]
    pub r_org_exit_persistence_v1: Duration,
    #[arg(long, env, default_value_t = 2.0)]
    pub r_org_exit_v2: f64,
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "3h")]
    pub r_org_exit_persistence_v2: Duration,

    #[arg(long, env, default_value_t = 0.5)]
    pub vol_tvl_exit_s: f64,
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "48h")]
    pub vol_tvl_exit_persistence_s: Duration,
    #[arg(long, env, default_value_t = 0.75)]
    pub vol_tvl_exit_v1: f64,
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "12h")]
    pub vol_tvl_exit_persistence_v1: Duration,
    #[arg(long, env, default_value_t = 2.0)]
    pub vol_tvl_exit_v2: f64,
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "6h")]
    pub vol_tvl_exit_persistence_v2: Duration,

    /// S: 7-day wk/wk volume change floor (fraction, negative); below this is a decay exit.
    #[arg(long, env, default_value_t = -0.50)]
    pub volume_decay_wk_wk_min: f64,
    /// V1: 24h volume must reach this fraction of the trailing-72h average.
    #[arg(long, env, default_value_t = 0.40)]
    pub volume_decay_v1_min: f64,
    /// V2, age < 7d: 24h volume must reach this fraction of the trailing-72h average.
    #[arg(long, env, default_value_t = 0.35)]
    pub volume_decay_v2_young_min: f64,
    /// V2, age >= 7d: same, older pools held to the same bar as the ranking gate's mature
    /// threshold. Our own extrapolation from the young-pool figure, not an independently
    /// specified value.
    #[arg(long, env, default_value_t = 0.50)]
    pub volume_decay_v2_mature_min: f64,

    /// A fee-parameter jack at or above this multiplier is an instant kill, no
    /// persistence window.
    #[arg(long, env, default_value_t = 2.0)]
    pub fee_jack_kill_multiplier: f64,
}

impl TriggersConfig {
    fn r_org_exit(&self, regime: Regime) -> (f64, Duration) {
        match regime {
            Regime::S => (self.r_org_exit_s, self.r_org_exit_persistence_s),
            Regime::V1 => (self.r_org_exit_v1, self.r_org_exit_persistence_v1),
            Regime::V2 => (self.r_org_exit_v2, self.r_org_exit_persistence_v2),
        }
    }
    fn vol_tvl_exit(&self, regime: Regime) -> (f64, Duration) {
        match regime {
            Regime::S => (self.vol_tvl_exit_s, self.vol_tvl_exit_persistence_s),
            Regime::V1 => (self.vol_tvl_exit_v1, self.vol_tvl_exit_persistence_v1),
            Regime::V2 => (self.vol_tvl_exit_v2, self.vol_tvl_exit_persistence_v2),
        }
    }
}

/// One historical indicator reading, for the persistence-window lookback. Callers
/// assemble this from `indicators_{tf}` rows already on disk; assumed sorted ascending
/// by `at`.
#[derive(Debug, Clone, Copy)]
pub struct HistoryPoint {
    pub at: DateTime<Utc>,
    pub r_org: Option<f64>,
    pub vol_tvl: Option<f64>,
}

pub struct TriggersInput {
    pub r_org: f64,
    pub vol_tvl: f64,
    /// S/V1: 7-day wk/wk volume change (fraction). V2: `vol_24h / avg_vol_72h`.
    pub volume_decay_metric: f64,
    pub v2_is_young: bool,
    /// Ratio of new to old base fee, if a `fee_param_updates` row landed this tick.
    pub fee_jack_multiplier: Option<f64>,
    pub history: Vec<HistoryPoint>,
    /// Weekend persistence windows double (Sat-Sun UTC): weekend volume is structurally
    /// lower and the un-doubled windows produce false exits.
    pub is_weekend_utc: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TriggersOutput {
    pub exit: bool,
}

/// Triggers stage: evaluate every exit condition as a lookback over recorded history,
/// not a stateful accumulator.
pub fn evaluate(
    input: &TriggersInput,
    regime: Regime,
    cfg: &TriggersConfig,
) -> (TriggersOutput, Vec<RationaleItem>) {
    let weekend_factor = if input.is_weekend_utc { 2.0 } else { 1.0 };

    let (r_org_threshold, r_org_persistence) = cfg.r_org_exit(regime);
    let r_org_hours =
        hours_continuously_below(&input.history, input.r_org, r_org_threshold, |p| p.r_org);
    let r_org_persistence_hours = r_org_persistence.as_secs_f64() / 3_600.0 * weekend_factor;

    let (vol_tvl_threshold, vol_tvl_persistence) = cfg.vol_tvl_exit(regime);
    let vol_tvl_hours =
        hours_continuously_below(&input.history, input.vol_tvl, vol_tvl_threshold, |p| {
            p.vol_tvl
        });
    let vol_tvl_persistence_hours = vol_tvl_persistence.as_secs_f64() / 3_600.0 * weekend_factor;

    let volume_decay_threshold = match regime {
        Regime::S => cfg.volume_decay_wk_wk_min,
        Regime::V1 => cfg.volume_decay_v1_min,
        Regime::V2 if input.v2_is_young => cfg.volume_decay_v2_young_min,
        Regime::V2 => cfg.volume_decay_v2_mature_min,
    };

    let mut rationale = vec![
        rationale::check(
            "r_org_exit_persistence_hours",
            r_org_hours,
            Comparator::Lt,
            r_org_persistence_hours,
        ),
        rationale::check(
            "vol_tvl_exit_persistence_hours",
            vol_tvl_hours,
            Comparator::Lt,
            vol_tvl_persistence_hours,
        ),
        rationale::check(
            "volume_decay",
            input.volume_decay_metric,
            Comparator::Ge,
            volume_decay_threshold,
        ),
    ];
    rationale.push(match input.fee_jack_multiplier {
        Some(m) => rationale::check(
            "fee_jack_kill",
            m,
            Comparator::Lt,
            cfg.fee_jack_kill_multiplier,
        ),
        None => rationale::info("fee_jack_kill", 0.0),
    });

    let exit = rationale.iter().any(|r| !r.passed);
    (TriggersOutput { exit }, rationale)
}

/// Hours the metric has been continuously below `threshold`, walking backward from the
/// current reading through `history`. Zero if the current reading is not below the
/// threshold at all.
fn hours_continuously_below(
    history: &[HistoryPoint],
    current: f64,
    threshold: f64,
    value: impl Fn(&HistoryPoint) -> Option<f64>,
) -> f64 {
    if current >= threshold {
        return 0.0;
    }
    let Some(latest) = history.last() else {
        return 0.0;
    };
    let now = latest.at;
    let mut boundary = now;
    for point in history.iter().rev() {
        match value(point) {
            Some(v) if v < threshold => boundary = point.at,
            _ => break,
        }
    }
    now.signed_duration_since(boundary).num_seconds() as f64 / 3_600.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TriggersConfig {
        TriggersConfig::parse_from(["engine"])
    }

    fn pt(hour: i64, r_org: f64) -> HistoryPoint {
        HistoryPoint {
            at: DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
                + chrono::Duration::hours(hour),
            r_org: Some(r_org),
            vol_tvl: Some(10.0),
        }
    }

    #[test]
    fn test_r_org_exit_fires_after_persistence_window_v2() {
        let history: Vec<_> = (0..4).map(|h| pt(h, 1.0)).collect(); // below the V2 exit threshold (2.0) for 3h
        let input = TriggersInput {
            r_org: 1.0,
            vol_tvl: 100.0,
            volume_decay_metric: 1.0,
            v2_is_young: false,
            fee_jack_multiplier: None,
            history,
            is_weekend_utc: false,
        };
        let (out, rationale) = evaluate(&input, Regime::V2, &cfg());
        assert!(out.exit);
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "r_org_exit_persistence_hours" && !r.passed)
        );
    }

    #[test]
    fn test_weekend_doubles_persistence_window() {
        // Below threshold for only 3h -- fires on a weekday (3h persistence) but not on a
        // weekend, where the window doubles to 6h.
        let history: Vec<_> = (0..4).map(|h| pt(h, 1.0)).collect();
        let input_weekday = TriggersInput {
            r_org: 1.0,
            vol_tvl: 100.0,
            volume_decay_metric: 1.0,
            v2_is_young: false,
            fee_jack_multiplier: None,
            history: history.clone(),
            is_weekend_utc: false,
        };
        let input_weekend = TriggersInput {
            is_weekend_utc: true,
            ..input_weekday
        };
        let (out_weekday, _) = evaluate(&input_weekday, Regime::V2, &cfg());
        let (out_weekend, _) = evaluate(&input_weekend, Regime::V2, &cfg());
        assert!(out_weekday.exit);
        assert!(!out_weekend.exit);
    }

    #[test]
    fn test_fee_jack_above_kill_multiplier_exits_instantly() {
        let input = TriggersInput {
            r_org: 10.0,
            vol_tvl: 100.0,
            volume_decay_metric: 1.0,
            v2_is_young: false,
            fee_jack_multiplier: Some(2.5),
            history: vec![pt(0, 10.0)],
            is_weekend_utc: false,
        };
        let (out, rationale) = evaluate(&input, Regime::V1, &cfg());
        assert!(out.exit);
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "fee_jack_kill" && !r.passed)
        );
    }

    #[test]
    fn test_healthy_pool_does_not_exit_but_still_emits_rationale() {
        let input = TriggersInput {
            r_org: 10.0,
            vol_tvl: 100.0,
            volume_decay_metric: 1.0,
            v2_is_young: false,
            fee_jack_multiplier: None,
            history: vec![pt(0, 10.0)],
            is_weekend_utc: false,
        };
        let (out, rationale) = evaluate(&input, Regime::V1, &cfg());
        assert!(!out.exit);
        assert_eq!(rationale.len(), 4);
    }
}
