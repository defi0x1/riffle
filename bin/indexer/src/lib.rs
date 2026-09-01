pub mod config;
pub mod convert;
pub mod workers;

use std::sync::Arc;

use eyre::WrapErr;
use tokio_util::sync::CancellationToken;

use common::{Worker, run_workers};
use source::geyser::GeyserSource;
use source::rpc::RpcSource;
use source::{Backend, Config as SourceConfig, Source};

use config::Args;
use workers::{
    DiscoveryWorker, EventWorker, HealthWorker, MetricsWorker, Progress, StateWorker, TierWorker,
};

fn build_source(config: &SourceConfig) -> eyre::Result<Arc<dyn Source>> {
    let source: Arc<dyn Source> = match config.backend {
        Backend::Rpc => Arc::new(RpcSource::new(config.rpc.clone())?),
        Backend::Geyser => Arc::new(GeyserSource::new(config.geyser.clone())?),
    };
    Ok(source)
}

fn backend_label(backend: Backend) -> &'static str {
    match backend {
        Backend::Rpc => "rpc",
        Backend::Geyser => "geyser",
    }
}

pub async fn run(args: Args, ct: CancellationToken) -> eyre::Result<()> {
    tracing::info!("{}", args);

    let pool = args
        .postgres
        .connect()
        .await
        .wrap_err_with(|| "Connecting to Postgres")?;
    storage::run_migrations(&pool)
        .await
        .wrap_err_with(|| "Running database migrations")?;

    let source = build_source(&args.source)?;
    let progress = Arc::new(Progress::default());

    let workers: Vec<Box<dyn Worker>> = vec![
        Box::new(DiscoveryWorker::new(
            pool.clone(),
            source.clone(),
            args.discovery_interval,
            args.discovery_batch_size,
        )),
        Box::new(StateWorker::new(
            pool.clone(),
            source.clone(),
            progress.clone(),
            args.tier.promotion_interval,
            args.state_flush_interval,
            args.state_flush_batch_size,
        )),
        Box::new(EventWorker::new(
            pool.clone(),
            source.clone(),
            progress.clone(),
            args.event_flush_interval,
            args.event_flush_batch_size,
        )),
        Box::new(TierWorker::new(
            pool.clone(),
            args.tier.promotion_interval,
            args.tier.max_watched,
            args.tier.exploration_slice,
            args.tier.demotion_margin,
        )),
        Box::new(HealthWorker::new(
            pool.clone(),
            progress.clone(),
            args.health_interval,
            args.health_freshness_threshold,
            backend_label(args.source.backend).to_string(),
        )),
        Box::new(MetricsWorker::new(args.metrics.clone())),
    ];

    run_workers(workers, ct).await
}
