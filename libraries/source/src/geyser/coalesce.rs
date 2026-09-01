use std::collections::HashMap;
use std::hash::Hash;

/// Buffers the latest value per key, discarding anything older than what is already
/// buffered. Geyser makes no ordering guarantee, so a hot account can deliver updates out
/// of sequence; this is what keeps a late-arriving stale one from clobbering a newer value.
pub struct SlotCoalescer<K, V> {
    buffer: HashMap<K, (u64, V)>,
}

impl<K: Eq + Hash + Clone, V> Default for SlotCoalescer<K, V> {
    fn default() -> Self {
        Self {
            buffer: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, V> SlotCoalescer<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn offer(&mut self, key: K, slot: u64, value: V) {
        if let Some((buffered_slot, _)) = self.buffer.get(&key)
            && *buffered_slot > slot
        {
            return;
        }
        self.buffer.insert(key, (slot, value));
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn snapshot(&self) -> Vec<V>
    where
        V: Clone,
    {
        self.buffer.values().map(|(_, v)| v.clone()).collect()
    }

    // Called only once a flush's downstream send has actually succeeded. Clearing eagerly
    // would drop data that never made it out on a failed send; leaving it buffered makes
    // the next flush attempt a retry instead of a loss.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_older_slot_does_not_overwrite_newer() {
        let mut c = SlotCoalescer::new();
        c.offer("pool", 100, "new");
        c.offer("pool", 90, "stale");
        assert_eq!(c.snapshot(), vec!["new"]);
    }

    #[test]
    fn test_newer_slot_overwrites_older() {
        let mut c = SlotCoalescer::new();
        c.offer("pool", 90, "old");
        c.offer("pool", 100, "new");
        assert_eq!(c.snapshot(), vec!["new"]);
    }

    #[test]
    fn test_equal_slot_replaces_value() {
        // Not older, so not discarded -- most likely a duplicate delivery of the same
        // update, harmless either way.
        let mut c = SlotCoalescer::new();
        c.offer("pool", 100, "first");
        c.offer("pool", 100, "second");
        assert_eq!(c.snapshot(), vec!["second"]);
    }

    #[test]
    fn test_distinct_keys_buffer_independently() {
        let mut c = SlotCoalescer::new();
        c.offer("a", 1, "a1");
        c.offer("b", 1, "b1");
        assert_eq!(c.len(), 2);
        let mut snap = c.snapshot();
        snap.sort_unstable();
        assert_eq!(snap, vec!["a1", "b1"]);
    }

    #[test]
    fn test_snapshot_does_not_clear_buffer() {
        let mut c = SlotCoalescer::new();
        c.offer("pool", 1, "v");
        let _ = c.snapshot();
        assert!(!c.is_empty());
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn test_clear_empties_buffer() {
        let mut c: SlotCoalescer<&str, &str> = SlotCoalescer::new();
        c.offer("pool", 1, "v");
        c.clear();
        assert!(c.is_empty());
        assert!(c.snapshot().is_empty());
    }

    #[test]
    fn test_failed_send_can_retry_because_buffer_was_not_cleared() {
        let mut c = SlotCoalescer::new();
        c.offer("pool", 1, "v1");
        // simulate a failed downstream send: snapshot taken, but clear() intentionally
        // skipped, exactly as the flush loop does on a send error
        let first_attempt = c.snapshot();
        assert_eq!(first_attempt, vec!["v1"]);
        // a newer update can still merge in while the failed batch awaits retry
        c.offer("pool", 2, "v2");
        let retry = c.snapshot();
        assert_eq!(retry, vec!["v2"]);
    }
}
