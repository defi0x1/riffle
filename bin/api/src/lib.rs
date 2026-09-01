//! `api` is the HTTP service the Telegram Mini App talks to (see `miniapp/README.md`'s "HTTP
//! contract expected from the backend"). It is keyless by construction: no type anywhere in this
//! crate can hold or derive signing material, and no function here ever produces a signature.
//! It builds unsigned transactions via `dlmm_tx`, records their lifecycle in `transaction_intents`
//! (all SQL for that lives in `libraries/storage`, never here), relays already-signed
//! transactions to RPC, and serves balances, positions and profit reads.

pub mod config;
mod dto;
mod error;
mod http_metrics;
mod risk;
mod routes;
mod rpc_ext;
mod state;
mod telegram_auth;
#[cfg(all(test, feature = "db-tests"))]
mod test_support;
mod tx_build;
mod wallet_resolve;

pub use config::Args;
pub use state::AppState;

use async_trait::async_trait;
use eyre::WrapErr;
use tokio_util::sync::CancellationToken;

/// Wraps the HTTP server as a `common::Worker`, the same lifecycle pattern every binary in this
/// workspace uses (see bin/bot's `TelegramWorker`, bin/indexer's top-level task).
pub struct ApiWorker {
    state: AppState,
    port: u16,
}

impl ApiWorker {
    pub fn new(state: AppState, port: u16) -> Self {
        Self { state, port }
    }
}

#[async_trait]
impl common::Worker for ApiWorker {
    fn name(&self) -> &'static str {
        "api"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        let app = routes::router(self.state.clone());
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .wrap_err_with(|| format!("Binding API listener on {addr}"))?;

        tracing::info!(%addr, "API server listening");

        axum::serve(listener, app)
            .with_graceful_shutdown(async move { ct.cancelled().await })
            .await
            .wrap_err_with(|| "Running API server")?;

        Ok(())
    }
}
