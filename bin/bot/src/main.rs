use async_trait::async_trait;
use clap::Parser;
use tokio_util::sync::CancellationToken;

use bot::{Config as TelegramConfig, TelegramWorker};

#[derive(Parser, Debug)]
struct Args {
    #[clap(flatten)]
    logging: logger::Config,
    #[clap(flatten)]
    postgres: common::PostgresConfig,
    #[clap(flatten)]
    metrics: metrics::Config,
    #[clap(flatten)]
    telegram: TelegramConfig,
}

// Wraps the metrics HTTP server as a Worker so it can sit in the same JoinSet as the bot
// itself: either one exiting brings the process down, which is the one lifecycle pattern
// every binary in this workspace uses.
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
    let args = Args::parse();
    args.logging.init()?;

    tracing::info!("Starting bot");

    let pool = args.postgres.connect().await?;

    let ct = CancellationToken::new();
    tokio::spawn({
        let ct = ct.clone();
        async move { common::shutdown_signal(ct).await }
    });

    let workers: Vec<Box<dyn common::Worker>> = vec![
        Box::new(TelegramWorker::new(args.telegram, pool)),
        Box::new(MetricsWorker(args.metrics)),
    ];

    common::run_workers(workers, ct).await
}
