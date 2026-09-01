mod domain;
pub use domain::*;

mod config;
pub use config::*;

mod bin_array;

pub mod geyser;
pub mod rpc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use solana_sdk::pubkey::Pubkey;

/// Ingestion is behind this trait so the same downstream pipeline runs on either backend.
/// RPC and Geyser write identical domain types; only how each one is produced differs.
#[async_trait]
pub trait Source: Send + Sync {
    /// Enumerate the pool universe. RPC: a zero-slice `getProgramAccounts` scan on a slow
    /// timer. Geyser: the same scan once at startup, then `LbPairCreate` events live.
    async fn discover_pools(&self) -> eyre::Result<Vec<PoolMeta>>;

    /// Pool + bin state for the watched (tier-1) set. RPC: grouped `getMultipleAccounts`
    /// batches on a timer. Geyser: an account subscription, pushed on change.
    fn state_stream(&self, watched: WatchSet) -> BoxStream<'_, StateUpdate>;

    /// Swap, liquidity and fee-parameter events. RPC has no affordable way to produce
    /// these (it would mean a signature walk per pool) and returns an empty stream. Geyser
    /// decodes them from a full transaction subscription.
    fn event_stream(&self, filter: EventFilter) -> BoxStream<'_, ChainEvent>;

    /// Flow metrics (volume, fees, TVL) for pools we cannot derive them for ourselves. RPC:
    /// the public data API. Geyser: `None` -- flow is computed from our own decoded events.
    async fn flow_metrics(&self, pools: &[Pubkey]) -> eyre::Result<Option<Vec<FlowMetrics>>>;

    fn capabilities(&self) -> Capabilities;
}
