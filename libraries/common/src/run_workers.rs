use eyre::WrapErr;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::Worker;

// Supervision: spawn every worker into one JoinSet holding a clone of the shared token, wait
// for the first to finish for any reason, then cancel the rest and return that outcome. Never
// panics on an unexpected exit — the caller decides what to do with the error.
pub async fn run_workers(workers: Vec<Box<dyn Worker>>, ct: CancellationToken) -> eyre::Result<()> {
    let mut tasks = JoinSet::new();

    for worker in workers {
        let worker_ct = ct.clone();
        tasks.spawn(async move {
            let name = worker.name();
            (name, worker.run(worker_ct).await)
        });
    }

    let result = match tasks.join_next().await {
        Some(Ok((_, Ok(())))) => Ok(()),
        Some(Ok((name, Err(e)))) => Err(e)
            .wrap_err_with(|| "Task failed unexpectedly")
            .inspect_err(|e| tracing::error!(error = ?e, worker = name, "Worker exited")),
        Some(Err(e)) => Err(e)
            .wrap_err_with(|| "Task panicked")
            .inspect_err(|e| tracing::error!(error = ?e, "Worker task panicked")),
        None => Ok(()),
    };

    ct.cancel();
    result
}
