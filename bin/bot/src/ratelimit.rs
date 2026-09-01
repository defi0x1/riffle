use std::time::{Duration, Instant};

// Telegram enforces roughly 1 message/second per chat and ~30/second across the whole bot.
// With a short allow-list the per-chat gap is the binding constraint in practice -- it is
// what a paginated `/why` dump would otherwise blow through -- so that is what this tracks;
// the small allow-list keeps the aggregate well under the global cap by construction.
//
// Pure so the spacing math is testable without sleeping in a test.
pub fn wait_for(last_sent: Option<Instant>, now: Instant, min_gap: Duration) -> Duration {
    match last_sent {
        None => Duration::ZERO,
        Some(last) => min_gap.saturating_sub(now.saturating_duration_since(last)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_wait_on_first_send() {
        let now = Instant::now();
        assert_eq!(
            wait_for(None, now, Duration::from_millis(1050)),
            Duration::ZERO
        );
    }

    #[test]
    fn test_full_gap_required_immediately_after() {
        let now = Instant::now();
        let gap = Duration::from_millis(1050);
        assert_eq!(wait_for(Some(now), now, gap), gap);
    }

    #[test]
    fn test_partial_wait_after_partial_elapsed() {
        let gap = Duration::from_millis(1000);
        let last = Instant::now();
        let now = last + Duration::from_millis(400);
        assert_eq!(wait_for(Some(last), now, gap), Duration::from_millis(600));
    }

    #[test]
    fn test_no_wait_once_gap_has_elapsed() {
        let gap = Duration::from_millis(1000);
        let last = Instant::now();
        let now = last + Duration::from_millis(1500);
        assert_eq!(wait_for(Some(last), now, gap), Duration::ZERO);
    }
}
