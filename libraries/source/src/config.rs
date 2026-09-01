use std::time::Duration;

use clap::Parser;

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Rpc,
    Geyser,
}

#[derive(Parser, Debug, Clone)]
#[group(id = "source")]
pub struct Config {
    /// Which ingestion backend to run: cheap RPC polling (works today, 10-20s cadence) or
    /// paid Geyser streaming (sub-second push). Both write the same schema, so switching
    /// backends is a config change and a restart, not a migration.
    #[arg(long, env, value_enum, default_value_t = Backend::Rpc)]
    pub backend: Backend,

    #[clap(flatten)]
    pub rpc: RpcConfig,

    #[clap(flatten)]
    pub geyser: GeyserConfig,
}

impl Config {
    pub fn is_rpc(&self) -> bool {
        self.backend == Backend::Rpc
    }
}

// rpc_url may carry an embedded API key, so any logging of this config must go through
// this impl rather than the derived Debug.
impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "source::Config {{ backend: {:?}, rpc_url: <redacted>, datapi_url: {}, geyser_endpoint: {}, geyser_x_token: {} }}",
            self.backend,
            self.rpc.datapi_url,
            self.geyser.geyser_endpoint.as_deref().unwrap_or("<unset>"),
            if self.geyser.geyser_x_token.is_some() {
                "<redacted>"
            } else {
                "<unset>"
            },
        )
    }
}

#[derive(Parser, Debug, Clone)]
#[group(id = "source-rpc")]
pub struct RpcConfig {
    /// Solana RPC endpoint used for account reads and pool discovery.
    #[arg(long, env)]
    pub rpc_url: String,

    /// Poll interval for tier-1 (watched) pool + bin state. Lower values increase RPC cost
    /// roughly linearly; below 10s the marginal benefit is small.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "15s")]
    pub poll_interval_state: Duration,

    /// Poll interval for the full pool universe scan (getProgramAccounts, zero-length data
    /// slice). This is the only place gPA is called -- keep it well above any provider's
    /// gPA rate limit.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "600s")]
    pub poll_interval_universe: Duration,

    /// Base URL of Meteora's public data API, used for tier-0 flow metrics (volume, fees,
    /// TVL) since RPC alone cannot supply them affordably.
    #[arg(long, env, default_value = "https://dlmm.datapi.meteora.ag")]
    pub datapi_url: String,

    /// Page size for datapi `/pools` requests. The API ignores `limit`/`per_page`; 500 is
    /// the largest page size observed to work, over ~248 pages for the full universe.
    #[arg(long, env, default_value_t = 500)]
    pub datapi_page_size: u32,

    /// TTL for a confirmed "pool not present in the datapi universe" result, so repeatedly
    /// asking about a delisted or not-yet-indexed pool does not force a full page walk
    /// every time.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "30s")]
    pub negative_cache_ttl: Duration,

    /// Maximum outbound RPC calls in flight at once.
    #[arg(long, env, default_value_t = 8)]
    pub max_concurrent_rpc: usize,

    /// Maximum retry attempts for a failed RPC or datapi call before giving up.
    #[arg(long, env, default_value_t = 5)]
    pub max_retries: usize,
}

#[derive(Parser, Debug, Clone)]
#[group(id = "source-geyser")]
pub struct GeyserConfig {
    /// Yellowstone Geyser gRPC endpoint. Required only when backend=geyser.
    #[arg(long, env)]
    pub geyser_endpoint: Option<String>,

    /// Geyser authentication token.
    #[arg(long, env)]
    pub geyser_x_token: Option<String>,

    /// Commitment level for the Geyser subscription.
    #[arg(long, env, default_value = "confirmed")]
    pub geyser_commitment: String,
}
