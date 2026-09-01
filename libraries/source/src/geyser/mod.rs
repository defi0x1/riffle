use async_trait::async_trait;
use futures::stream::BoxStream;
use solana_sdk::pubkey::Pubkey;

use crate::{
    Capabilities, ChainEvent, EventFilter, FlowMetrics, GeyserConfig, PoolMeta, Source,
    StateUpdate, WatchSet,
};

pub struct GeyserSource {
    #[allow(dead_code)]
    config: GeyserConfig,
}

impl GeyserSource {
    pub fn new(config: GeyserConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Source for GeyserSource {
    async fn discover_pools(&self) -> eyre::Result<Vec<PoolMeta>> {
        // TODO: gPA once at startup, then LbPairCreate events kept live. Lands with the
        // rest of the Geyser backend.
        unimplemented!("Geyser backend lands in a later milestone")
    }

    fn state_stream(&self, _watched: WatchSet) -> BoxStream<'_, StateUpdate> {
        // TODO: account subscription pushed on change, coalesced by pubkey and guarded by
        // slot. Lands with the rest of the Geyser backend.
        unimplemented!("Geyser backend lands in a later milestone")
    }

    fn event_stream(&self, _filter: EventFilter) -> BoxStream<'_, ChainEvent> {
        // TODO: full transaction subscription, decoded to our own events. Lands with the
        // rest of the Geyser backend.
        unimplemented!("Geyser backend lands in a later milestone")
    }

    async fn flow_metrics(&self, _pools: &[Pubkey]) -> eyre::Result<Option<Vec<FlowMetrics>>> {
        // Geyser computes its own flow metrics from decoded events rather than deferring to
        // the public API, so this always returns `None` once wired up.
        Ok(None)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            swap_level_events: true,
            own_flow_metrics: true,
            push_latency: true,
            buy_sell_split: true,
        }
    }
}
