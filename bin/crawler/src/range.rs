//! Pure range logic: deciding whether one signature falls inside the requested backfill
//! window, and turning a slot span into progress-reporting chunks. Nothing here touches the
//! network or the filesystem, so it is exercised entirely by unit tests below.

/// The slot and/or on-chain-time bounds a backfill run was asked to cover. Either half of
/// either bound may be absent -- an absent lower bound means "as far back as the node still
/// has history for", an absent upper bound means "up to the current head".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RangeSpec {
    pub from_slot: Option<u64>,
    pub to_slot: Option<u64>,
    pub from_time: Option<i64>,
    pub to_time: Option<i64>,
}

impl RangeSpec {
    pub fn is_unbounded(&self) -> bool {
        self.from_slot.is_none()
            && self.to_slot.is_none()
            && self.from_time.is_none()
            && self.to_time.is_none()
    }
}

/// Where one signature sits relative to a `RangeSpec`, from the point of view of a walk that
/// pages backward from the newest signature towards the oldest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// Newer than the upper bound -- skip it and keep paging, more signatures in range may
    /// still follow (this only really happens when resuming with an explicit `to` in the past).
    TooNew,
    Within,
    /// Older than the lower bound. Since a backward walk only gets older from here, seeing
    /// this is the signal to stop paging entirely, not just to skip the one signature.
    TooOld,
}

/// `block_time` is `None` on some RPC responses (very old nodes, or a still-processing slot);
/// when absent, only the slot bounds are checked, matching how `getSignaturesForAddress`
/// itself treats an unknown block time as "not disqualifying".
pub fn classify(slot: u64, block_time: Option<i64>, spec: &RangeSpec) -> Position {
    if let Some(to) = spec.to_slot
        && slot > to
    {
        return Position::TooNew;
    }
    if let Some(bt) = block_time
        && let Some(to) = spec.to_time
        && bt > to
    {
        return Position::TooNew;
    }
    if let Some(from) = spec.from_slot
        && slot < from
    {
        return Position::TooOld;
    }
    if let Some(bt) = block_time
        && let Some(from) = spec.from_time
        && bt < from
    {
        return Position::TooOld;
    }
    Position::Within
}

/// Splits `[from, to)` into `chunk_size`-wide windows, used only to turn "we are at slot S"
/// into a human chunk count for progress logging -- `getSignaturesForAddress` itself has no
/// slot-range parameter, so this never drives an actual RPC call.
pub fn chunk_slot_range(from: u64, to: u64, chunk_size: u64) -> Vec<(u64, u64)> {
    if from >= to || chunk_size == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = from;
    while start < to {
        let end = start.saturating_add(chunk_size).min(to);
        chunks.push((start, end));
        start = end;
    }
    chunks
}

/// Which chunk (0-based) `slot` falls into within `[from, to)`, for progress reporting.
/// Clamps rather than panicking on an out-of-span slot, since a resumed walk's checkpoint
/// slot can legitimately sit right on a boundary.
pub fn chunk_index(slot: u64, from: u64, to: u64, chunk_size: u64) -> u64 {
    if chunk_size == 0 || to <= from {
        return 0;
    }
    let clamped = slot.clamp(from, to.saturating_sub(1));
    (clamped - from) / chunk_size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(from_slot: Option<u64>, to_slot: Option<u64>) -> RangeSpec {
        RangeSpec {
            from_slot,
            to_slot,
            from_time: None,
            to_time: None,
        }
    }

    #[test]
    fn test_unbounded_spec_accepts_everything() {
        let s = RangeSpec::default();
        assert!(s.is_unbounded());
        assert_eq!(classify(0, None, &s), Position::Within);
        assert_eq!(classify(u64::MAX, Some(i64::MAX), &s), Position::Within);
    }

    #[test]
    fn test_slot_below_lower_bound_is_too_old() {
        let s = spec(Some(100), None);
        assert_eq!(classify(99, None, &s), Position::TooOld);
        assert_eq!(classify(100, None, &s), Position::Within);
    }

    #[test]
    fn test_slot_above_upper_bound_is_too_new() {
        let s = spec(None, Some(100));
        assert_eq!(classify(101, None, &s), Position::TooNew);
        assert_eq!(classify(100, None, &s), Position::Within);
    }

    #[test]
    fn test_time_bounds_only_apply_when_block_time_known() {
        let s = RangeSpec {
            from_slot: None,
            to_slot: None,
            from_time: Some(1_000),
            to_time: Some(2_000),
        };
        // No block_time reported: slot-only bounds are absent too, so nothing disqualifies it.
        assert_eq!(classify(1, None, &s), Position::Within);
        assert_eq!(classify(1, Some(999), &s), Position::TooOld);
        assert_eq!(classify(1, Some(2_001), &s), Position::TooNew);
        assert_eq!(classify(1, Some(1_500), &s), Position::Within);
    }

    #[test]
    fn test_slot_and_time_bounds_combine() {
        let s = RangeSpec {
            from_slot: Some(50),
            to_slot: Some(150),
            from_time: Some(1_000),
            to_time: None,
        };
        // Within the slot window but before the time window: still too old.
        assert_eq!(classify(60, Some(500), &s), Position::TooOld);
        assert_eq!(classify(60, Some(1_500), &s), Position::Within);
        assert_eq!(classify(200, Some(1_500), &s), Position::TooNew);
    }

    #[test]
    fn test_chunk_slot_range_even_split() {
        let chunks = chunk_slot_range(0, 100, 25);
        assert_eq!(chunks, vec![(0, 25), (25, 50), (50, 75), (75, 100)]);
    }

    #[test]
    fn test_chunk_slot_range_uneven_last_chunk() {
        let chunks = chunk_slot_range(0, 90, 25);
        assert_eq!(chunks, vec![(0, 25), (25, 50), (50, 75), (75, 90)]);
    }

    #[test]
    fn test_chunk_slot_range_empty_when_inverted_or_equal() {
        assert!(chunk_slot_range(100, 100, 10).is_empty());
        assert!(chunk_slot_range(100, 50, 10).is_empty());
    }

    #[test]
    fn test_chunk_slot_range_rejects_zero_chunk_size() {
        assert!(chunk_slot_range(0, 100, 0).is_empty());
    }

    #[test]
    fn test_chunk_index_picks_correct_bucket() {
        assert_eq!(chunk_index(0, 0, 100, 25), 0);
        assert_eq!(chunk_index(24, 0, 100, 25), 0);
        assert_eq!(chunk_index(25, 0, 100, 25), 1);
        assert_eq!(chunk_index(99, 0, 100, 25), 3);
    }

    #[test]
    fn test_chunk_index_clamps_out_of_span_slot() {
        // A resumed walk's last-seen slot can sit exactly on (or past) `to` once the range
        // has been fully covered; this must not panic on the subtraction.
        assert_eq!(chunk_index(1_000, 0, 100, 25), 3);
    }
}
