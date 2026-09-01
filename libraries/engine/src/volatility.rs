use chrono::{DateTime, Utc};
use dlmm_math::{Comparator, RationaleItem};

use crate::rationale;

/// A single OHLC bar, the volatility stage's raw input (our own 5-minute OHLC, or an
/// external exchange's 1-minute klines for majors).
#[derive(Debug, Clone, Copy)]
pub struct OhlcBar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// EWMA state for `sigma_fast`/`sigma_slow`, carried across ticks. Persisted and restored
/// per pool per timeframe the same way `RegimeState` is — it has no meaning recomputed
/// from scratch every tick.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct VolatilityState {
    pub sigma_fast_variance: f64,
    pub sigma_slow_variance: f64,
    /// When this pool's volatility series started. Drives the cold-start rule: `sigma_slow`
    /// needs a few days of bars before it means anything.
    pub first_observed_at: DateTime<Utc>,
}

impl VolatilityState {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            sigma_fast_variance: 0.0,
            sigma_slow_variance: 0.0,
            first_observed_at: now,
        }
    }

    /// Fold one new bar into both EWMAs.
    pub fn observe(&mut self, bar: OhlcBar) {
        let sq = dlmm_math::garman_klass_variance(bar.open, bar.high, bar.low, bar.close);
        self.sigma_fast_variance =
            dlmm_math::ewma_update(self.sigma_fast_variance, sq, dlmm_math::LAMBDA_FAST);
        self.sigma_slow_variance =
            dlmm_math::ewma_update(self.sigma_slow_variance, sq, dlmm_math::LAMBDA_SLOW);
    }

    fn history_days(&self, now: DateTime<Utc>) -> f64 {
        now.signed_duration_since(self.first_observed_at)
            .num_seconds() as f64
            / 86_400.0
    }
}

/// Cold-start floor: below this, `sigma_slow` does not mean anything yet.
pub const MIN_HISTORY_DAYS: f64 = 3.0;

#[derive(Debug, Clone, Copy)]
pub struct VolatilityOutput {
    pub sigma_gk: f64,
    pub sigma_fast: f64,
    pub sigma_slow: f64,
    pub sigma_d: f64,
    pub sigma_d_bps: f64,
    pub sigma_jump: f64,
    /// Whether the pool has enough history to be ranked at all (`MIN_HISTORY_DAYS`).
    pub sufficient_history: bool,
}

/// Volatility stage: `latest_bar` is the bar closing this bucket, folded into `state`
/// before reading it back out; `autocorrelations` and `log_returns_24h` are precomputed
/// by the caller from the same OHLC series.
pub fn evaluate(
    state: &mut VolatilityState,
    latest_bar: OhlcBar,
    autocorrelations: &[f64],
    log_returns_24h: &[f64],
    decay_window_secs: f64,
    now: DateTime<Utc>,
) -> (VolatilityOutput, Vec<RationaleItem>) {
    let sigma_gk = dlmm_math::garman_klass_variance(
        latest_bar.open,
        latest_bar.high,
        latest_bar.low,
        latest_bar.close,
    );
    state.observe(latest_bar);

    let sigma_fast = state.sigma_fast_variance.max(0.0).sqrt();
    let sigma_slow = state.sigma_slow_variance.max(0.0).sqrt();
    let sigma_d = dlmm_math::daily_vol(state.sigma_fast_variance.max(0.0), autocorrelations);
    let sigma_d_bps = dlmm_math::decay_window_vol_bps(sigma_fast, decay_window_secs);
    let sigma_jump = dlmm_math::jump_share(log_returns_24h);

    let history_days = state.history_days(now);
    let sufficient_history = history_days >= MIN_HISTORY_DAYS;

    let rationale = vec![
        rationale::check(
            "sufficient_volatility_history_days",
            history_days,
            Comparator::Ge,
            MIN_HISTORY_DAYS,
        ),
        rationale::info("sigma_gk", sigma_gk),
        rationale::info("sigma_fast", sigma_fast),
        rationale::info("sigma_slow", sigma_slow),
        rationale::info("sigma_d", sigma_d),
        rationale::info("sigma_jump", sigma_jump),
    ];

    (
        VolatilityOutput {
            sigma_gk,
            sigma_fast,
            sigma_slow,
            sigma_d,
            sigma_d_bps,
            sigma_jump,
            sufficient_history,
        },
        rationale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cold_start_flags_insufficient_history() {
        let now = DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = VolatilityState::new(now - chrono::Duration::hours(12));
        let bar = OhlcBar {
            open: 1.0,
            high: 1.01,
            low: 0.99,
            close: 1.0,
        };
        let (out, rationale) = evaluate(&mut state, bar, &[], &[], 600.0, now);
        assert!(!out.sufficient_history);
        assert!(
            rationale
                .iter()
                .any(|r| r.signal == "sufficient_volatility_history_days" && !r.passed)
        );
    }

    #[test]
    fn test_three_days_of_history_is_sufficient() {
        let now = DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = VolatilityState::new(now - chrono::Duration::days(4));
        let bar = OhlcBar {
            open: 1.0,
            high: 1.01,
            low: 0.99,
            close: 1.0,
        };
        let (out, _) = evaluate(&mut state, bar, &[], &[], 600.0, now);
        assert!(out.sufficient_history);
    }
}
