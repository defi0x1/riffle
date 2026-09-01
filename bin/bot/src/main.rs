use std::path::PathBuf;

use async_trait::async_trait;
use clap::Parser;
use eyre::WrapErr;
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

    /// Load settings from a YAML file (see config/bot.example.yaml). A flag or environment
    /// variable of the same name still overrides anything set here. Omit this and the
    /// binary behaves exactly as it always has: flags and environment variables only.
    #[arg(long)]
    config: Option<PathBuf>,
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
    let config_path = common::config_flag(std::env::args().skip(1));
    let args: Args = common::load_config_with_env(std::env::args_os(), config_path.as_deref())
        .wrap_err_with(|| "Loading configuration")?;
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
