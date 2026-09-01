//! Recovers the real on-chain amounts a confirmed remove-liquidity, add-liquidity or claim-fee
//! transaction moved, by fetching the transaction and decoding the DLMM self-CPI events it
//! emitted. The decode step (`dlmm_decode::decode_event` over each self-CPI inner instruction)
//! is the exact machinery `bin/indexer`'s event worker and `bin/crawler`'s `convert.rs` already
//! trust for the same job -- this module only adds the "fetch one transaction by signature and
//! pick out the event for one position" plumbing neither of those needs, since they walk many
//! transactions for many positions at once.
//!
//! What each action's event actually reports, once decoded:
//!   - `RemoveLiquidity`: the token amounts the withdrawal actually paid out, in `amounts`.
//!   - `AddLiquidity`: the token amounts the deposit actually pulled in, in `amounts` -- which
//!     can differ from what a strategy deposit requested, since the target bins bound what they
//!     accept.
//!   - `ClaimFee` / `ClaimFee2`: the fee amounts paid out to the position's owner, in
//!     `fee_x`/`fee_y`. Unlike `Swap`, this event carries no separate `protocol_fee` field to
//!     subtract -- see `claim_fee_amounts` below for why that is the correct reading, not an
//!     oversight.

use dlmm_decode::{DecodedEvent, decode_event};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiInstruction, UiMessage,
    UiTransactionEncoding, UiTransactionStatusMeta,
};

/// Fetches `signature`'s confirmed transaction with inner instructions included -- the only
/// response shape that carries the self-CPI event bytes this module decodes. `confirmed` rather
/// than `finalized`: by the time this is called the caller has already observed at least
/// `Confirmed` via `getSignatureStatuses`, so this asks for nothing stronger than what is
/// already known to have landed.
pub async fn fetch_transaction(
    rpc: &RpcClient,
    signature: &Signature,
) -> eyre::Result<EncodedConfirmedTransactionWithStatusMeta> {
    let config = RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::Json),
        commitment: Some(CommitmentConfig::confirmed()),
        max_supported_transaction_version: Some(0),
    };
    rpc.get_transaction_with_config(signature, config)
        .await
        .map_err(|e| eyre::eyre!("Fetching confirmed transaction {signature}: {e}"))
}

/// The account keys a compiled instruction's `program_id_index` indexes into: the message's own
/// keys, followed by any address-lookup-table keys resolved for this transaction. Identical to
/// `bin/crawler/src/convert.rs`'s helper of the same name -- kept local rather than shared
/// because that crate is a separate binary this one does not depend on.
fn account_keys(meta: &UiTransactionStatusMeta, message_keys: &[String]) -> Vec<String> {
    let mut keys = message_keys.to_vec();
    if let Some(loaded) = Option::from(meta.loaded_addresses.clone()) {
        let loaded: solana_transaction_status_client_types::UiLoadedAddresses = loaded;
        keys.extend(loaded.writable);
        keys.extend(loaded.readonly);
    }
    keys
}

/// Every DLMM self-CPI event emitted anywhere in `tx`, in emission order. Empty for a failed
/// transaction (its inner instructions never committed) or a response shape this cannot read --
/// both are ordinary "nothing to decode" outcomes for the caller, not distinguished further
/// here, since either way there is no event to recover an amount from.
pub fn decode_dlmm_events(tx: &EncodedConfirmedTransactionWithStatusMeta) -> Vec<DecodedEvent> {
    let mut events = Vec::new();

    let Some(meta) = &tx.transaction.meta else {
        return events;
    };
    if meta.err.is_some() {
        return events;
    }
    let EncodedTransaction::Json(ui_tx) = &tx.transaction.transaction else {
        return events;
    };
    let UiMessage::Raw(message) = &ui_tx.message else {
        return events;
    };
    let Some(inner_groups): Option<Vec<_>> = Option::from(meta.inner_instructions.clone()) else {
        return events;
    };

    let keys = account_keys(meta, &message.account_keys);
    let program_id = dlmm_decode::ID.to_string();

    for group in inner_groups {
        for instruction in &group.instructions {
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
            events.push(event);
        }
    }

    events
}

/// The two token amounts an event reported for one position, plus the active bin at the moment
/// it fired when the event carries one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredAmounts {
    pub amount_x_raw: u64,
    pub amount_y_raw: u64,
    pub active_bin_id: Option<i32>,
}

/// The token amounts a confirmed remove-liquidity transaction actually withdrew for `position`.
/// Read from `RemoveLiquidity`, never derived from the request: `bps_to_remove` only bounds what
/// gets pulled, it is not the executed amount.
pub fn remove_liquidity_amounts(
    events: &[DecodedEvent],
    position: &Pubkey,
) -> Option<RecoveredAmounts> {
    events.iter().find_map(|event| match event {
        DecodedEvent::RemoveLiquidity(liq) if liq.position == *position => Some(RecoveredAmounts {
            amount_x_raw: liq.amount_x,
            amount_y_raw: liq.amount_y,
            active_bin_id: Some(liq.active_bin_id),
        }),
        _ => None,
    })
}

/// The token amounts a confirmed add-liquidity transaction actually deposited for `position`.
/// Read from `AddLiquidity` rather than trusted from the request, since a strategy deposit is
/// bounded by what its target bins accept -- what actually moved can be less than what was
/// asked for.
pub fn add_liquidity_amounts(
    events: &[DecodedEvent],
    position: &Pubkey,
) -> Option<RecoveredAmounts> {
    events.iter().find_map(|event| match event {
        DecodedEvent::AddLiquidity(liq) if liq.position == *position => Some(RecoveredAmounts {
            amount_x_raw: liq.amount_x,
            amount_y_raw: liq.amount_y,
            active_bin_id: Some(liq.active_bin_id),
        }),
        _ => None,
    })
}

/// The fee amounts a confirmed claim-fee transaction actually paid out to `position`'s owner.
/// Read from `ClaimFee`/`ClaimFee2`'s `fee_x`/`fee_y` directly, with no further subtraction.
///
/// That is a deliberate reading of this system's established protocol-fee rule
/// (`dlmm_decode::events::decode::map_swap`'s `lp_fee = fee - protocol_fee`, applied to the
/// `Swap` event), not an oversight of it: that rule exists because `Swap`'s wire event carries
/// one `fee` field for the *whole* swap fee plus a separate `protocol_fee` field for the
/// protocol's cut of it, so the two must be subtracted to find what LPs earned. `ClaimFee` and
/// `ClaimFee2` carry no such second field -- their wire layout (mirrored in
/// `dlmm_decode::events::wire::ClaimFeeWire`/`ClaimFee2Wire`, and matching the program's
/// published Anchor IDL) is only `lb_pair`, `position`, `owner`, `fee_x`, `fee_y`. That is
/// consistent with how a position's fee entitlement is tracked in the first place: the
/// protocol's share is split off at swap time into the pool's own account, never credited into
/// any bin's or position's fee accumulator (the same accumulator `fee_x`/`fee_y` here reports
/// paying out), so there is nothing of the protocol's left in it to subtract. This service's own
/// pre-existing fee estimate shown before a claim is built (`rpc_ext::pending_fees_in_range`,
/// reading that same per-position accumulator) already treats it as the LP's own figure with no
/// discount, for the same reason.
pub fn claim_fee_amounts(events: &[DecodedEvent], position: &Pubkey) -> Option<RecoveredAmounts> {
    events.iter().find_map(|event| match event {
        DecodedEvent::ClaimFee(fee) if fee.position == *position => Some(RecoveredAmounts {
            amount_x_raw: fee.fee_x,
            amount_y_raw: fee.fee_y,
            active_bin_id: fee.active_bin_id,
        }),
        DecodedEvent::ClaimFee2(fee) if fee.position == *position => Some(RecoveredAmounts {
            amount_x_raw: fee.fee_x,
            amount_y_raw: fee.fee_y,
            active_bin_id: fee.active_bin_id,
        }),
        _ => None,
    })
}

/// The LP's own share of a decoded `Swap` event's fee: `dlmm_decode`'s own `lp_fee`
/// (`fee - protocol_fee`, computed once in `map_swap`), never the raw `fee` field, which
/// includes the protocol's cut. No action this module currently recovers amounts for emits a
/// `Swap` event -- `remove`/`add`/`claim` transactions each carry exactly one DLMM instruction,
/// none of which is a swap, and see `claim_fee_amounts`'s doc comment for why `ClaimFee` needs
/// no analogous treatment. Kept, not called from production code yet, so that a future caller
/// that does need a swap's LP-earned amount out of a decoded transaction reaches for the field
/// this codebase's established rule says is correct, instead of re-deriving the subtraction or
/// reaching for `fee` by mistake.
#[allow(dead_code)]
pub fn swap_lp_fee(swap: &dlmm_decode::DecodedSwap) -> u64 {
    swap.lp_fee
}

#[cfg(test)]
mod tests {
    use super::*;
    use dlmm_decode::{DecodedClaimFee, DecodedLiquidityEvent, LiquidityEventKind};
    use solana_sdk::pubkey::Pubkey;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    #[test]
    fn test_remove_liquidity_amounts_matches_on_position() {
        let position = pk(3);
        let events = vec![
            DecodedEvent::RemoveLiquidity(DecodedLiquidityEvent {
                kind: LiquidityEventKind::Remove,
                lb_pair: pk(1),
                from: pk(2),
                position: pk(9), // a different position -- must not match
                amount_x: 111,
                amount_y: 222,
                active_bin_id: 5,
            }),
            DecodedEvent::RemoveLiquidity(DecodedLiquidityEvent {
                kind: LiquidityEventKind::Remove,
                lb_pair: pk(1),
                from: pk(2),
                position,
                amount_x: 1_000,
                amount_y: 2_000,
                active_bin_id: -42,
            }),
        ];

        let recovered = remove_liquidity_amounts(&events, &position).expect("event present");
        assert_eq!(recovered.amount_x_raw, 1_000);
        assert_eq!(recovered.amount_y_raw, 2_000);
        assert_eq!(recovered.active_bin_id, Some(-42));
    }

    #[test]
    fn test_remove_liquidity_amounts_none_when_absent() {
        let position = pk(3);
        let events = vec![DecodedEvent::AddLiquidity(DecodedLiquidityEvent {
            kind: LiquidityEventKind::Add,
            lb_pair: pk(1),
            from: pk(2),
            position,
            amount_x: 1,
            amount_y: 2,
            active_bin_id: 0,
        })];

        assert!(remove_liquidity_amounts(&events, &position).is_none());
    }

    #[test]
    fn test_add_liquidity_amounts_reads_actual_deposit() {
        let position = pk(4);
        let events = vec![DecodedEvent::AddLiquidity(DecodedLiquidityEvent {
            kind: LiquidityEventKind::Add,
            lb_pair: pk(1),
            from: pk(2),
            position,
            amount_x: 500,
            amount_y: 750,
            active_bin_id: 12,
        })];

        let recovered = add_liquidity_amounts(&events, &position).expect("event present");
        assert_eq!(recovered.amount_x_raw, 500);
        assert_eq!(recovered.amount_y_raw, 750);
        assert_eq!(recovered.active_bin_id, Some(12));
    }

    #[test]
    fn test_claim_fee_amounts_uses_fee_fields_directly() {
        let position = pk(5);
        let events = vec![DecodedEvent::ClaimFee(DecodedClaimFee {
            lb_pair: pk(1),
            position,
            owner: pk(6),
            fee_x: 300,
            fee_y: 400,
            active_bin_id: None,
        })];

        let recovered = claim_fee_amounts(&events, &position).expect("event present");
        assert_eq!(recovered.amount_x_raw, 300);
        assert_eq!(recovered.amount_y_raw, 400);
        assert_eq!(recovered.active_bin_id, None);
    }

    #[test]
    fn test_claim_fee_amounts_prefers_claim_fee2_active_bin() {
        let position = pk(5);
        let events = vec![DecodedEvent::ClaimFee2(DecodedClaimFee {
            lb_pair: pk(1),
            position,
            owner: pk(6),
            fee_x: 30,
            fee_y: 40,
            active_bin_id: Some(-7),
        })];

        let recovered = claim_fee_amounts(&events, &position).expect("event present");
        assert_eq!(recovered.amount_x_raw, 30);
        assert_eq!(recovered.amount_y_raw, 40);
        assert_eq!(recovered.active_bin_id, Some(-7));
    }

    // From here down, the fixtures are raw self-CPI event bytes -- `EVENT_IX_TAG` plus the
    // per-event Anchor discriminator plus a Borsh-encoded payload built field-by-field, exactly
    // the pattern `libraries/dlmm_decode/tests/golden.rs` uses for events (that crate has no
    // live mainnet fixture for an event, only for accounts; see its own comment on why). Routing
    // through `dlmm_decode::decode_event` here, rather than constructing `DecodedEvent` values
    // directly as the tests above do, additionally proves this module's field assumptions
    // (`position` at the offset each selector reads) survive the real Borsh decode, not just
    // Rust struct literals this module wrote itself.
    fn event_bytes(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16 + payload.len());
        buf.extend_from_slice(dlmm_decode::EVENT_IX_TAG.as_slice());
        buf.extend_from_slice(&dlmm_decode::discriminator("event", name));
        buf.extend_from_slice(payload);
        buf
    }

    fn liquidity_payload(lb_pair: [u8; 32], from: [u8; 32], position: [u8; 32]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&lb_pair);
        payload.extend_from_slice(&from);
        payload.extend_from_slice(&position);
        payload.extend_from_slice(&1_500u64.to_le_bytes()); // amounts[0] (x)
        payload.extend_from_slice(&2_500u64.to_le_bytes()); // amounts[1] (y)
        payload.extend_from_slice(&(-8i32).to_le_bytes()); // active_bin_id
        payload
    }

    #[test]
    fn test_remove_liquidity_amounts_from_decoded_fixture_bytes() {
        let position = Pubkey::new_from_array([3u8; 32]);
        let bytes = event_bytes(
            "RemoveLiquidity",
            &liquidity_payload([1u8; 32], [2u8; 32], position.to_bytes()),
        );
        let event = decode_event(&bytes).expect("decoding constructed RemoveLiquidity event");

        let recovered =
            remove_liquidity_amounts(std::slice::from_ref(&event), &position).expect("matches");
        assert_eq!(recovered.amount_x_raw, 1_500);
        assert_eq!(recovered.amount_y_raw, 2_500);
        assert_eq!(recovered.active_bin_id, Some(-8));
    }

    #[test]
    fn test_add_liquidity_amounts_from_decoded_fixture_bytes() {
        let position = Pubkey::new_from_array([4u8; 32]);
        let bytes = event_bytes(
            "AddLiquidity",
            &liquidity_payload([1u8; 32], [2u8; 32], position.to_bytes()),
        );
        let event = decode_event(&bytes).expect("decoding constructed AddLiquidity event");

        let recovered =
            add_liquidity_amounts(std::slice::from_ref(&event), &position).expect("matches");
        assert_eq!(recovered.amount_x_raw, 1_500);
        assert_eq!(recovered.amount_y_raw, 2_500);
    }

    #[test]
    fn test_claim_fee_amounts_from_decoded_claim_fee2_fixture_bytes() {
        let position = Pubkey::new_from_array([5u8; 32]);
        let mut payload = Vec::new();
        payload.extend_from_slice(&[1u8; 32]); // lb_pair
        payload.extend_from_slice(&position.to_bytes());
        payload.extend_from_slice(&[6u8; 32]); // owner
        payload.extend_from_slice(&987u64.to_le_bytes()); // fee_x
        payload.extend_from_slice(&654u64.to_le_bytes()); // fee_y
        payload.extend_from_slice(&11i32.to_le_bytes()); // active_bin_id

        let bytes = event_bytes("ClaimFee2", &payload);
        let event = decode_event(&bytes).expect("decoding constructed ClaimFee2 event");

        // This is the case that would fail to reproduce `event.fee_x`/`fee_y` untouched if this
        // module wrongly re-derived a protocol-fee subtraction here: ClaimFee2's wire layout
        // has no protocol_fee field to subtract (see `claim_fee_amounts`'s doc comment), so the
        // recovered amount must equal the raw decoded field exactly.
        let recovered =
            claim_fee_amounts(std::slice::from_ref(&event), &position).expect("matches");
        assert_eq!(recovered.amount_x_raw, 987);
        assert_eq!(recovered.amount_y_raw, 654);
        assert_eq!(recovered.active_bin_id, Some(11));
    }

    // The protocol-fee-subtraction rule this codebase already established for Swap events
    // (`event.fee` includes the protocol's cut; LPs earn `fee - protocol_fee`), decoded from
    // constructed fixture bytes exactly like `dlmm_decode`'s own golden test for this
    // derivation. This is the case that would pass if `swap_lp_fee` returned the raw `fee`
    // field instead of `lp_fee`: fee=1_000, protocol_fee=100 would make a caller that used
    // `fee` directly overstate the LP's earnings by exactly the protocol's cut.
    #[test]
    fn test_swap_lp_fee_excludes_protocol_cut() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&[1u8; 32]); // lb_pair
        payload.extend_from_slice(&[2u8; 32]); // from
        payload.extend_from_slice(&(-10i32).to_le_bytes()); // start_bin_id
        payload.extend_from_slice(&(-12i32).to_le_bytes()); // end_bin_id
        payload.extend_from_slice(&10_000u64.to_le_bytes()); // amount_in
        payload.extend_from_slice(&9_000u64.to_le_bytes()); // amount_out
        payload.push(1); // swap_for_y
        payload.extend_from_slice(&1_000u64.to_le_bytes()); // fee (includes protocol_fee)
        payload.extend_from_slice(&100u64.to_le_bytes()); // protocol_fee
        payload.extend_from_slice(&1_000_000u128.to_le_bytes()); // fee_bps, FEE_PRECISION units
        payload.extend_from_slice(&0u64.to_le_bytes()); // host_fee

        let bytes = event_bytes("Swap", &payload);
        let DecodedEvent::Swap(swap) =
            decode_event(&bytes).expect("decoding constructed Swap event")
        else {
            panic!("expected Swap variant")
        };

        assert_eq!(swap_lp_fee(&swap), 900);
        assert_ne!(
            swap_lp_fee(&swap),
            swap.lp_fee + swap.protocol_fee,
            "must not report the protocol-inclusive raw fee as the LP's own"
        );
    }
}
