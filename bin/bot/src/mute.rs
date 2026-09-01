use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

// Process-local mute registry. There is no persisted store for this: signal suppression has
// no backing table or query in storage, and this binary writes no SQL of its own, so a mute
// resets on restart. Acceptable for a single operator; worth a real table once there is an
// alert-broadcasting subsystem for it to actually gate.
#[derive(Default)]
pub struct MuteStore {
    until: Mutex<HashMap<String, DateTime<Utc>>>,
}

// A poisoned lock still holds a perfectly usable map; recovering it beats panicking a
// message handler over an unrelated task's bug.
fn lock(
    mutex: &Mutex<HashMap<String, DateTime<Utc>>>,
) -> std::sync::MutexGuard<'_, HashMap<String, DateTime<Utc>>> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl MuteStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mute(&self, pool_address: String, until: DateTime<Utc>) {
        lock(&self.until).insert(pool_address, until);
    }

    pub fn is_muted(&self, pool_address: &str) -> bool {
        lock(&self.until)
            .get(pool_address)
            .is_some_and(|until| *until > Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_muted_pool_is_muted_before_expiry() {
        let store = MuteStore::new();
        store.mute("pool1".to_string(), Utc::now() + chrono::Duration::hours(1));
        assert!(store.is_muted("pool1"));
    }

    #[test]
    fn test_expired_mute_is_not_muted() {
        let store = MuteStore::new();
        store.mute(
            "pool1".to_string(),
            Utc::now() - chrono::Duration::seconds(1),
        );
        assert!(!store.is_muted("pool1"));
    }

    #[test]
    fn test_unmuted_pool_is_not_muted() {
        let store = MuteStore::new();
        assert!(!store.is_muted("pool1"));
    }

    #[test]
    fn test_remuting_overwrites_previous_expiry() {
        let store = MuteStore::new();
        store.mute(
            "pool1".to_string(),
            Utc::now() - chrono::Duration::seconds(1),
        );
        store.mute("pool1".to_string(), Utc::now() + chrono::Duration::hours(1));
        assert!(store.is_muted("pool1"));
    }
}
