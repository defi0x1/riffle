mod registry;
pub use registry::*;

mod ingest;
pub use ingest::*;

use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use clap::Parser;
use eyre::WrapErr;
use prometheus::Encoder;
use tokio_util::sync::CancellationToken;

#[derive(Parser, Debug, Clone)]
#[group(id = "metrics")]
pub struct Config {
    /// Disable the Prometheus /metrics HTTP server entirely.
    #[arg(long, env)]
    pub disable_metrics_server: bool,

    /// Port the /metrics endpoint listens on.
    #[arg(long, env, default_value_t = 9101)]
    pub metrics_port: u16,
}

impl Config {
    pub fn is_enabled(&self) -> bool {
        !self.disable_metrics_server
    }

    pub async fn serve(&self, ct: CancellationToken) -> eyre::Result<()> {
        let addr = format!("0.0.0.0:{}", self.metrics_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .wrap_err_with(|| "Binding metrics listener")?;

        tracing::info!("Metrics server listening on {}", addr);

        let app = Router::new().route("/metrics", get(handler));
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { ct.cancelled().await })
            .await
            .wrap_err_with(|| "Running metrics server")?;

        Ok(())
    }
}

async fn handler() -> impl IntoResponse {
    let encoder = prometheus::TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buf = Vec::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buf) {
        tracing::error!(error = ?e, "Failed to encode metrics");
        return (StatusCode::INTERNAL_SERVER_ERROR, String::new());
    }
    (StatusCode::OK, String::from_utf8_lossy(&buf).into_owned())
}
