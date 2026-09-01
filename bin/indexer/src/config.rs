use std::time::Duration;

use clap::Parser;

// Magic numbers referenced from clap defaults below, kept out of the field declarations
// themselves.
pub mod tier {
    /// Fraction of the tier-1 watch set reserved for pools that have never been measured,
    /// so a weak screening prior can never permanently hide a pool from promotion.
    pub const DEFAULT_EXPLORATION_SLICE: f64 = 0.10;

    /// Extra slots below the rank cutoff a currently-watched pool may fall into before it
    /// is actually demoted. Without this, a pool sitting near the cutoff flaps in and out
    /// of the watch set on every sweep.
    pub const DEFAULT_DEMOTION_MARGIN: i64 = 20;
}

pub mod discovery {
    /// Newly discovered pools onboarded (full on-chain state fetched, row created) per
    /// discovery tick. Bounds the RPC burst on a cold start against an existing large
    /// universe instead of fetching all of it in one tick.
    pub const DEFAULT_BATCH_SIZE: usize = 200;
}

pub mod state {
    /// Coalesced state updates are flushed once this many distinct pools are buffered,
    /// independent of the flush timer.
    pub const DEFAULT_FLUSH_BATCH_SIZE: usize = 50;
}

pub mod event {
    /// Coalesced chain events are flushed once this many are buffered, independent of the
    /// flush timer.
    pub const DEFAULT_FLUSH_BATCH_SIZE: usize = 200;
}

pub mod health {
    /// Average Solana slot time, used to convert a wall-clock staleness figure into an
    /// approximate slot lag when the backend does not report one directly (RPC).
    pub const SLOT_TIME_SECS: f64 = 0.4;
}

// DLMM's `collect_fee_mode` is not yet surfaced by the account decoder (see
// dlmm_decode::PoolState). 0 is the mode every pool observed so far uses; recorded here
// rather than inline so the placeholder is visible and searchable.
pub const DEFAULT_COLLECT_FEE_MODE: i16 = 0;

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[clap(flatten)]
    pub logging: logger::Config,

    #[clap(flatten)]
    pub postgres: common::PostgresConfig,

    #[clap(flatten)]
    pub source: source::Config,

    #[clap(flatten)]
    pub metrics: metrics::Config,

    #[clap(flatten)]
    pub tier: TierConfig,

    /// Interval between full-universe pool discovery sweeps.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "600s")]
    pub discovery_interval: Duration,

    /// Newly discovered pools fully onboarded per discovery tick.
    #[arg(long, env, default_value_t = discovery::DEFAULT_BATCH_SIZE)]
    pub discovery_batch_size: usize,

    /// How often buffered pool/bin state is flushed to Postgres, independent of batch size.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "10s")]
    pub state_flush_interval: Duration,

    /// Buffered pool/bin state updates that force an immediate flush once reached.
    #[arg(long, env, default_value_t = state::DEFAULT_FLUSH_BATCH_SIZE)]
    pub state_flush_batch_size: usize,

    /// How often buffered chain events are flushed to Postgres, independent of batch size.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "5s")]
    pub event_flush_interval: Duration,

    /// Buffered chain events that force an immediate flush once reached.
    #[arg(long, env, default_value_t = event::DEFAULT_FLUSH_BATCH_SIZE)]
    pub event_flush_batch_size: usize,

    /// Interval between ingest_health writes and metric refreshes.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "30s")]
    pub health_interval: Duration,

    /// Data older than this is no longer considered fresh for heartbeat purposes.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "60s")]
    pub health_freshness_threshold: Duration,
}

#[derive(Parser, Debug, Clone)]
#[group(id = "tier")]
pub struct TierConfig {
    /// Maximum size of the tier-1 (watched) pool set.
    #[arg(long, env, default_value_t = 100)]
    pub max_watched: i64,

    /// How often tier membership is re-evaluated.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "300s")]
    pub promotion_interval: Duration,

    /// Fraction of watch-set slots reserved for pools that have never been measured.
    #[arg(long, env, default_value_t = tier::DEFAULT_EXPLORATION_SLICE)]
    pub exploration_slice: f64,

    /// Extra slots below the rank cutoff a watched pool may fall into before it is demoted.
    #[arg(long, env, default_value_t = tier::DEFAULT_DEMOTION_MARGIN)]
    pub demotion_margin: i64,
}

// database_url and any embedded RPC/Geyser credentials must never reach a log line, so this
// composes each sub-config's own redacted Display rather than deriving Debug.
impl std::fmt::Display for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "indexer::Args {{ log_level: {}, log_format: {:?}, postgres: {}, {}, metrics_port: {}, \
             tier: {{ max_watched: {}, promotion_interval: {:?}, exploration_slice: {}, demotion_margin: {} }}, \
             discovery_interval: {:?}, discovery_batch_size: {}, \
             state_flush_interval: {:?}, state_flush_batch_size: {}, \
             event_flush_interval: {:?}, event_flush_batch_size: {}, \
             health_interval: {:?}, health_freshness_threshold: {:?} }}",
            self.logging.log_level,
            self.logging.log_format,
            self.postgres,
            self.source,
            self.metrics.metrics_port,
            self.tier.max_watched,
            self.tier.promotion_interval,
            self.tier.exploration_slice,
            self.tier.demotion_margin,
            self.discovery_interval,
            self.discovery_batch_size,
            self.state_flush_interval,
            self.state_flush_batch_size,
            self.event_flush_interval,
            self.event_flush_batch_size,
            self.health_interval,
            self.health_freshness_threshold,
        )
    }
}
