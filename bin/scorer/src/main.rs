use clap::Parser;
use eyre::WrapErr;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use scorer::config::Args;
use scorer::indicators::IndicatorsWorker;
use scorer::paper::PaperPositionWorker;
use scorer::rollup::RollupWorker;
use scorer::signals::SignalsWorker;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    args.logging.init()?;
    tracing::info!(config = %args, "Starting scorer");

    let pool = args
        .postgres
        .connect()
        .await
        .wrap_err_with(|| "Connecting to postgres")?;
    storage::run_migrations(&pool)
        .await
        .wrap_err_with(|| "Running database migrations")?;

    let ct = CancellationToken::new();
    tokio::spawn({
        let ct = ct.clone();
        async move { common::shutdown_signal(ct).await }
    });

    let mut tasks = JoinSet::new();

    if args.metrics.is_enabled() {
        let metrics_cfg = args.metrics.clone();
        let metrics_ct = ct.clone();
        tasks.spawn(async move { metrics_cfg.serve(metrics_ct).await });
    }

    let signal_cooldown =
        chrono::Duration::from_std(args.tick.signal_cooldown).unwrap_or(chrono::Duration::hours(1));

    let workers: Vec<Box<dyn common::Worker>> = vec![
        Box::new(RollupWorker::new(pool.clone(), args.tick.rollup_interval)),
        Box::new(IndicatorsWorker::new(
            pool.clone(),
            args.tick.indicators_interval,
            args.engine.clone(),
            args.pipeline_defaults.clone(),
        )),
        Box::new(SignalsWorker::new(
            pool.clone(),
            args.tick.signals_interval,
            args.engine.clone(),
            signal_cooldown,
        )),
        Box::new(PaperPositionWorker::new(
            pool.clone(),
            args.tick.paper_position_mark_interval,
            args.tick.outcomes_interval,
            args.engine.clone(),
            args.pipeline_defaults.clone(),
        )),
    ];

    let worker_ct = ct.clone();
    tasks.spawn(async move { common::run_workers(workers, worker_ct).await });

    if let Some(first) = tasks.join_next().await {
        let result = match first {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e).wrap_err_with(|| "Task failed unexpectedly"),
            Err(e) => Err(e).wrap_err_with(|| "Task panicked"),
        };
        ct.cancel();
        return result;
    }

    Ok(())
}
