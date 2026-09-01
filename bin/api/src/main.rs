use std::sync::Arc;

use async_trait::async_trait;
use eyre::WrapErr;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use tokio_util::sync::CancellationToken;

use api::{ApiWorker, Args, AppState};

// Wraps the metrics HTTP server as a Worker, matching bin/bot's own MetricsWorker exactly --
// either it or the API server exiting brings the process down, the one lifecycle pattern every
// binary in this workspace uses.
struct MetricsWorker(metrics::Config);

#[async_trait]
impl common::Worker for MetricsWorker {
    fn name(&self) -> &'static str {
        "metrics"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        if !self.0.is_enabled() {
            ct.cancelled().await;
            return Ok(());
        }
        self.0.serve(ct).await
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let config_path = common::config_flag(std::env::args().skip(1));
    let args: Args = common::load_config_with_env(std::env::args_os(), config_path.as_deref())
        .wrap_err_with(|| "Loading configuration")?;
    args.logging.init()?;

    tracing::info!("Starting api");

    let db = args.postgres.connect().await?;
    let rpc = Arc::new(RpcClient::new(args.rpc_url.clone()));
    let port = args.port;
    let metrics_config = args.metrics.clone();

    let state = AppState {
        db,
        rpc,
        config: Arc::new(args),
    };

    let ct = CancellationToken::new();
    tokio::spawn({
        let ct = ct.clone();
        async move { common::shutdown_signal(ct).await }
    });

    let workers: Vec<Box<dyn common::Worker>> = vec![
        Box::new(ApiWorker::new(state, port)),
        Box::new(MetricsWorker(metrics_config)),
    ];

    common::run_workers(workers, ct).await
}
