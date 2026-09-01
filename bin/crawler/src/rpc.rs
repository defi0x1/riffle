//! The paced, retrying RPC surface the walk runs over. Mirrors `source::rpc::state`'s shape
//! (a semaphore bounding concurrency, a wrapped call that retries on failure) rather than
//! introducing a second client abstraction, but adds the pacing gap `state.rs` does not need
//! -- that poller is driven by its own fixed tick interval already, while the crawler fires
//! requests back-to-back and has to impose spacing itself.

use std::sync::Arc;

use eyre::WrapErr;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use solana_rpc_client_api::config::RpcTransactionConfig;
use solana_rpc_client_api::response::RpcConfirmedTransactionStatusWithSignature;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::{
    EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding,
};
use tokio::sync::Semaphore;

use crate::cli::RpcConfig;
use crate::pacing::{Pacer, PacingConfig, retry_with_backoff};

pub struct HistoryClient {
    client: Arc<RpcClient>,
    semaphore: Arc<Semaphore>,
    pacer: Pacer,
    max_retries: usize,
    backoff_base: std::time::Duration,
    backoff_max: std::time::Duration,
}

impl HistoryClient {
    pub fn new(rpc: &RpcConfig, pacing: &PacingConfig) -> Self {
        Self {
            client: Arc::new(RpcClient::new(rpc.rpc_url.clone())),
            semaphore: Arc::new(Semaphore::new(pacing.max_concurrent_rpc.max(1))),
            pacer: Pacer::new(pacing.min_request_interval),
            max_retries: pacing.max_retries,
            backoff_base: pacing.backoff_base,
            backoff_max: pacing.backoff_max,
        }
    }

    /// The shared client, for callers (pool bootstrap) that need `source`'s own batched
    /// account reader rather than a history-walk call.
    pub fn rpc_client(&self) -> Arc<RpcClient> {
        self.client.clone()
    }

    async fn call<T, F, Fut>(&self, label: &'static str, mut f: F) -> eyre::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = solana_rpc_client_api::client_error::Result<T>>,
    {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .wrap_err_with(|| "Acquiring RPC concurrency permit")?;
        self.pacer.wait().await;
        retry_with_backoff(
            label,
            self.max_retries,
            self.backoff_base,
            self.backoff_max,
            &mut f,
        )
        .await
        .wrap_err_with(|| format!("Calling {label} after retries"))
    }

    /// One page of a pool's signature history, newest-first, walking backward from `before`
    /// (or the chain head, when `before` is `None`).
    pub async fn signatures_page(
        &self,
        pool: &Pubkey,
        before: Option<Signature>,
        limit: usize,
    ) -> eyre::Result<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        // Rebuilt inside the closure on every attempt rather than captured once:
        // `GetConfirmedSignaturesForAddress2Config` does not implement `Clone`, and `before`
        // and `limit` are cheap `Copy` values anyway.
        self.call("getSignaturesForAddress", || {
            let client = self.client.clone();
            let pool = *pool;
            async move {
                let config = GetConfirmedSignaturesForAddress2Config {
                    before,
                    until: None,
                    limit: Some(limit),
                    commitment: None,
                };
                client
                    .get_signatures_for_address_with_config(&pool, config)
                    .await
            }
        })
        .await
    }

    /// Current chain head, used only as the progress denominator when the operator did not
    /// pin an explicit `--to-slot`.
    pub async fn head_slot(&self) -> eyre::Result<u64> {
        self.call("getSlot", || {
            let client = self.client.clone();
            async move { client.get_slot().await }
        })
        .await
    }

    /// The full transaction body for one signature, with inner instructions included so the
    /// self-CPI event data is present to decode.
    pub async fn transaction(
        &self,
        signature: &Signature,
    ) -> eyre::Result<EncodedConfirmedTransactionWithStatusMeta> {
        let config = RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::Json),
            commitment: None,
            max_supported_transaction_version: Some(0),
        };
        self.call("getTransaction", || {
            let client = self.client.clone();
            let signature = *signature;
            async move { client.get_transaction_with_config(&signature, config).await }
        })
        .await
    }
}
