//! In-memory dedup/cooldown for signal broadcasts. Deliberately not database-backed:
//! `engine::regime::RegimeState` is the piece whose hysteresis clock must survive a restart
//! (it changes the pipeline's own decisions), while a cooldown miss after a restart only
//! means one signal is re-announced a little early -- safer than silently dropping one the
//! operator needed to see. State lives for the life of the worker.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::classify::SignalKind;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SignalKey {
    pool_address: String,
    timeframe: String,
    kind: &'static str,
}

pub struct Cooldown {
    window: chrono::Duration,
    last_broadcast: HashMap<SignalKey, DateTime<Utc>>,
}

impl Cooldown {
    pub fn new(window: chrono::Duration) -> Self {
        Self {
            window,
            last_broadcast: HashMap::new(),
        }
    }

    /// Whether (pool, timeframe, kind) should be broadcast now. Records `now` as the new
    /// last-broadcast time whenever it returns `true`, so a persistent condition fires once
    /// and then stays quiet until the cooldown elapses, rather than every tick.
    pub fn should_broadcast(
        &mut self,
        pool_address: &str,
        timeframe: &str,
        kind: SignalKind,
        now: DateTime<Utc>,
    ) -> bool {
        let key = SignalKey {
            pool_address: pool_address.to_string(),
            timeframe: timeframe.to_string(),
            kind: kind.as_str(),
        };

        let due = match self.last_broadcast.get(&key) {
            Some(last) => now.signed_duration_since(*last) >= self.window,
            None => true,
        };
        if due {
            self.last_broadcast.insert(key, now);
        }
        due
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
    fn test_first_occurrence_always_broadcasts() {
        let mut cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(0)));
    }

    #[test]
    fn test_persistent_condition_does_not_rebroadcast_every_tick() {
        let mut cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(0)));
        // Same condition, still firing 5 and 30 minutes later -- both inside the 1h window.
        assert!(!cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(5)));
        assert!(!cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(30)));
    }

    #[test]
    fn test_rebroadcasts_once_the_cooldown_elapses() {
        let mut cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(0)));
        assert!(!cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(59)));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(60)));
    }

    #[test]
    fn test_different_kind_has_an_independent_cooldown() {
        let mut cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(0)));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Degrading, t(1)));
    }

    #[test]
    fn test_different_pool_has_an_independent_cooldown() {
        let mut cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(0)));
        assert!(cooldown.should_broadcast("pool2", "1h", SignalKind::Potential, t(1)));
    }

    #[test]
    fn test_different_timeframe_has_an_independent_cooldown() {
        let mut cooldown = Cooldown::new(chrono::Duration::hours(1));
        assert!(cooldown.should_broadcast("pool1", "1h", SignalKind::Potential, t(0)));
        assert!(cooldown.should_broadcast("pool1", "4h", SignalKind::Potential, t(1)));
    }
}
