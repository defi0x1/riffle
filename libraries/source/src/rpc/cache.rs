use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use solana_sdk::pubkey::Pubkey;

// A poisoned mutex still holds a perfectly usable map; recovering it is preferable to a
// panic in a cache that only ever affects performance, never correctness.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Remembers that a lookup came back empty, for a short TTL, so a pool that keeps getting
/// asked about (delisted, or not yet indexed upstream) does not force a full re-fetch of
/// the whole universe on every call.
pub struct NegativeCache {
    ttl: Duration,
    misses: Mutex<HashMap<Pubkey, Instant>>,
}

impl NegativeCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            misses: Mutex::new(HashMap::new()),
        }
    }

    pub fn record_miss(&self, pool: Pubkey) {
        lock(&self.misses).insert(pool, Instant::now());
    }

    pub fn is_miss(&self, pool: &Pubkey) -> bool {
        lock(&self.misses)
            .get(pool)
            .is_some_and(|seen_at| seen_at.elapsed() < self.ttl)
    }
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use super::*;

    #[test]
    fn test_unseen_pool_is_not_a_miss() {
        let cache = NegativeCache::new(Duration::from_secs(1));
        assert!(!cache.is_miss(&Pubkey::new_unique()));
    }

    #[test]
    fn test_recorded_miss_is_reported_within_ttl() {
        let cache = NegativeCache::new(Duration::from_millis(50));
        let pool = Pubkey::new_unique();
        cache.record_miss(pool);
        assert!(cache.is_miss(&pool));
    }

    #[test]
    fn test_miss_expires_after_ttl() {
        let cache = NegativeCache::new(Duration::from_millis(20));
        let pool = Pubkey::new_unique();
        cache.record_miss(pool);
        sleep(Duration::from_millis(45));
        assert!(!cache.is_miss(&pool));
    }
}
