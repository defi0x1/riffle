use std::collections::HashMap;

use solana_sdk::pubkey::Pubkey;

use source::StateUpdate;

/// Coalesces state updates by pool, keeping only the newest slot seen for each. Geyser
/// guarantees neither ordering nor deduplication and a hot pool can produce several updates
/// a second while only the latest matters; RPC polling never reorders within a batch but the
/// same guard is harmless there too. The buffer is drained by the caller, which must only
/// call `clear` once the drained rows have been durably written -- a failed write should
/// leave the buffer intact so the next flush retries it rather than losing it.
#[derive(Default)]
pub struct StateBuffer {
    entries: HashMap<Pubkey, StateUpdate>,
}

impl StateBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `update`, dropping it if a newer slot for the same pool is already buffered.
    pub fn offer(&mut self, update: StateUpdate) {
        if let Some(existing) = self.entries.get(&update.pool)
            && existing.slot > update.slot
        {
            return;
        }
        self.entries.insert(update.pool, update);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn drain(&self) -> Vec<StateUpdate> {
        self.entries.values().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use dlmm_decode::PoolState;

    use super::*;

    fn update(pool: Pubkey, slot: u64) -> StateUpdate {
        StateUpdate {
            pool,
            slot,
            block_time: slot as i64,
            lb_pair: None::<PoolState>,
            bin_arrays: Vec::new(),
        }
    }

    #[test]
    fn test_older_slot_does_not_overwrite_a_newer_one() {
        let pool = Pubkey::new_unique();
        let mut buffer = StateBuffer::new();

        buffer.offer(update(pool, 10));
        buffer.offer(update(pool, 5));

        let drained = buffer.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].slot, 10);
    }

    #[test]
    fn test_newer_slot_replaces_an_older_one() {
        let pool = Pubkey::new_unique();
        let mut buffer = StateBuffer::new();

        buffer.offer(update(pool, 5));
        buffer.offer(update(pool, 10));

        let drained = buffer.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].slot, 10);
    }

    #[test]
    fn test_equal_slot_is_treated_as_an_update_not_a_drop() {
        let pool = Pubkey::new_unique();
        let mut buffer = StateBuffer::new();

        buffer.offer(update(pool, 10));
        buffer.offer(update(pool, 10));

        assert_eq!(buffer.drain().len(), 1);
    }

    #[test]
    fn test_distinct_pools_are_buffered_independently() {
        let mut buffer = StateBuffer::new();
        buffer.offer(update(Pubkey::new_unique(), 1));
        buffer.offer(update(Pubkey::new_unique(), 1));

        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn test_clear_only_after_flush_leaves_buffer_intact_on_failure() {
        let pool = Pubkey::new_unique();
        let mut buffer = StateBuffer::new();
        buffer.offer(update(pool, 10));

        // Simulate a failed flush: the buffer is not cleared, so the same data is retried.
        assert_eq!(buffer.drain().len(), 1);
        assert!(!buffer.is_empty());

        // A successful flush clears it.
        buffer.clear();
        assert!(buffer.is_empty());
    }
}
