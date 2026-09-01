mod coalesce;
mod connection;
mod discovery;
mod events;
mod filters;
mod state;

use async_trait::async_trait;
use futures::stream::BoxStream;
use solana_sdk::pubkey::Pubkey;

use crate::{
    Capabilities, ChainEvent, EventFilter, FlowMetrics, GeyserConfig, PoolMeta, Source,
    StateUpdate, WatchSet,
};

pub struct GeyserSource {
    config: GeyserConfig,
}

impl GeyserSource {
    pub fn new(config: GeyserConfig) -> eyre::Result<Self> {
        // Fail at construction, not on first stream use: a missing endpoint or an
        // unparseable commitment level should surface at startup.
        connection::ConnectionConfig::new(&config)?;
        filters::parse_commitment(&config.geyser_commitment)?;
        Ok(Self { config })
    }
}

#[async_trait]
impl Source for GeyserSource {
    async fn discover_pools(&self) -> eyre::Result<Vec<PoolMeta>> {
        discovery::discover_pools(&self.config).await
    }

    fn state_stream(&self, watched: WatchSet) -> BoxStream<'_, StateUpdate> {
        state::state_stream(&self.config, watched)
    }

    fn event_stream(&self, filter: EventFilter) -> BoxStream<'_, ChainEvent> {
        events::event_stream(&self.config, filter)
    }

    async fn flow_metrics(&self, _pools: &[Pubkey]) -> eyre::Result<Option<Vec<FlowMetrics>>> {
        // Geyser derives its own flow metrics from decoded swap events rather than
        // deferring to the public API.
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
