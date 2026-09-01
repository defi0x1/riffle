//! Turns one fetched transaction into the same domain events the Geyser backend produces
//! live, then into the storage rows the indexer's event worker writes. The decode step
//! (`dlmm_decode::decode_event` over each self-CPI inner instruction) is identical to what
//! `source::geyser::events` does; the difference here is upstream of that -- the bytes come
//! from a historical `getTransaction` response instead of a subscription push, which is also
//! why a real signature and instruction index are available instead of the synthetic key the
//! live path has to fall back on.

use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status_client_types::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiInstruction, UiMessage,
    UiTransactionStatusMeta,
};

use dlmm_decode::{DecodedEvent, LiquidityEventKind, decode_event};
use storage::types::liquidity_action;
use storage::write::{NewLiquidityEvent, NewSwap};

fn decimal_from_u64(x: u64) -> Decimal {
    Decimal::from(x)
}

pub fn unix_to_datetime(unix_secs: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(unix_secs, 0).unwrap_or_else(chrono::Utc::now)
}

/// One decoded self-CPI event, still tagged with where in the transaction it came from --
/// `ix_index` is a synthetic-but-stable combination of the top-level instruction index and
/// its position within that instruction's inner-instruction list, since the schema's natural
/// key is `(pool_address, ts, signature, ix_index)` and two events in the same transaction
/// always share a signature and timestamp.
pub struct TxEvent {
    pub ix_index: i32,
    pub event: DecodedEvent,
}

fn event_pool(event: &DecodedEvent) -> Option<Pubkey> {
    match event {
        DecodedEvent::Swap(e) => Some(e.lb_pair),
        DecodedEvent::AddLiquidity(e) | DecodedEvent::RemoveLiquidity(e) => Some(e.lb_pair),
        DecodedEvent::ClaimFee(e) | DecodedEvent::ClaimFee2(e) => Some(e.lb_pair),
        DecodedEvent::LbPairCreate(e) => Some(e.lb_pair),
        DecodedEvent::PositionCreate(e) => Some(e.lb_pair),
        DecodedEvent::PositionClose(_) => None,
    }
}

/// The account keys a compiled instruction's `program_id_index` indexes into: the message's
/// own keys, followed by address-lookup-table keys resolved for this transaction (writable
/// then readonly, the same order `getTransaction` reports them in).
fn account_keys(meta: &UiTransactionStatusMeta, message_keys: &[String]) -> Vec<String> {
    let mut keys = message_keys.to_vec();
    if let Some(loaded) = Option::from(meta.loaded_addresses.clone()) {
        let loaded: solana_transaction_status_client_types::UiLoadedAddresses = loaded;
        keys.extend(loaded.writable);
        keys.extend(loaded.readonly);
    }
    keys
}

/// Decodes every DLMM self-CPI event out of one transaction. Returns nothing for a
/// failed transaction (its inner instructions never committed, so a "swap" decoded from one
/// would describe liquidity movement that never happened) or one with no metadata at all.
pub fn decode_transaction(
    tx: &EncodedConfirmedTransactionWithStatusMeta,
    pool_filter: &Pubkey,
) -> Vec<TxEvent> {
    let Some(meta) = &tx.transaction.meta else {
        return Vec::new();
    };
    if meta.err.is_some() {
        return Vec::new();
    }
    let EncodedTransaction::Json(ui_tx) = &tx.transaction.transaction else {
        return Vec::new();
    };
    let UiMessage::Raw(message) = &ui_tx.message else {
        return Vec::new();
    };

    let keys = account_keys(meta, &message.account_keys);
    let program_id = dlmm_decode::ID.to_string();

    let Some(inner_groups): Option<Vec<_>> = Option::from(meta.inner_instructions.clone()) else {
        return Vec::new();
    };

    let mut events = Vec::new();
    for group in inner_groups {
        for (position, instruction) in group.instructions.iter().enumerate() {
            let UiInstruction::Compiled(compiled) = instruction else {
                continue;
            };
            let Some(candidate) = keys.get(compiled.program_id_index as usize) else {
                continue;
            };
            if *candidate != program_id {
                continue;
            }
            let Ok(data) = bs58::decode(&compiled.data).into_vec() else {
                continue;
            };
            let Ok(event) = decode_event(&data) else {
                continue;
            };
            let Some(pool) = event_pool(&event) else {
                continue;
            };
            if pool != *pool_filter {
                continue;
            }
            // group.index is the top-level instruction index (u8); scaling it well clear of
            // the inner-position range keeps the combined key unique across every group in
            // one transaction without needing the inner position's own width.
            let ix_index = (group.index as i32) * 1_000 + position as i32;
            events.push(TxEvent { ix_index, event });
        }
    }
    events
}

/// Maps one decoded event into the storage row(s) it belongs in, appending to whichever
/// buffer applies. Mirrors `bin/indexer`'s event worker field-for-field so a range covered by
/// both the crawler and the live indexer produces identical rows for the overlap; only
/// `signature`/`ix_index` differ in kind (real here, synthetic there).
pub fn append_rows(
    pool_address: &str,
    ts: chrono::DateTime<chrono::Utc>,
    slot: u64,
    signature: &str,
    tx_event: TxEvent,
    swaps: &mut Vec<NewSwap>,
    liquidity: &mut Vec<NewLiquidityEvent>,
) {
    match tx_event.event {
        DecodedEvent::Swap(swap) => {
            let fee_raw = decimal_from_u64(swap.lp_fee) + decimal_from_u64(swap.protocol_fee);
            swaps.push(NewSwap {
                pool_address: pool_address.to_string(),
                ts,
                slot: slot as i64,
                signature: signature.to_string(),
                ix_index: tx_event.ix_index,
                signer: swap.trader.to_string(),
                swap_for_y: swap.swap_for_y,
                amount_in_raw: decimal_from_u64(swap.amount_in),
                amount_out_raw: decimal_from_u64(swap.amount_out),
                // Token decimals are not resolved at this layer, matching the live event
                // worker; a later reconciliation pass scales these once decimals are known.
                amount_in: decimal_from_u64(swap.amount_in),
                amount_out: decimal_from_u64(swap.amount_out),
                start_bin_id: swap.start_bin_id,
                end_bin_id: swap.end_bin_id,
                start_price: None,
                end_price: None,
                fee_raw,
                protocol_fee_raw: decimal_from_u64(swap.protocol_fee),
                host_fee_raw: Some(decimal_from_u64(swap.host_fee)),
                fee_bps: decimal_from_u64(swap.fee_bps),
                volume_usd: None,
                trade_fee_usd: None,
                protocol_fee_usd: None,
            });
        }
        DecodedEvent::AddLiquidity(liq) | DecodedEvent::RemoveLiquidity(liq) => {
            let action = match liq.kind {
                LiquidityEventKind::Add => liquidity_action::ADD,
                LiquidityEventKind::Remove => liquidity_action::REMOVE,
            };
            liquidity.push(NewLiquidityEvent {
                pool_address: pool_address.to_string(),
                ts,
                slot: slot as i64,
                signature: signature.to_string(),
                ix_index: tx_event.ix_index,
                position_address: Some(liq.position.to_string()),
                owner: liq.from.to_string(),
                action,
                active_bin_id: liq.active_bin_id,
                amount_x_raw: Some(decimal_from_u64(liq.amount_x)),
                amount_y_raw: Some(decimal_from_u64(liq.amount_y)),
                amount_usd: None,
            });
        }
        // ClaimFee/ClaimFee2/LbPairCreate/PositionCreate/PositionClose have no write path in
        // the current schema, on RPC or on Geyser -- see the event worker's own comment.
        // Nothing to do here without adding a table this crate does not own.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dlmm_decode::{DecodedClaimFee, DecodedLiquidityEvent, DecodedSwap};

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    #[test]
    fn test_append_rows_maps_swap_fields() {
        let pool = pk(1).to_string();
        let ts = unix_to_datetime(1_700_000_000);
        let mut swaps = Vec::new();
        let mut liquidity = Vec::new();

        let tx_event = TxEvent {
            ix_index: 3,
            event: DecodedEvent::Swap(DecodedSwap {
                lb_pair: pk(1),
                trader: pk(2),
                start_bin_id: 10,
                end_bin_id: 12,
                amount_in: 1_000,
                amount_out: 900,
                swap_for_y: true,
                fee_bps: 30,
                lp_fee: 25,
                protocol_fee: 5,
                host_fee: 1,
            }),
        };

        append_rows(
            &pool,
            ts,
            55,
            "sigABC",
            tx_event,
            &mut swaps,
            &mut liquidity,
        );

        assert_eq!(swaps.len(), 1);
        assert!(liquidity.is_empty());
        let row = &swaps[0];
        assert_eq!(row.pool_address, pool);
        assert_eq!(row.signature, "sigABC");
        assert_eq!(row.ix_index, 3);
        assert_eq!(row.slot, 55);
        assert_eq!(row.fee_raw, Decimal::from(30));
        assert_eq!(row.protocol_fee_raw, Decimal::from(5));
    }

    #[test]
    fn test_append_rows_maps_liquidity_action() {
        let pool = pk(1).to_string();
        let ts = unix_to_datetime(1_700_000_000);
        let mut swaps = Vec::new();
        let mut liquidity = Vec::new();

        let tx_event = TxEvent {
            ix_index: 0,
            event: DecodedEvent::RemoveLiquidity(DecodedLiquidityEvent {
                kind: LiquidityEventKind::Remove,
                lb_pair: pk(1),
                from: pk(2),
                position: pk(3),
                amount_x: 10,
                amount_y: 20,
                active_bin_id: 5,
            }),
        };

        append_rows(&pool, ts, 1, "sig1", tx_event, &mut swaps, &mut liquidity);

        assert!(swaps.is_empty());
        assert_eq!(liquidity.len(), 1);
        assert_eq!(liquidity[0].action, liquidity_action::REMOVE);
        assert_eq!(liquidity[0].active_bin_id, 5);
    }

    #[test]
    fn test_append_rows_skips_unwritable_event_kinds() {
        let pool = pk(1).to_string();
        let ts = unix_to_datetime(0);
        let mut swaps = Vec::new();
        let mut liquidity = Vec::new();

        let tx_event = TxEvent {
            ix_index: 0,
            event: DecodedEvent::ClaimFee(DecodedClaimFee {
                lb_pair: pk(1),
                position: pk(2),
                owner: pk(3),
                fee_x: 1,
                fee_y: 2,
                active_bin_id: None,
            }),
        };

        append_rows(&pool, ts, 1, "sig1", tx_event, &mut swaps, &mut liquidity);

        assert!(swaps.is_empty());
        assert!(liquidity.is_empty());
    }

    fn empty_meta() -> UiTransactionStatusMeta {
        use solana_transaction_status_client_types::option_serializer::OptionSerializer;
        UiTransactionStatusMeta {
            err: None,
            status: Ok(()),
            fee: 0,
            pre_balances: vec![],
            post_balances: vec![],
            inner_instructions: OptionSerializer::None,
            log_messages: OptionSerializer::None,
            pre_token_balances: OptionSerializer::None,
            post_token_balances: OptionSerializer::None,
            rewards: OptionSerializer::None,
            loaded_addresses: OptionSerializer::Skip,
            return_data: OptionSerializer::Skip,
            compute_units_consumed: OptionSerializer::Skip,
            cost_units: OptionSerializer::Skip,
        }
    }

    fn tx_with_inner_instructions(
        slot: u64,
        account_keys: Vec<String>,
        groups: Vec<solana_transaction_status_client_types::UiInnerInstructions>,
        err: Option<solana_sdk::transaction::TransactionError>,
    ) -> EncodedConfirmedTransactionWithStatusMeta {
        use solana_transaction_status_client_types::option_serializer::OptionSerializer;
        use solana_transaction_status_client_types::{
            EncodedTransactionWithStatusMeta, UiRawMessage, UiTransaction,
        };

        let mut meta = empty_meta();
        meta.err = err;
        meta.inner_instructions = OptionSerializer::Some(groups);

        EncodedConfirmedTransactionWithStatusMeta {
            slot,
            block_time: Some(1_700_000_000),
            transaction: EncodedTransactionWithStatusMeta {
                transaction: EncodedTransaction::Json(UiTransaction {
                    signatures: vec!["sig1".to_string()],
                    message: UiMessage::Raw(UiRawMessage {
                        header: Default::default(),
                        account_keys,
                        recent_blockhash: "11111111111111111111111111111111".to_string(),
                        instructions: vec![],
                        address_table_lookups: None,
                    }),
                }),
                meta: Some(meta),
                version: None,
            },
        }
    }

    fn compiled_group(
        index: u8,
        program_id_index: u8,
        data: &[u8],
    ) -> solana_transaction_status_client_types::UiInnerInstructions {
        use solana_transaction_status_client_types::{UiCompiledInstruction, UiInnerInstructions};
        UiInnerInstructions {
            index,
            instructions: vec![UiInstruction::Compiled(UiCompiledInstruction {
                program_id_index,
                accounts: vec![],
                data: bs58::encode(data).into_string(),
                stack_height: None,
            })],
        }
    }

    #[test]
    fn test_decode_transaction_ignores_non_program_inner_instructions() {
        let pool = pk(1);
        let tx = tx_with_inner_instructions(
            10,
            vec![
                pool.to_string(),
                "11111111111111111111111111111111".to_string(),
            ],
            vec![compiled_group(0, 1, &[9, 9, 9])],
            None,
        );
        assert!(decode_transaction(&tx, &pool).is_empty());
    }

    #[test]
    fn test_decode_transaction_skips_failed_transactions() {
        let pool = pk(1);
        let tx = tx_with_inner_instructions(
            10,
            vec![pool.to_string(), dlmm_decode::ID.to_string()],
            vec![compiled_group(0, 1, &[9, 9, 9])],
            Some(solana_sdk::transaction::TransactionError::AccountNotFound),
        );
        assert!(decode_transaction(&tx, &pool).is_empty());
    }

    // Hand-encodes a real `Swap` self-CPI event payload (Anchor event tag + discriminator +
    // Borsh body, all little-endian, matching `dlmm_decode::events::wire::SwapWire`'s field
    // order) so the ix_index-combining and pool-filtering logic is exercised against an
    // actually decodable event, not just junk bytes.
    fn encode_swap_event(lb_pair: Pubkey, trader: Pubkey) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(dlmm_decode::EVENT_IX_TAG.as_slice());
        data.extend_from_slice(&dlmm_decode::discriminator("event", "Swap"));
        data.extend_from_slice(lb_pair.as_ref());
        data.extend_from_slice(trader.as_ref());
        data.extend_from_slice(&10i32.to_le_bytes()); // start_bin_id
        data.extend_from_slice(&12i32.to_le_bytes()); // end_bin_id
        data.extend_from_slice(&1_000u64.to_le_bytes()); // amount_in
        data.extend_from_slice(&900u64.to_le_bytes()); // amount_out
        data.push(1); // swap_for_y = true
        data.extend_from_slice(&30u64.to_le_bytes()); // fee
        data.extend_from_slice(&5u64.to_le_bytes()); // protocol_fee
        data.extend_from_slice(&3_000_000u128.to_le_bytes()); // fee_bps (FEE_PRECISION units)
        data.extend_from_slice(&1u64.to_le_bytes()); // host_fee
        data
    }

    #[test]
    fn test_decode_transaction_decodes_real_swap_and_computes_ix_index() {
        let pool = pk(1);
        let trader = pk(2);
        let payload = encode_swap_event(pool, trader);
        // Group index 2, second instruction (position 1) within it: 2 * 1000 + 1 = 2001.
        let group = solana_transaction_status_client_types::UiInnerInstructions {
            index: 2,
            instructions: vec![
                UiInstruction::Compiled(
                    solana_transaction_status_client_types::UiCompiledInstruction {
                        program_id_index: 1,
                        accounts: vec![],
                        data: bs58::encode([1, 2, 3]).into_string(),
                        stack_height: None,
                    },
                ),
                UiInstruction::Compiled(
                    solana_transaction_status_client_types::UiCompiledInstruction {
                        program_id_index: 1,
                        accounts: vec![],
                        data: bs58::encode(&payload).into_string(),
                        stack_height: None,
                    },
                ),
            ],
        };
        let tx = tx_with_inner_instructions(
            10,
            vec![pool.to_string(), dlmm_decode::ID.to_string()],
            vec![group],
            None,
        );

        let events = decode_transaction(&tx, &pool);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ix_index, 2_001);
        match &events[0].event {
            DecodedEvent::Swap(swap) => {
                assert_eq!(swap.lb_pair, pool);
                assert_eq!(swap.trader, trader);
                assert_eq!(swap.amount_in, 1_000);
                assert_eq!(swap.lp_fee, 25);
                assert_eq!(swap.protocol_fee, 5);
            }
            other => panic!("expected Swap, got {other:?}"),
        }
    }

    #[test]
    fn test_decode_transaction_filters_events_for_other_pools() {
        let pool = pk(1);
        let other_pool = pk(9);
        let payload = encode_swap_event(other_pool, pk(2));
        let tx = tx_with_inner_instructions(
            10,
            vec![pool.to_string(), dlmm_decode::ID.to_string()],
            vec![compiled_group(0, 1, &payload)],
            None,
        );
        assert!(decode_transaction(&tx, &pool).is_empty());
    }

    #[test]
    fn test_event_pool_is_none_for_position_close() {
        assert_eq!(
            event_pool(&DecodedEvent::PositionClose(
                dlmm_decode::DecodedPositionClose {
                    position: pk(1),
                    owner: pk(2),
                }
            )),
            None
        );
    }
}
