use std::future::Future;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

// The worker loop shape from plans/12 §1.7. `biased` matters: without it, cancellation can
// lose a race against ready work. Sleep happens *after* the tick, not via `interval.tick()`,
// so a slow tick delays the next one instead of bursting to catch up. A failing tick is
// logged and swallowed here — one bad iteration must never take the whole loop down.
pub async fn tick_loop<F, Fut>(ct: CancellationToken, interval: Duration, mut tick: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = eyre::Result<()>>,
{
    loop {
        tokio::select! {
            biased;
            _ = ct.cancelled() => break,
            _ = async {} => {}
        }

        if let Err(e) = tick().await {
            tracing::error!(error = ?e, "Tick failed");
        }

        tokio::time::sleep(interval).await;
    }
}
