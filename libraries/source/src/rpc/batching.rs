use solana_sdk::pubkey::Pubkey;

/// Keys accepted by one `getMultipleAccounts` call.
pub const MAX_KEYS_PER_CALL: usize = 100;

/// One slot of the cap is reserved for the Clock sysvar appended to every batch, so pool
/// data groups must fit within this many keys.
pub const MAX_POOL_KEYS_PER_BATCH: usize = MAX_KEYS_PER_CALL - 1;

/// The accounts that must be read at the same slot for one pool: the `LbPair` plus the
/// three `BinArray`s around its last-known active bin. Before the active bin is known (a
/// newly promoted pool) the group is just the `LbPair`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountGroup {
    pub pool: Pubkey,
    pub keys: Vec<Pubkey>,
}

impl AccountGroup {
    pub fn lb_pair_only(pool: Pubkey) -> Self {
        Self {
            pool,
            keys: vec![pool],
        }
    }

    pub fn with_bin_arrays(pool: Pubkey, bin_arrays: [Pubkey; 3]) -> Self {
        let mut keys = Vec::with_capacity(4);
        keys.push(pool);
        keys.extend(bin_arrays);
        Self { pool, keys }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Bin-pack whole groups into batches of at most `max_keys` keys each, never splitting a
/// group across a boundary. Reading an `LbPair` and its `BinArray`s in different batches --
/// so at different slots -- would produce an active-bin liquidity figure describing no
/// single moment in time. First-fit: a group joins the current batch if it fits, else it
/// opens a new one.
pub fn pack_groups(
    groups: Vec<AccountGroup>,
    max_keys: usize,
) -> eyre::Result<Vec<Vec<AccountGroup>>> {
    let mut batches: Vec<Vec<AccountGroup>> = Vec::new();
    let mut current: Vec<AccountGroup> = Vec::new();
    let mut current_len = 0usize;

    for group in groups {
        if group.len() > max_keys {
            eyre::bail!(
                "Account group for pool {} needs {} keys, exceeds the {max_keys}-key batch cap",
                group.pool,
                group.len()
            );
        }
        if current_len + group.len() > max_keys && !current.is_empty() {
            batches.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current_len += group.len();
        current.push(group);
    }
    if !current.is_empty() {
        batches.push(current);
    }

    Ok(batches)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn pool(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn full_group(seed: u8) -> AccountGroup {
        AccountGroup::with_bin_arrays(
            pool(seed),
            [
                pool(seed.wrapping_add(101)),
                pool(seed.wrapping_add(102)),
                pool(seed.wrapping_add(103)),
            ],
        )
    }

    #[test]
    fn test_single_pool_fits_one_batch() {
        let batches = pack_groups(vec![full_group(1)], MAX_POOL_KEYS_PER_BATCH).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[0][0].len(), 4);
    }

    #[test]
    fn test_24_pools_fit_in_a_single_batch() {
        let groups: Vec<_> = (0..24).map(full_group).collect();
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 24);
        let total_keys: usize = batches[0].iter().map(|g| g.len()).sum();
        // 24 groups * 4 keys = 96; plus the Clock key the caller appends, 97 <= 100.
        assert_eq!(total_keys, 96);
    }

    #[test]
    fn test_25_pools_spill_into_a_second_batch() {
        let groups: Vec<_> = (0..25).map(full_group).collect();
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 24);
        assert_eq!(batches[1].len(), 1);
    }

    #[test]
    fn test_100_pools_split_into_five_batches_of_24() {
        let groups: Vec<_> = (0..100).map(full_group).collect();
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();
        let sizes: Vec<usize> = batches.iter().map(|b| b.len()).collect();
        assert_eq!(sizes, vec![24, 24, 24, 24, 4]);
    }

    #[test]
    fn test_no_batch_ever_exceeds_the_key_cap_including_clock() {
        let groups: Vec<_> = (0..137u32).map(|i| full_group((i % 250) as u8)).collect();
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();
        for batch in &batches {
            let keys: usize = batch.iter().map(|g| g.len()).sum();
            assert!(
                keys < MAX_KEYS_PER_CALL,
                "batch of {keys} keys plus Clock exceeds cap"
            );
        }
    }

    #[test]
    fn test_a_group_is_never_split_across_batches() {
        let groups: Vec<_> = (0..53).map(full_group).collect();
        let expected_pools: HashSet<_> = groups.iter().map(|g| g.pool).collect();
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();

        let mut seen = HashSet::new();
        for batch in &batches {
            for group in batch {
                assert!(
                    seen.insert(group.pool),
                    "pool {} appeared in more than one batch",
                    group.pool
                );
                assert_eq!(group.len(), 4);
            }
        }
        assert_eq!(seen, expected_pools);
    }

    #[test]
    fn test_bootstrap_groups_of_one_key_pack_densely() {
        // A newly promoted pool with no known active bin id contributes a 1-key group
        // (LbPair only) until the next tick learns where its bin arrays live.
        let groups: Vec<_> = (0..99u16)
            .map(|i| AccountGroup::lb_pair_only(pool((i % 250) as u8)))
            .collect();
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 99);
    }

    #[test]
    fn test_mixed_group_sizes_still_respect_the_cap() {
        let mut groups = Vec::new();
        for i in 0..20u8 {
            groups.push(full_group(i));
        }
        for i in 20..40u8 {
            groups.push(AccountGroup::lb_pair_only(pool(i)));
        }
        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH).unwrap();
        for batch in &batches {
            let keys: usize = batch.iter().map(|g| g.len()).sum();
            assert!(keys <= MAX_POOL_KEYS_PER_BATCH);
        }
        let total_groups: usize = batches.iter().map(|b| b.len()).sum();
        assert_eq!(total_groups, 40);
    }

    #[test]
    fn test_group_larger_than_cap_is_rejected() {
        let oversized = AccountGroup {
            pool: pool(1),
            keys: vec![pool(1); MAX_POOL_KEYS_PER_BATCH + 1],
        };
        assert!(pack_groups(vec![oversized], MAX_POOL_KEYS_PER_BATCH).is_err());
    }

    #[test]
    fn test_empty_input_produces_no_batches() {
        let batches = pack_groups(Vec::new(), MAX_POOL_KEYS_PER_BATCH).unwrap();
        assert!(batches.is_empty());
    }
}
