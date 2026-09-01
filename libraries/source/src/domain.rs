use solana_sdk::pubkey::Pubkey;

use dlmm_decode::{BinArrayState, DecodedEvent, PoolState};

/// A pool discovered via enumeration. RPC discovery reads a zero-length data slice, so
/// nothing beyond the address and the slot it was seen at is available here -- state and
/// metadata are filled in separately, by a grouped account read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolMeta {
    pub address: Pubkey,
    pub discovered_at_slot: u64,
}

/// The tier-1 (watched) pool set a state stream should poll or subscribe to.
#[derive(Clone, Debug, Default)]
pub struct WatchSet {
    pub pools: Vec<Pubkey>,
}

/// `LbPair` and its surrounding `BinArray`s for one pool, read at a single slot and
/// stamped with that batch's on-chain time, never wall-clock time.
#[derive(Clone, Debug)]
pub struct StateUpdate {
    pub pool: Pubkey,
    pub slot: u64,
    pub block_time: i64,
    pub lb_pair: Option<PoolState>,
    pub bin_arrays: Vec<BinArrayState>,
}

/// Restricts an event stream to a pool subset; `None` means the whole watched universe.
#[derive(Clone, Debug, Default)]
pub struct EventFilter {
    pub pools: Option<Vec<Pubkey>>,
}

/// A decoded chain event, stamped with the slot and on-chain time it occurred at.
#[derive(Clone, Debug)]
pub struct ChainEvent {
    pub pool: Pubkey,
    pub slot: u64,
    pub block_time: i64,
    pub event: DecodedEvent,
}

/// One metric reported over several rolling windows by the public data API.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowedMetric {
    pub m30: f64,
    pub h1: f64,
    pub h2: f64,
    pub h4: f64,
    pub h12: f64,
    pub h24: f64,
}

/// Flow metrics for a pool (volume, fees, TVL) sourced from the public data API, since RPC
/// alone cannot supply per-pool swap aggregates without an unaffordable signature walk.
#[derive(Clone, Debug)]
pub struct FlowMetrics {
    pub pool: Pubkey,
    pub tvl: f64,
    pub current_price: f64,
    pub bin_step: u16,
    pub base_fee_pct: f64,
    pub dynamic_fee_pct: f64,
    pub protocol_fee_pct: f64,
    // fee_24h / tvl * 100 -- the API's own `apr` is a daily ratio, not annualised. `apy` is.
    pub apr: f64,
    pub apy: f64,
    pub volume: WindowedMetric,
    pub fees: WindowedMetric,
    pub fee_tvl_ratio: WindowedMetric,
    pub has_farm: bool,
    pub is_blacklisted: bool,
    pub launchpad: Option<String>,
}

/// What the active backend can supply. The scorer reads this to decide whether the
/// timing-based organic-flow estimator is available and to label rationale accordingly --
/// this is not decoration, an alert must say which estimators fed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub swap_level_events: bool,
    pub own_flow_metrics: bool,
    pub push_latency: bool,
    pub buy_sell_split: bool,
}
