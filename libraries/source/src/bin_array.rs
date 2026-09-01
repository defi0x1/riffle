use solana_sdk::pubkey::Pubkey;

/// The PDA of the BinArray at `index` for `lb_pair`. Shared by both backends so they can
/// never end up watching a different set of arrays for the same pool state.
pub(crate) fn bin_array_pda(lb_pair: &Pubkey, index: i64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            lb_clmm::utils::seeds::BIN_ARRAY,
            lb_pair.as_ref(),
            &index.to_le_bytes(),
        ],
        &lb_clmm::ID,
    )
    .0
}

/// Which BinArray a bin id falls in.
pub(crate) fn bin_array_index(active_bin_id: i32) -> i64 {
    // Only fails on i32 overflow at the extreme edge of the bin id range, which real pools
    // never reach; falling back to array 0 there is harmless since the next update corrects it.
    lb_clmm::state::bin::BinArray::bin_id_to_bin_array_index(active_bin_id).unwrap_or(0) as i64
}

/// The three BinArrays that matter for a pool's active bin: the one containing it plus its
/// immediate neighbours either side. That is enough slack to keep covering the active bin
/// across the normal gap between state updates.
pub(crate) fn surrounding_bin_arrays(lb_pair: &Pubkey, active_bin_id: i32) -> [Pubkey; 3] {
    let center = bin_array_index(active_bin_id);
    [
        bin_array_pda(lb_pair, center - 1),
        bin_array_pda(lb_pair, center),
        bin_array_pda(lb_pair, center + 1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bin_array_pda_is_deterministic_and_distinct_per_index() {
        let pool = Pubkey::new_from_array([1; 32]);
        assert_eq!(bin_array_pda(&pool, 5), bin_array_pda(&pool, 5));
        assert_ne!(bin_array_pda(&pool, 5), bin_array_pda(&pool, 6));
    }

    #[test]
    fn test_surrounding_bin_arrays_are_centered_on_the_active_bin_array() {
        let pool = Pubkey::new_from_array([7; 32]);
        let center = bin_array_index(0);
        let arrays = surrounding_bin_arrays(&pool, 0);
        assert_eq!(
            arrays,
            [
                bin_array_pda(&pool, center - 1),
                bin_array_pda(&pool, center),
                bin_array_pda(&pool, center + 1),
            ]
        );
    }
}
