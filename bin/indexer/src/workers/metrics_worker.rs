use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use common::Worker;

/// Thin adapter so the `/metrics` HTTP server can be spawned through the same
/// `common::run_workers` supervision as every ingestion worker.
pub struct MetricsWorker {
    config: metrics::Config,
}

impl MetricsWorker {
    pub fn new(config: metrics::Config) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Worker for MetricsWorker {
    fn name(&self) -> &'static str {
        "metrics"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        if !self.config.is_enabled() {
            ct.cancelled().await;
            return Ok(());
        }
        self.config.serve(ct).await
    }
}
