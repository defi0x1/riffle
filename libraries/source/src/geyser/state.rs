use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::stream::{self, BoxStream, StreamExt};
use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;
use yellowstone_grpc_proto::prelude::{SubscribeUpdate, subscribe_update::UpdateOneof};

use dlmm_decode::{BinArrayState, PoolState, decode_bin_array, decode_lb_pair};
use metrics::DECODE_ERROR_TOTAL;

use crate::{GeyserConfig, StateUpdate, WatchSet};

use super::coalesce::SlotCoalescer;
use super::connection::{ConnectionConfig, ReconnectPolicy, run_resilient};
use super::filters::{self, BIN_ARRAY_FILTER, LB_PAIR_FILTER};

const FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const FLUSH_SIZE: usize = 64;
const RAW_CHANNEL_CAPACITY: usize = 1024;
const OUT_CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
enum RawKind {
    LbPair(PoolState),
    BinArray(BinArrayState),
}

#[derive(Clone)]
struct RawAccountUpdate {
    account: Pubkey,
    pool: Pubkey,
    slot: u64,
    kind: RawKind,
}

fn decode_account_update(
    update: SubscribeUpdate,
    watched: &HashSet<Pubkey>,
) -> Option<RawAccountUpdate> {
    let UpdateOneof::Account(acc) = update.update_oneof? else {
        return None;
    };
    let info = acc.account?;
    let account = Pubkey::try_from(info.pubkey.as_slice()).ok()?;

    if update.filters.iter().any(|f| f == LB_PAIR_FILTER) {
        return match decode_lb_pair(&info.data) {
            Ok(state) => Some(RawAccountUpdate {
                account,
                pool: account,
                slot: acc.slot,
                kind: RawKind::LbPair(state),
            }),
            Err(e) => {
                tracing::warn!(error = ?e, pool = %account, "Failed to decode LbPair account");
                DECODE_ERROR_TOTAL.with_label_values(&["lb_pair"]).inc();
                None
            }
        };
    }

    if update.filters.iter().any(|f| f == BIN_ARRAY_FILTER) {
        return match decode_bin_array(&info.data) {
            Ok(state) => {
                // BinArray subscriptions can't be scoped server-side to the watched set
                // (see filters.rs), so the narrowing happens here once the pool is known.
                if !watched.contains(&state.lb_pair) {
                    return None;
                }
                Some(RawAccountUpdate {
                    account,
                    pool: state.lb_pair,
                    slot: acc.slot,
                    kind: RawKind::BinArray(state),
                })
            }
            Err(e) => {
                tracing::warn!(error = ?e, account = %account, "Failed to decode BinArray account");
                DECODE_ERROR_TOTAL.with_label_values(&["bin_array"]).inc();
                None
            }
        };
    }

    None
}

// Pure and independently testable: groups whatever raw account updates a flush window
// collected into one StateUpdate per pool. `last_block_time` carries the LbPair's own
// on-chain `last_updated_at` forward for pools where only a BinArray changed in this
// window, since BinArray accounts carry no timestamp of their own.
fn group_into_state_updates(
    items: Vec<RawAccountUpdate>,
    last_block_time: &mut HashMap<Pubkey, i64>,
) -> Vec<StateUpdate> {
    let mut by_pool: HashMap<Pubkey, (Option<PoolState>, Vec<BinArrayState>, u64)> = HashMap::new();

    for item in items {
        let entry = by_pool.entry(item.pool).or_insert((None, Vec::new(), 0));
        entry.2 = entry.2.max(item.slot);
        match item.kind {
            RawKind::LbPair(state) => entry.0 = Some(state),
            RawKind::BinArray(state) => entry.1.push(state),
        }
    }

    by_pool
        .into_iter()
        .map(|(pool, (lb_pair, bin_arrays, slot))| {
            if let Some(state) = &lb_pair {
                last_block_time.insert(pool, state.last_updated_at);
            }
            let block_time = last_block_time.get(&pool).copied().unwrap_or(0);
            StateUpdate {
                pool,
                slot,
                block_time,
                lb_pair,
                bin_arrays,
            }
        })
        .collect()
}

async fn flush(
    coalescer: &mut SlotCoalescer<Pubkey, RawAccountUpdate>,
    last_block_time: &mut HashMap<Pubkey, i64>,
    out_tx: &mpsc::Sender<StateUpdate>,
) -> bool {
    if coalescer.is_empty() {
        return true;
    }
    let items = coalescer.snapshot();
    let updates = group_into_state_updates(items, last_block_time);
    for update in updates {
        if out_tx.send(update).await.is_err() {
            // downstream is gone; leave the buffer intact, there is nothing left to retry for
            return false;
        }
    }
    // clear only now that every update from this flush made it out -- a send failure above
    // returns before this point, so a retry on the next flush re-sends rather than losing data
    coalescer.clear();
    true
}

async fn coalesce_loop(
    mut raw_rx: mpsc::Receiver<SubscribeUpdate>,
    watched: HashSet<Pubkey>,
    out_tx: mpsc::Sender<StateUpdate>,
) {
    let mut coalescer: SlotCoalescer<Pubkey, RawAccountUpdate> = SlotCoalescer::new();
    let mut last_block_time: HashMap<Pubkey, i64> = HashMap::new();
    let mut flush_interval = tokio::time::interval(FLUSH_INTERVAL);
    flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = flush_interval.tick() => {
                if !flush(&mut coalescer, &mut last_block_time, &out_tx).await {
                    return;
                }
            }
            maybe_update = raw_rx.recv() => {
                let Some(update) = maybe_update else { return };
                if let Some(raw) = decode_account_update(update, &watched) {
                    coalescer.offer(raw.account, raw.slot, raw);
                    if coalescer.len() >= FLUSH_SIZE
                        && !flush(&mut coalescer, &mut last_block_time, &out_tx).await
                    {
                        return;
                    }
                }
            }
        }
    }
}

pub fn state_stream(config: &GeyserConfig, watched: WatchSet) -> BoxStream<'static, StateUpdate> {
    let (raw_tx, raw_rx) = mpsc::channel(RAW_CHANNEL_CAPACITY);
    let (out_tx, out_rx) = mpsc::channel(OUT_CHANNEL_CAPACITY);

    let watched_set: HashSet<Pubkey> = watched.pools.iter().copied().collect();

    match ConnectionConfig::new(config) {
        Ok(conn_cfg) => match filters::parse_commitment(&config.geyser_commitment) {
            Ok(commitment) => {
                let pools = watched.pools.clone();
                tokio::spawn(run_resilient(
                    conn_cfg,
                    ReconnectPolicy::default(),
                    move |from_slot| {
                        filters::state_subscribe_request(&pools, commitment, from_slot)
                    },
                    raw_tx,
                ));
            }
            Err(e) => tracing::error!(error = ?e, "Cannot start Geyser state stream"),
        },
        Err(e) => tracing::error!(error = ?e, "Cannot start Geyser state stream"),
    }

    tokio::spawn(coalesce_loop(raw_rx, watched_set, out_tx));

    stream::unfold(out_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use solana_sdk::pubkey::Pubkey;

    use dlmm_decode::PoolStatus;

    use super::*;

    fn pool(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn lb_pair_state(last_updated_at: i64) -> PoolState {
        PoolState {
            token_x_mint: pool(10),
            token_y_mint: pool(11),
            reserve_x: pool(12),
            reserve_y: pool(13),
            oracle: pool(14),
            bin_step: 10,
            active_bin_id: 0,
            status: PoolStatus::Enabled,
            base_factor: 0,
            base_fee_power_factor: 0,
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            variable_fee_control: 0,
            max_volatility_accumulator: 0,
            protocol_share_bps: 0,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            protocol_fee_x: 0,
            protocol_fee_y: 0,
            last_updated_at,
        }
    }

    fn bin_array_state(lb_pair: Pubkey, index: i64) -> BinArrayState {
        BinArrayState {
            lb_pair,
            index,
            bins: Vec::new(),
        }
    }

    #[test]
    fn test_lb_pair_and_bin_arrays_for_the_same_pool_group_into_one_update() {
        let p = pool(1);
        let items = vec![
            RawAccountUpdate {
                account: p,
                pool: p,
                slot: 10,
                kind: RawKind::LbPair(lb_pair_state(555)),
            },
            RawAccountUpdate {
                account: pool(2),
                pool: p,
                slot: 10,
                kind: RawKind::BinArray(bin_array_state(p, -1)),
            },
            RawAccountUpdate {
                account: pool(3),
                pool: p,
                slot: 9,
                kind: RawKind::BinArray(bin_array_state(p, 0)),
            },
        ];
        let mut cache = HashMap::new();
        let updates = group_into_state_updates(items, &mut cache);
        assert_eq!(updates.len(), 1);
        let update = &updates[0];
        assert_eq!(update.pool, p);
        assert_eq!(update.slot, 10);
        assert_eq!(update.block_time, 555);
        assert!(update.lb_pair.is_some());
        assert_eq!(update.bin_arrays.len(), 2);
    }

    #[test]
    fn test_bin_array_only_update_falls_back_to_last_known_block_time() {
        let p = pool(1);
        let mut cache = HashMap::new();
        cache.insert(p, 777);
        let items = vec![RawAccountUpdate {
            account: pool(2),
            pool: p,
            slot: 20,
            kind: RawKind::BinArray(bin_array_state(p, 0)),
        }];
        let updates = group_into_state_updates(items, &mut cache);
        assert_eq!(updates[0].block_time, 777);
        assert!(updates[0].lb_pair.is_none());
    }

    #[test]
    fn test_distinct_pools_produce_distinct_updates() {
        let (p1, p2) = (pool(1), pool(2));
        let items = vec![
            RawAccountUpdate {
                account: p1,
                pool: p1,
                slot: 1,
                kind: RawKind::LbPair(lb_pair_state(1)),
            },
            RawAccountUpdate {
                account: p2,
                pool: p2,
                slot: 2,
                kind: RawKind::LbPair(lb_pair_state(2)),
            },
        ];
        let mut cache = HashMap::new();
        let updates = group_into_state_updates(items, &mut cache);
        assert_eq!(updates.len(), 2);
    }
}
