//! Conversions between the engine's in-memory pipeline state and the rows it round-trips
//! through in `regime_state`/`volatility_state`. Kept pure and separate from the I/O calls
//! themselves so the round trip is testable without a database.

use chrono::{DateTime, Utc};
use engine::Regime;
use engine::regime::RegimeState;
use engine::volatility::VolatilityState;
use std::str::FromStr;
use storage::queries::{RegimeStateRow, VolatilityStateRow};
use storage::write::{NewRegimeStateRow, NewVolatilityStateRow};

pub fn regime_state_to_row(
    pool_address: &str,
    venue: i16,
    timeframe: &str,
    state: &RegimeState,
    now: DateTime<Utc>,
) -> NewRegimeStateRow {
    NewRegimeStateRow {
        pool_address: pool_address.to_string(),
        venue,
        timeframe: timeframe.to_string(),
        regime: state.regime.map(|r| r.to_string()),
        since: state.since,
        pending: state.pending.map(|r| r.to_string()),
        pending_since: state.pending_since,
        last_transition: state.last_transition,
        updated_at: now,
    }
}

pub fn regime_state_from_row(row: &RegimeStateRow) -> RegimeState {
    RegimeState {
        regime: row.regime.as_deref().and_then(|s| Regime::from_str(s).ok()),
        since: row.since,
        pending: row
            .pending
            .as_deref()
            .and_then(|s| Regime::from_str(s).ok()),
        pending_since: row.pending_since,
        last_transition: row.last_transition,
    }
}

pub fn volatility_state_to_row(
    pool_address: &str,
    venue: i16,
    timeframe: &str,
    state: &VolatilityState,
    now: DateTime<Utc>,
) -> NewVolatilityStateRow {
    NewVolatilityStateRow {
        pool_address: pool_address.to_string(),
        venue,
        timeframe: timeframe.to_string(),
        sigma_fast_variance: state.sigma_fast_variance,
        sigma_slow_variance: state.sigma_slow_variance,
        first_observed_at: state.first_observed_at,
        updated_at: now,
    }
}

pub fn volatility_state_from_row(row: &VolatilityStateRow) -> VolatilityState {
    VolatilityState {
        sigma_fast_variance: row.sigma_fast_variance,
        sigma_slow_variance: row.sigma_slow_variance,
        first_observed_at: row.first_observed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(minute_offset: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::minutes(minute_offset)
    }

    #[test]
    fn test_regime_state_round_trips_through_storage_row() {
        let original = RegimeState {
            regime: Some(Regime::V2),
            since: t(0),
            pending: Some(Regime::V1),
            pending_since: Some(t(10)),
            last_transition: Some(t(-120)),
        };

        let row = regime_state_to_row("pool1", 0, "1h", &original, t(20));
        // Simulate the row as it would come back from the database: same fields, no cross-
        // process information lost.
        let reloaded = RegimeStateRow {
            regime: row.regime.clone(),
            since: row.since,
            pending: row.pending.clone(),
            pending_since: row.pending_since,
            last_transition: row.last_transition,
        };
        let restored = regime_state_from_row(&reloaded);

        assert_eq!(restored.regime, original.regime);
        assert_eq!(restored.since, original.since);
        assert_eq!(restored.pending, original.pending);
        assert_eq!(restored.pending_since, original.pending_since);
        assert_eq!(restored.last_transition, original.last_transition);
    }

    #[test]
    fn test_regime_state_with_no_committed_regime_round_trips_as_none() {
        let original = RegimeState::new(t(0));
        let row = regime_state_to_row("pool1", 0, "5m", &original, t(0));
        let reloaded = RegimeStateRow {
            regime: row.regime.clone(),
            since: row.since,
            pending: row.pending.clone(),
            pending_since: row.pending_since,
            last_transition: row.last_transition,
        };
        let restored = regime_state_from_row(&reloaded);

        assert_eq!(restored.regime, None);
        assert_eq!(restored.pending, None);
    }

    #[test]
    fn test_volatility_state_round_trips_through_storage_row() {
        let original = VolatilityState {
            sigma_fast_variance: 0.000123,
            sigma_slow_variance: 0.000045,
            first_observed_at: t(-4_320), // 3 days back
        };

        let row = volatility_state_to_row("pool1", 0, "24h", &original, t(0));
        let reloaded = VolatilityStateRow {
            sigma_fast_variance: row.sigma_fast_variance,
            sigma_slow_variance: row.sigma_slow_variance,
            first_observed_at: row.first_observed_at,
        };
        let restored = volatility_state_from_row(&reloaded);

        assert_eq!(restored.sigma_fast_variance, original.sigma_fast_variance);
        assert_eq!(restored.sigma_slow_variance, original.sigma_slow_variance);
        assert_eq!(restored.first_observed_at, original.first_observed_at);
    }
}
