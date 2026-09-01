//! Request pacing and retry backoff for the RPC walk. A public endpoint will 429 well before
//! it hard-fails, so both halves are configurable from the CLI rather than baked in: how far
//! apart requests are spaced going out, and how long a failed one waits before its retry.

use std::future::Future;
use std::time::Duration;

use clap::Parser;
use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Parser, Debug, Clone)]
#[group(id = "pacing")]
pub struct PacingConfig {
    /// Outbound RPC calls in flight at once.
    #[arg(long, env, default_value_t = 4)]
    pub max_concurrent_rpc: usize,

    /// Minimum spacing enforced between the starts of two RPC calls, independent of retries.
    /// This is the primary throttle against a provider's rate limit; concurrency alone does
    /// not bound request rate once individual calls are fast.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "150ms")]
    pub min_request_interval: Duration,

    /// Retry attempts for a failed call before the walk gives up on that pool.
    #[arg(long, env, default_value_t = 6)]
    pub max_retries: usize,

    /// Delay before the first retry. Doubles on each subsequent attempt up to `backoff_max`.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "500ms")]
    pub backoff_base: Duration,

    /// Ceiling on the retry delay, reached once doubling from `backoff_base` would exceed it.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "20s")]
    pub backoff_max: Duration,
}

/// The delay before retry attempt `attempt` (0-indexed: 0 is the first retry after the
/// original call failed). Doubles each attempt and saturates at `max` rather than overflowing
/// once `attempt` gets large -- a stuck pool retrying for minutes must not wrap the shift.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let shift = attempt.min(31);
    base.checked_mul(1u32 << shift).unwrap_or(max).min(max)
}

/// Enforces a minimum spacing between call starts. A `Mutex<Option<Instant>>` rather than a
/// token-bucket: the crawler's whole point is to be gentle, so a strict "never start sooner
/// than `interval` after the last start" is the intended behaviour, not a burst allowance.
pub struct Pacer {
    interval: Duration,
    last_start: Mutex<Option<Instant>>,
}

impl Pacer {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_start: Mutex::new(None),
        }
    }

    pub async fn wait(&self) {
        let mut guard = self.last_start.lock().await;
        if let Some(last) = *guard {
            let elapsed = last.elapsed();
            if elapsed < self.interval {
                tokio::time::sleep(self.interval - elapsed).await;
            }
        }
        *guard = Some(Instant::now());
    }
}

/// Retries `op` with exponential backoff, logging each retry at warn level so an operator
/// watching the crawler run can tell a 429 storm from a hang. Gives up and returns the last
/// error once `max_retries` is exhausted.
pub async fn retry_with_backoff<T, E, F, Fut>(
    label: &str,
    max_retries: usize,
    base: Duration,
    max: Duration,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let mut attempt = 0u32;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if attempt as usize >= max_retries {
                    return Err(e);
                }
                let delay = backoff_delay(attempt, base, max);
                tracing::warn!(call = label, attempt, delay = ?delay, error = ?e, "Retrying RPC call");
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_delay_doubles_each_attempt() {
        let base = Duration::from_millis(500);
        let max = Duration::from_secs(60);
        assert_eq!(backoff_delay(0, base, max), Duration::from_millis(500));
        assert_eq!(backoff_delay(1, base, max), Duration::from_millis(1_000));
        assert_eq!(backoff_delay(2, base, max), Duration::from_millis(2_000));
        assert_eq!(backoff_delay(3, base, max), Duration::from_millis(4_000));
    }

    #[test]
    fn test_backoff_delay_saturates_at_max() {
        let base = Duration::from_millis(500);
        let max = Duration::from_secs(20);
        assert_eq!(backoff_delay(10, base, max), max);
        assert_eq!(backoff_delay(1_000, base, max), max);
    }

    #[test]
    fn test_backoff_delay_never_overflows() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(3600);
        // Would overflow a naive `1 << attempt` multiply without the shift clamp.
        assert_eq!(backoff_delay(u32::MAX, base, max), max);
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_with_backoff_stops_after_max_retries() {
        let mut calls = 0u32;
        let result: Result<(), &str> = retry_with_backoff(
            "test",
            2,
            Duration::from_millis(1),
            Duration::from_millis(10),
            || {
                calls += 1;
                async { Err("boom") }
            },
        )
        .await;
        assert_eq!(result, Err("boom"));
        // Original attempt plus two retries.
        assert_eq!(calls, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_with_backoff_returns_first_success() {
        let mut calls = 0u32;
        let result: Result<u32, &str> = retry_with_backoff(
            "test",
            5,
            Duration::from_millis(1),
            Duration::from_millis(10),
            || {
                calls += 1;
                async move { if calls < 3 { Err("not yet") } else { Ok(calls) } }
            },
        )
        .await;
        assert_eq!(result, Ok(3));
    }

    #[tokio::test(start_paused = true)]
    async fn test_pacer_enforces_minimum_spacing() {
        let pacer = Pacer::new(Duration::from_millis(50));
        let start = Instant::now();
        pacer.wait().await;
        pacer.wait().await;
        // The second wait must not return before the interval has elapsed.
        assert!(start.elapsed() >= Duration::from_millis(50));
    }
}
