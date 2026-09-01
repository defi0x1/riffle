use std::collections::HashMap;

use solana_sdk::pubkey::Pubkey;
use yellowstone_grpc_proto::prelude::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts,
    SubscribeRequestFilterAccountsFilter, SubscribeRequestFilterAccountsFilterMemcmp,
    SubscribeRequestFilterBlocksMeta, SubscribeRequestFilterSlots,
    SubscribeRequestFilterTransactions,
    subscribe_request_filter_accounts_filter::Filter as AccountsFilterKind,
    subscribe_request_filter_accounts_filter_memcmp::Data as MemcmpData,
};

// Named filter keys. Yellowstone unions matches across the entries in a map (an account
// hits the subscription if it matches any named filter), but ANDs the conditions inside
// one entry -- so LbPair and BinArray, which never share a discriminator, each need their
// own named filter rather than two memcmps on one entry (which would match nothing).
pub const LB_PAIR_FILTER: &str = "lb_pair";
pub const BIN_ARRAY_FILTER: &str = "bin_array";
pub const SWAPS_FILTER: &str = "swaps";
pub const HEALTH_SLOTS_FILTER: &str = "health";
pub const BLOCK_META_FILTER: &str = "meta";

pub fn parse_commitment(value: &str) -> eyre::Result<CommitmentLevel> {
    match value.to_ascii_lowercase().as_str() {
        "processed" => Ok(CommitmentLevel::Processed),
        "confirmed" => Ok(CommitmentLevel::Confirmed),
        "finalized" => Ok(CommitmentLevel::Finalized),
        other => eyre::bail!("Unknown Geyser commitment level \"{other}\""),
    }
}

fn discriminator_filter(discriminator: &[u8]) -> SubscribeRequestFilterAccountsFilter {
    SubscribeRequestFilterAccountsFilter {
        filter: Some(AccountsFilterKind::Memcmp(
            SubscribeRequestFilterAccountsFilterMemcmp {
                offset: 0,
                data: Some(MemcmpData::Bytes(discriminator.to_vec())),
            },
        )),
    }
}

// Scoped to the watched pool set directly: we already know these addresses, so there is no
// reason to pay for the whole universe's LbPair traffic.
fn lb_pair_account_filter(pools: &[Pubkey]) -> SubscribeRequestFilterAccounts {
    SubscribeRequestFilterAccounts {
        account: pools.iter().map(Pubkey::to_string).collect(),
        owner: vec![lb_clmm::ID.to_string()],
        filters: vec![discriminator_filter(
            dlmm_decode::LB_PAIR_DISCRIMINATOR.as_slice(),
        )],
        nonempty_txn_signature: None,
    }
}

// BinArray addresses are PDAs derived from a pool's active bin, which we don't know ahead
// of subscribing, so this can't be scoped by explicit account list the way LbPair is. It
// matches every BinArray on the program instead; the watched set is applied client-side
// once the account is decoded and its owning pool is known.
fn bin_array_account_filter() -> SubscribeRequestFilterAccounts {
    SubscribeRequestFilterAccounts {
        account: Vec::new(),
        owner: vec![lb_clmm::ID.to_string()],
        filters: vec![discriminator_filter(
            dlmm_decode::BIN_ARRAY_DISCRIMINATOR.as_slice(),
        )],
        nonempty_txn_signature: None,
    }
}

fn broad_transactions_filter() -> SubscribeRequestFilterTransactions {
    SubscribeRequestFilterTransactions {
        vote: Some(false),
        failed: Some(false),
        signature: None,
        account_include: vec![lb_clmm::ID.to_string()],
        account_exclude: Vec::new(),
        account_required: Vec::new(),
    }
}

// filter_by_commitment keeps this to one notification per slot at our chosen commitment
// level, rather than one per intermediate status -- all we need to track the high-water
// mark and notice a skipped slot.
fn health_slots_filter() -> SubscribeRequestFilterSlots {
    SubscribeRequestFilterSlots {
        filter_by_commitment: Some(true),
        interslot_updates: Some(false),
    }
}

pub fn state_subscribe_request(
    pools: &[Pubkey],
    commitment: CommitmentLevel,
    from_slot: Option<u64>,
) -> SubscribeRequest {
    let mut accounts = HashMap::new();
    accounts.insert(LB_PAIR_FILTER.to_string(), lb_pair_account_filter(pools));
    accounts.insert(BIN_ARRAY_FILTER.to_string(), bin_array_account_filter());

    let mut slots = HashMap::new();
    slots.insert(HEALTH_SLOTS_FILTER.to_string(), health_slots_filter());

    SubscribeRequest {
        accounts,
        slots,
        commitment: Some(commitment as i32),
        from_slot,
        ..Default::default()
    }
}

pub fn event_subscribe_request(
    commitment: CommitmentLevel,
    from_slot: Option<u64>,
) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    transactions.insert(SWAPS_FILTER.to_string(), broad_transactions_filter());

    let mut blocks_meta = HashMap::new();
    blocks_meta.insert(
        BLOCK_META_FILTER.to_string(),
        SubscribeRequestFilterBlocksMeta {},
    );

    let mut slots = HashMap::new();
    slots.insert(HEALTH_SLOTS_FILTER.to_string(), health_slots_filter());

    SubscribeRequest {
        transactions,
        blocks_meta,
        slots,
        commitment: Some(commitment as i32),
        from_slot,
        ..Default::default()
    }
}

// The startup pool scan. Subscribing to every LbPair without an explicit account list
// makes Geyser replay its full current snapshot of matching accounts before live updates
// begin, which stands in for a one-time getProgramAccounts scan without needing an RPC
// endpoint in this backend's config.
pub fn discovery_subscribe_request(commitment: CommitmentLevel) -> SubscribeRequest {
    let mut accounts = HashMap::new();
    accounts.insert(LB_PAIR_FILTER.to_string(), lb_pair_account_filter(&[]));

    SubscribeRequest {
        accounts,
        commitment: Some(commitment as i32),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    #[test]
    fn test_state_request_has_two_account_filters() {
        let req = state_subscribe_request(&[pool(1), pool(2)], CommitmentLevel::Confirmed, None);
        assert_eq!(req.accounts.len(), 2);
        assert!(req.accounts.contains_key(LB_PAIR_FILTER));
        assert!(req.accounts.contains_key(BIN_ARRAY_FILTER));
    }

    #[test]
    fn test_lb_pair_filter_scoped_to_watched_pools() {
        let req = state_subscribe_request(&[pool(1), pool(2)], CommitmentLevel::Confirmed, None);
        let lb_pair = &req.accounts[LB_PAIR_FILTER];
        assert_eq!(
            lb_pair.account,
            vec![pool(1).to_string(), pool(2).to_string()]
        );
        assert_eq!(lb_pair.owner, vec![lb_clmm::ID.to_string()]);
    }

    #[test]
    fn test_bin_array_filter_has_no_explicit_accounts() {
        let req = state_subscribe_request(&[pool(1)], CommitmentLevel::Confirmed, None);
        let bin_array = &req.accounts[BIN_ARRAY_FILTER];
        assert!(bin_array.account.is_empty());
        assert_eq!(bin_array.owner, vec![lb_clmm::ID.to_string()]);
    }

    #[test]
    fn test_each_account_filter_carries_its_own_discriminator_not_both() {
        let req = state_subscribe_request(&[pool(1)], CommitmentLevel::Confirmed, None);
        for name in [LB_PAIR_FILTER, BIN_ARRAY_FILTER] {
            assert_eq!(req.accounts[name].filters.len(), 1);
        }
    }

    #[test]
    fn test_event_request_filters_on_program_id() {
        let req = event_subscribe_request(CommitmentLevel::Confirmed, None);
        let swaps = &req.transactions[SWAPS_FILTER];
        assert_eq!(swaps.account_include, vec![lb_clmm::ID.to_string()]);
        assert_eq!(swaps.vote, Some(false));
        assert_eq!(swaps.failed, Some(false));
    }

    #[test]
    fn test_event_request_includes_block_meta_for_timestamps() {
        let req = event_subscribe_request(CommitmentLevel::Confirmed, None);
        assert!(req.blocks_meta.contains_key(BLOCK_META_FILTER));
    }

    #[test]
    fn test_from_slot_round_trips_for_reconnect_replay() {
        let req = state_subscribe_request(&[], CommitmentLevel::Confirmed, Some(12345));
        assert_eq!(req.from_slot, Some(12345));
        let fresh = state_subscribe_request(&[], CommitmentLevel::Confirmed, None);
        assert_eq!(fresh.from_slot, None);
    }

    #[test]
    fn test_discovery_request_has_no_pool_list() {
        let req = discovery_subscribe_request(CommitmentLevel::Confirmed);
        assert!(req.accounts[LB_PAIR_FILTER].account.is_empty());
    }

    #[test]
    fn test_parse_commitment_accepts_known_levels() {
        assert_eq!(
            parse_commitment("confirmed").unwrap(),
            CommitmentLevel::Confirmed
        );
        assert_eq!(
            parse_commitment("Processed").unwrap(),
            CommitmentLevel::Processed
        );
        assert_eq!(
            parse_commitment("FINALIZED").unwrap(),
            CommitmentLevel::Finalized
        );
        assert!(parse_commitment("nonsense").is_err());
    }
}
