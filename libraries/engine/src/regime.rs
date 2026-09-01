use std::time::Duration;

use chrono::{DateTime, Utc};
use clap::Parser;
use dlmm_math::{Comparator, RationaleItem};

use crate::indicators::Regime;
use crate::rationale;

#[derive(Parser, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[group(id = "regime")]
pub struct RegimeConfig {
    /// Time a candidate regime must persist before the classifier commits to it. Without
    /// this the classifier oscillates on noise and burns the daily rebalance budget on
    /// churn.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "30m")]
    pub persistence: Duration,

    /// Minimum time between regime flips, regardless of which regime is next. Kill-switch
    /// transitions bypass this.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "2h")]
    pub cooldown: Duration,

    /// S-enter: `sigma_slow` must be below this (fraction, e.g. 0.005 = 0.5%).
    #[arg(long, env, default_value_t = 0.005)]
    pub s_enter_sigma_slow_max: f64,
    /// S-enter: peg deviation must be below this (fraction, e.g. 0.003 = 30 bp).
    #[arg(long, env, default_value_t = 0.003)]
    pub s_enter_dev_peg_max: f64,
    /// S-exit: `sigma_fast` above this leaves S.
    #[arg(long, env, default_value_t = 0.01)]
    pub s_exit_sigma_fast_min: f64,
    /// S-exit: peg deviation above this leaves S.
    #[arg(long, env, default_value_t = 0.005)]
    pub s_exit_dev_peg_min: f64,
    /// V2-enter: `sigma_slow` above this enters V2.
    #[arg(long, env, default_value_t = 0.08)]
    pub v2_enter_sigma_slow_min: f64,
    /// V2-enter: age below this (days) enters V2.
    #[arg(long, env, default_value_t = 30.0)]
    pub v2_enter_age_max_days: f64,
    /// V2-exit: `sigma_slow` must stay below this to leave V2.
    #[arg(long, env, default_value_t = 0.05)]
    pub v2_exit_sigma_slow_max: f64,
    /// V2-exit: age must reach this (days) to leave V2.
    #[arg(long, env, default_value_t = 30.0)]
    pub v2_exit_age_min_days: f64,
}

/// Regime hysteresis state, modelled explicitly so it can be persisted and restored
/// across restarts (rather than living only in memory): `regime` is the committed
/// classification, `pending`/`pending_since` track a candidate that has not yet persisted
/// long enough to be committed, and `last_transition` enforces the cooldown.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegimeState {
    pub regime: Option<Regime>,
    pub since: DateTime<Utc>,
    pub pending: Option<Regime>,
    pub pending_since: Option<DateTime<Utc>>,
    pub last_transition: Option<DateTime<Utc>>,
}

impl RegimeState {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            regime: None,
            since: now,
            pending: None,
            pending_since: None,
            last_transition: None,
        }
    }

    /// Apply the hysteresis rule to this tick's candidate regime and return the single
    /// `RationaleItem` this stage contributes, whether or not it commits.
    /// `kill_switch` bypasses both the persistence and cooldown checks.
    pub fn update(
        &mut self,
        candidate: Option<Regime>,
        now: DateTime<Utc>,
        cfg: &RegimeConfig,
        kill_switch: bool,
    ) -> RationaleItem {
        if kill_switch {
            self.commit(candidate, now);
            return rationale::info("regime_kill_switch", 1.0);
        }

        if candidate == self.regime {
            self.pending = None;
            self.pending_since = None;
            let minutes_stable = now.signed_duration_since(self.since).num_seconds() as f64 / 60.0;
            return rationale::info("regime_stable_minutes", minutes_stable);
        }

        if self.pending != candidate {
            self.pending = candidate;
            self.pending_since = Some(now);
        }
        let pending_since = self.pending_since.unwrap_or(now);
        let elapsed_minutes = now.signed_duration_since(pending_since).num_seconds() as f64 / 60.0;
        let persistence_minutes = cfg.persistence.as_secs_f64() / 60.0;

        if elapsed_minutes < persistence_minutes {
            return rationale::check(
                "regime_persistence_minutes",
                elapsed_minutes,
                Comparator::Ge,
                persistence_minutes,
            );
        }

        let cooldown_minutes = cfg.cooldown.as_secs_f64() / 60.0;
        if let Some(last) = self.last_transition {
            let since_last_minutes = now.signed_duration_since(last).num_seconds() as f64 / 60.0;
            if since_last_minutes < cooldown_minutes {
                return rationale::check(
                    "regime_cooldown_minutes",
                    since_last_minutes,
                    Comparator::Ge,
                    cooldown_minutes,
                );
            }
        }

        self.commit(candidate, now);
        rationale::check(
            "regime_persistence_minutes",
            elapsed_minutes,
            Comparator::Ge,
            persistence_minutes,
        )
    }

    fn commit(&mut self, candidate: Option<Regime>, now: DateTime<Utc>) {
        self.regime = candidate;
        self.since = now;
        self.pending = None;
        self.pending_since = None;
        self.last_transition = Some(now);
    }
}

/// This tick's candidate regime, before hysteresis. Enter/exit conditions are asymmetric
/// per regime (a Schmitt-trigger band) so a pool sitting near a boundary
/// does not relabel on every tick even before the time-based hysteresis in
/// [`RegimeState::update`] is applied; the two mechanisms are independent and both load-
/// bearing.
#[allow(clippy::too_many_arguments)]
pub fn classify_candidate(
    current: Option<Regime>,
    sigma_slow: f64,
    sigma_fast: f64,
    dev_peg: Option<f64>,
    age_days: f64,
    is_pegged_whitelisted: bool,
    is_major: bool,
    cfg: &RegimeConfig,
) -> Option<Regime> {
    let s_enter = is_pegged_whitelisted
        && sigma_slow < cfg.s_enter_sigma_slow_max
        && dev_peg.is_some_and(|d| d < cfg.s_enter_dev_peg_max);
    let v2_enter = sigma_slow > cfg.v2_enter_sigma_slow_min
        || age_days < cfg.v2_enter_age_max_days
        || !is_major;

    match current {
        Some(Regime::S) => {
            let s_exit = sigma_fast > cfg.s_exit_sigma_fast_min
                || dev_peg.is_some_and(|d| d > cfg.s_exit_dev_peg_min);
            if s_exit {
                Some(if v2_enter { Regime::V2 } else { Regime::V1 })
            } else {
                Some(Regime::S)
            }
        }
        Some(Regime::V2) => {
            let v2_exit = sigma_slow < cfg.v2_exit_sigma_slow_max
                && age_days >= cfg.v2_exit_age_min_days
                && is_major;
            if v2_exit {
                Some(if s_enter { Regime::S } else { Regime::V1 })
            } else {
                Some(Regime::V2)
            }
        }
        Some(Regime::V1) | None => {
            if s_enter {
                Some(Regime::S)
            } else if v2_enter {
                Some(Regime::V2)
            } else {
                Some(Regime::V1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RegimeConfig {
        RegimeConfig::parse_from(["engine"])
    }

    fn t(minute_offset: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::minutes(minute_offset)
    }

    #[test]
    fn test_candidate_persisting_29_minutes_does_not_flip() {
        let cfg = cfg();
        let mut state = RegimeState::new(t(0));
        state.regime = Some(Regime::V1);
        state.since = t(0);

        state.update(Some(Regime::V2), t(0), &cfg, false);
        let item = state.update(Some(Regime::V2), t(29), &cfg, false);

        assert_eq!(state.regime, Some(Regime::V1));
        assert!(!item.passed);
        assert_eq!(item.signal, "regime_persistence_minutes");
    }

    #[test]
    fn test_candidate_persisting_31_minutes_flips() {
        let cfg = cfg();
        let mut state = RegimeState::new(t(0));
        state.regime = Some(Regime::V1);
        state.since = t(0);

        state.update(Some(Regime::V2), t(0), &cfg, false);
        let item = state.update(Some(Regime::V2), t(31), &cfg, false);

        assert_eq!(state.regime, Some(Regime::V2));
        assert!(item.passed);
    }

    #[test]
    fn test_flip_within_two_hours_of_last_transition_is_blocked() {
        let cfg = cfg();
        let mut state = RegimeState::new(t(0));
        state.regime = Some(Regime::V1);
        state.since = t(0);
        state.last_transition = Some(t(0)); // a flip happened at t=0

        // Candidate S starts persisting at t=40 and holds past the 30-minute persistence
        // window at t=71, but that is only 71 minutes after the last transition -- inside
        // the 2-hour cooldown.
        state.update(Some(Regime::S), t(40), &cfg, false);
        let item = state.update(Some(Regime::S), t(71), &cfg, false);

        assert_eq!(state.regime, Some(Regime::V1));
        assert!(!item.passed);
        assert_eq!(item.signal, "regime_cooldown_minutes");
    }

    #[test]
    fn test_flip_after_cooldown_elapses_succeeds() {
        let cfg = cfg();
        let mut state = RegimeState::new(t(0));
        state.regime = Some(Regime::V1);
        state.since = t(0);
        state.last_transition = Some(t(0));

        state.update(Some(Regime::S), t(150), &cfg, false);
        let item = state.update(Some(Regime::S), t(181), &cfg, false);

        assert_eq!(state.regime, Some(Regime::S));
        assert!(item.passed);
    }

    #[test]
    fn test_kill_switch_bypasses_persistence_and_cooldown() {
        let cfg = cfg();
        let mut state = RegimeState::new(t(0));
        state.regime = Some(Regime::V1);
        state.since = t(0);
        state.last_transition = Some(t(0));

        let item = state.update(Some(Regime::V2), t(1), &cfg, true);

        assert_eq!(state.regime, Some(Regime::V2));
        assert!(item.passed);
    }

    #[test]
    fn test_stable_regime_still_emits_rationale() {
        let cfg = cfg();
        let mut state = RegimeState::new(t(0));
        state.regime = Some(Regime::V1);
        state.since = t(0);

        let item = state.update(Some(Regime::V1), t(10), &cfg, false);
        assert_eq!(item.signal, "regime_stable_minutes");
        assert!(item.passed);
    }

    #[test]
    fn test_classify_candidate_s_enter() {
        let cfg = cfg();
        let candidate =
            classify_candidate(None, 0.001, 0.001, Some(0.001), 400.0, true, true, &cfg);
        assert_eq!(candidate, Some(Regime::S));
    }

    #[test]
    fn test_classify_candidate_v2_enter_on_young_age() {
        let cfg = cfg();
        let candidate = classify_candidate(None, 0.02, 0.01, None, 3.0, false, true, &cfg);
        assert_eq!(candidate, Some(Regime::V2));
    }

    #[test]
    fn test_classify_candidate_v1_otherwise() {
        let cfg = cfg();
        let candidate = classify_candidate(None, 0.02, 0.01, None, 400.0, false, true, &cfg);
        assert_eq!(candidate, Some(Regime::V1));
    }

    #[test]
    fn test_classify_candidate_s_sticky_until_exit_band() {
        let cfg = cfg();
        // Currently S; sigma_fast rises above the enter band but below the exit band --
        // should remain S because the exit condition has not fired.
        let candidate = classify_candidate(
            Some(Regime::S),
            0.001,
            0.006,
            Some(0.001),
            400.0,
            true,
            true,
            &cfg,
        );
        assert_eq!(candidate, Some(Regime::S));
    }
}
