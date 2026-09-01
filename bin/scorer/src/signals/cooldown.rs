//! Cooldown decision for signal broadcasts. The `signals` table is the persisted key --
//! (pool_address, timeframe, kind) plus `ts` is exactly the dedup state a cooldown needs, so
//! there is nothing to hold in memory: `storage::queries::last_signal_broadcast` reads the
//! most recent matching row and this module only carries the pure window comparison. This
//! follows `regime_state`/`volatility_state` (see `bin/scorer/src/state.rs`) in surviving a
//! restart, without needing a table of its own.

use chrono::{DateTime, Utc};

pub struct Cooldown {
    window: chrono::Duration,
}

impl Cooldown {
    pub fn new(window: chrono::Duration) -> Self {
        Self { window }
    }

    /// Whether a broadcast is due now, given the most recent one for this key (if any).
    pub fn is_due(&self, last_broadcast: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
        match last_broadcast {
            Some(last) => now.signed_duration_since(last) >= self.window,
            None => true,
        }
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
    fn test_no_prior_broadcast_is_always_due() {
        let cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.is_due(None, t(0)));
    }

    #[test]
    fn test_not_due_inside_the_cooldown_window() {
        let cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(!cooldown.is_due(Some(t(0)), t(30)));
    }

    #[test]
    fn test_due_once_the_window_elapses() {
        let cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.is_due(Some(t(0)), t(60)));
    }

    #[test]
    fn test_not_due_one_minute_before_the_window_elapses() {
        let cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(!cooldown.is_due(Some(t(0)), t(59)));
    }
}
