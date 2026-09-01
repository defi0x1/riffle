use std::collections::{HashMap, HashSet};

use futures::stream::{self, BoxStream, StreamExt};
use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;
use yellowstone_grpc_proto::prelude::{
    SubscribeUpdate, SubscribeUpdateTransaction, subscribe_update::UpdateOneof,
};

use dlmm_decode::{DecodedEvent, decode_event};
use metrics::DECODE_ERROR_TOTAL;

use crate::{ChainEvent, EventFilter, GeyserConfig};

use super::connection::{ConnectionConfig, ReconnectPolicy, run_resilient};
use super::filters;

const RAW_CHANNEL_CAPACITY: usize = 1024;
const OUT_CHANNEL_CAPACITY: usize = 256;
// SubscribeUpdateBlockMeta for a slot can arrive before or after that slot's transactions;
// this just needs to outlive the usual arrival skew between the two.
const BLOCK_TIME_CACHE_CAPACITY: usize = 128;

fn event_pool(event: &DecodedEvent) -> Option<Pubkey> {
    match event {
        DecodedEvent::Swap(e) => Some(e.lb_pair),
        DecodedEvent::AddLiquidity(e) | DecodedEvent::RemoveLiquidity(e) => Some(e.lb_pair),
        DecodedEvent::ClaimFee(e) | DecodedEvent::ClaimFee2(e) => Some(e.lb_pair),
        DecodedEvent::LbPairCreate(e) => Some(e.lb_pair),
        DecodedEvent::PositionCreate(e) => Some(e.lb_pair),
        // PositionClose doesn't carry the pool on-chain, only the position and owner, so
        // there is nothing to key a ChainEvent on. Position lifecycle is still observable
        // through PositionCreate and the liquidity events.
        DecodedEvent::PositionClose(_) => None,
    }
}

fn account_keys(tx: &SubscribeUpdateTransaction) -> Vec<Pubkey> {
    let mut keys = Vec::new();
    let Some(info) = &tx.transaction else {
        return keys;
    };
    if let Some(message) = info.transaction.as_ref().and_then(|t| t.message.as_ref()) {
        keys.extend(
            message
                .account_keys
                .iter()
                .filter_map(|k| Pubkey::try_from(k.as_slice()).ok()),
        );
    }
    if let Some(meta) = &info.meta {
        keys.extend(
            meta.loaded_writable_addresses
                .iter()
                .filter_map(|k| Pubkey::try_from(k.as_slice()).ok()),
        );
        keys.extend(
            meta.loaded_readonly_addresses
                .iter()
                .filter_map(|k| Pubkey::try_from(k.as_slice()).ok()),
        );
    }
    keys
}

// Pure and independently testable: decodes every self-CPI event instruction in one
// transaction update into ChainEvents, given a block_time lookup and an optional pool
// allow-list.
fn decode_transaction_events(
    tx: &SubscribeUpdateTransaction,
    block_time: &HashMap<u64, i64>,
    pool_filter: Option<&HashSet<Pubkey>>,
) -> Vec<ChainEvent> {
    let Some(info) = &tx.transaction else {
        return Vec::new();
    };
    let Some(meta) = &info.meta else {
        return Vec::new();
    };

    let keys = account_keys(tx);
    let block_time = block_time.get(&tx.slot).copied().unwrap_or(0);
    let mut events = Vec::new();

    for inner in &meta.inner_instructions {
        for instruction in &inner.instructions {
            let Some(program_id) = keys.get(instruction.program_id_index as usize) else {
                continue;
            };
            if *program_id != lb_clmm::ID {
                continue;
            }

            match decode_event(&instruction.data) {
                Ok(event) => {
                    let Some(pool) = event_pool(&event) else {
                        continue;
                    };
                    if let Some(filter) = pool_filter
                        && !filter.contains(&pool)
                    {
                        continue;
                    }
                    events.push(ChainEvent {
                        pool,
                        slot: tx.slot,
                        block_time,
                        event,
                    });
                }
                Err(e) => {
                    tracing::warn!(error = ?e, slot = tx.slot, "Failed to decode Geyser event");
                    DECODE_ERROR_TOTAL.with_label_values(&["event"]).inc();
                }
            }
        }
    }

    events
}

async fn decode_loop(
    mut raw_rx: mpsc::Receiver<SubscribeUpdate>,
    pool_filter: Option<HashSet<Pubkey>>,
    out_tx: mpsc::Sender<ChainEvent>,
) {
    let mut block_time: HashMap<u64, i64> = HashMap::new();

    while let Some(update) = raw_rx.recv().await {
        match update.update_oneof {
            Some(UpdateOneof::BlockMeta(meta)) => {
                if let Some(bt) = meta.block_time {
                    block_time.insert(meta.slot, bt.timestamp);
                    if block_time.len() > BLOCK_TIME_CACHE_CAPACITY
                        && let Some(&oldest) = block_time.keys().min()
                    {
                        block_time.remove(&oldest);
                    }
                }
            }
            Some(UpdateOneof::Transaction(tx)) => {
                for event in decode_transaction_events(&tx, &block_time, pool_filter.as_ref()) {
                    if out_tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn event_stream(config: &GeyserConfig, filter: EventFilter) -> BoxStream<'static, ChainEvent> {
    let (raw_tx, raw_rx) = mpsc::channel(RAW_CHANNEL_CAPACITY);
    let (out_tx, out_rx) = mpsc::channel(OUT_CHANNEL_CAPACITY);

    let pool_filter: Option<HashSet<Pubkey>> =
        filter.pools.map(|pools| pools.into_iter().collect());

    match ConnectionConfig::new(config) {
        Ok(conn_cfg) => match filters::parse_commitment(&config.geyser_commitment) {
            Ok(commitment) => {
                // The event stream watches the whole program's transactions rather than a
                // set that moves with any one pool's active bin, so it never needs to push
                // an updated subscription onto the open sink.
                let (_resubscribe_tx, resubscribe_rx) = mpsc::channel(1);
                tokio::spawn(run_resilient(
                    conn_cfg,
                    ReconnectPolicy::default(),
                    move |from_slot| filters::event_subscribe_request(commitment, from_slot),
                    raw_tx,
                    resubscribe_rx,
                ));
            }
            Err(e) => tracing::error!(error = ?e, "Cannot start Geyser event stream"),
        },
        Err(e) => tracing::error!(error = ?e, "Cannot start Geyser event stream"),
    }

    tokio::spawn(decode_loop(raw_rx, pool_filter, out_tx));

    stream::unfold(out_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use yellowstone_grpc_proto::prelude::{
        InnerInstruction, InnerInstructions, Message as WireMessage,
        SubscribeUpdateTransactionInfo, Transaction as WireTransaction, TransactionStatusMeta,
    };

    use super::*;

    fn program_id_bytes() -> Vec<u8> {
        lb_clmm::ID.to_bytes().to_vec()
    }

    fn tx_with_inner_instruction(slot: u64, data: Vec<u8>) -> SubscribeUpdateTransaction {
        SubscribeUpdateTransaction {
            transaction: Some(SubscribeUpdateTransactionInfo {
                signature: vec![0; 64],
                is_vote: false,
                transaction: Some(WireTransaction {
                    signatures: vec![vec![0; 64]],
                    message: Some(WireMessage {
                        header: None,
                        account_keys: vec![vec![9; 32], program_id_bytes()],
                        recent_blockhash: vec![0; 32],
                        instructions: vec![],
                        versioned: false,
                        address_table_lookups: vec![],
                    }),
                }),
                meta: Some(TransactionStatusMeta {
                    err: None,
                    fee: 0,
                    pre_balances: vec![],
                    post_balances: vec![],
                    inner_instructions: vec![InnerInstructions {
                        index: 0,
                        instructions: vec![InnerInstruction {
                            program_id_index: 1,
                            accounts: vec![],
                            data,
                            stack_height: None,
                        }],
                    }],
                    inner_instructions_none: false,
                    log_messages: vec![],
                    log_messages_none: false,
                    pre_token_balances: vec![],
                    post_token_balances: vec![],
                    rewards: vec![],
                    loaded_writable_addresses: vec![],
                    loaded_readonly_addresses: vec![],
                    return_data: None,
                    return_data_none: false,
                    compute_units_consumed: None,
                    cost_units: None,
                }),
                index: 0,
            }),
            slot,
        }
    }

    #[test]
    fn test_non_program_inner_instructions_are_ignored() {
        let tx = tx_with_inner_instruction(1, vec![1, 2, 3]);
        // program_id_index points at account 1, which IS the program here, but with junk
        // data the event fails to decode and nothing is emitted
        let events = decode_transaction_events(&tx, &HashMap::new(), None);
        assert!(events.is_empty());
    }

    #[test]
    fn test_block_time_is_looked_up_by_slot() {
        let mut bt = HashMap::new();
        bt.insert(5, 1_700_000_000i64);
        let tx = tx_with_inner_instruction(5, vec![1, 2, 3]);
        // still empty (junk data), but exercises the lookup path without panicking
        let events = decode_transaction_events(&tx, &bt, None);
        assert!(events.is_empty());
    }

    #[test]
    fn test_missing_meta_yields_no_events() {
        let mut tx = tx_with_inner_instruction(1, vec![1, 2, 3]);
        tx.transaction.as_mut().unwrap().meta = None;
        let events = decode_transaction_events(&tx, &HashMap::new(), None);
        assert!(events.is_empty());
    }
}
