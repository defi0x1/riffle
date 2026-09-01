use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;

use crate::{
    Capabilities, ChainEvent, EventFilter, FlowMetrics, PoolMeta, RpcConfig, Source, StateUpdate,
    WatchSet,
};

use super::datapi::DatapiClient;
use super::discovery;
use super::state::{StatePoller, state_stream};

pub struct RpcSource {
    config: RpcConfig,
    rpc_client: Arc<RpcClient>,
    poller: Arc<StatePoller>,
    datapi: DatapiClient,
}

impl RpcSource {
    pub fn new(config: RpcConfig) -> eyre::Result<Self> {
        let rpc_client = Arc::new(RpcClient::new(config.rpc_url.clone()));
        let poller = Arc::new(StatePoller::new(
            rpc_client.clone(),
            config.max_concurrent_rpc,
            config.max_retries,
        ));
        let datapi = DatapiClient::new(&config)?;
        Ok(Self {
            config,
            rpc_client,
            poller,
            datapi,
        })
    }
}

#[async_trait]
impl Source for RpcSource {
    async fn discover_pools(&self) -> eyre::Result<Vec<PoolMeta>> {
        discovery::discover_pools(&self.rpc_client).await
    }

    fn state_stream(&self, watched: WatchSet) -> BoxStream<'_, StateUpdate> {
        state_stream(
            self.poller.clone(),
            watched,
            self.config.poll_interval_state,
        )
    }

    fn event_stream(&self, _filter: EventFilter) -> BoxStream<'_, ChainEvent> {
        // Swap-level events over RPC would mean a getSignaturesForAddress + getTransaction
        // walk per pool -- affordable for one watched pool, not for a tier of a hundred.
        // That detail is a Geyser-only capability; RPC honestly reports none of it.
        stream::empty().boxed()
    }

    async fn flow_metrics(&self, pools: &[Pubkey]) -> eyre::Result<Option<Vec<FlowMetrics>>> {
        self.datapi.flow_metrics(pools).await.map(Some)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            swap_level_events: false,
            own_flow_metrics: false,
            push_latency: false,
            buy_sell_split: false,
        }
    }
}
