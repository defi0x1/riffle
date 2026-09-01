use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::args::RemainingAccountsInfo;
use crate::compute_budget::ComputeBudgetConfig;
use crate::error::DlmmTxError;
use crate::pda;

#[derive(Clone, Copy, Debug)]
pub struct RemoveLiquidityByRangeParams {
    pub lb_pair: Pubkey,
    pub position: Pubkey,
    pub position_lower_bin_id: i32,
    pub position_upper_bin_id: i32,
    pub owner: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub from_bin_id: i32,
    pub to_bin_id: i32,
    /// Basis points of the liquidity in range to withdraw, 1..=10000. 10000 empties the range.
    pub bps_to_remove: u16,
}

/// Builds `remove_liquidity_by_range2`. Chosen over `remove_liquidity`/`remove_liquidity2` (which
/// take an explicit per-bin `Vec<BinLiquidityReduction>`) and over pulling everything with
/// `remove_all_liquidity`: a fee-farming position that rebalances around the active bin needs to
/// pull specific edge bins without touching the rest, which a range plus a bps fraction expresses
/// directly. `remove_liquidity_by_range` (without the `2`) is the same idea but forces plain SPL
/// Token mints and lacks the `memo_program` account some RPC providers now require on withdrawal
/// instructions; `2` costs nothing for a non-Token-2022 pool.
pub fn build_remove_liquidity_by_range(
    params: &RemoveLiquidityByRangeParams,
    compute_budget: &ComputeBudgetConfig,
) -> Result<Vec<Instruction>, DlmmTxError> {
    if params.from_bin_id > params.to_bin_id {
        return Err(DlmmTxError::InvertedBinRange {
            from: params.from_bin_id,
            to: params.to_bin_id,
        });
    }
    if params.bps_to_remove == 0 || params.bps_to_remove > 10_000 {
        return Err(DlmmTxError::BpsOutOfRange {
            bps: params.bps_to_remove,
        });
    }
    if params.from_bin_id < params.position_lower_bin_id
        || params.to_bin_id > params.position_upper_bin_id
    {
        return Err(DlmmTxError::RangeExceedsPosition {
            from: params.from_bin_id,
            to: params.to_bin_id,
            lower: params.position_lower_bin_id,
            upper: params.position_upper_bin_id,
        });
    }

    let user_token_x =
        pda::associated_token_address(&params.owner, &params.token_x_mint, &params.token_x_program);
    let user_token_y =
        pda::associated_token_address(&params.owner, &params.token_y_mint, &params.token_y_program);
    let reserve_x = pda::reserve(&params.lb_pair, &params.token_x_mint);
    let reserve_y = pda::reserve(&params.lb_pair, &params.token_y_mint);
    let bitmap_extension = pda::optional_bin_array_bitmap_extension(
        &params.lb_pair,
        params.from_bin_id,
        params.to_bin_id,
    );
    let bin_arrays =
        pda::bin_arrays_covering_range(&params.lb_pair, params.from_bin_id, params.to_bin_id);

    let mut accounts = vec![
        AccountMeta::new(params.position, false),
        AccountMeta::new(params.lb_pair, false),
        AccountMeta::new(bitmap_extension, false),
        AccountMeta::new(user_token_x, false),
        AccountMeta::new(user_token_y, false),
        AccountMeta::new(reserve_x, false),
        AccountMeta::new(reserve_y, false),
        AccountMeta::new_readonly(params.token_x_mint, false),
        AccountMeta::new_readonly(params.token_y_mint, false),
        AccountMeta::new_readonly(params.owner, true),
        AccountMeta::new_readonly(params.token_x_program, false),
        AccountMeta::new_readonly(params.token_y_program, false),
        AccountMeta::new_readonly(pda::MEMO_PROGRAM_ID, false),
        AccountMeta::new_readonly(pda::event_authority(), false),
        AccountMeta::new_readonly(dlmm_decode::ID, false),
    ];
    accounts.extend(bin_arrays.into_iter().map(|pk| AccountMeta::new(pk, false)));

    let mut data = dlmm_decode::discriminator("global", "remove_liquidity_by_range2").to_vec();
    data.extend(borsh::to_vec(&params.from_bin_id).expect("borsh serialisation cannot fail"));
    data.extend(borsh::to_vec(&params.to_bin_id).expect("borsh serialisation cannot fail"));
    data.extend(borsh::to_vec(&params.bps_to_remove).expect("borsh serialisation cannot fail"));
    data.extend(
        borsh::to_vec(&RemainingAccountsInfo::none()).expect("borsh serialisation cannot fail"),
    );

    let mut ixs = compute_budget.instructions();
    ixs.push(Instruction {
        program_id: dlmm_decode::ID,
        accounts,
        data,
    });
    Ok(ixs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{assert_matches_idl, pubkey};

    fn params() -> RemoveLiquidityByRangeParams {
        RemoveLiquidityByRangeParams {
            lb_pair: pubkey(1),
            position: pubkey(2),
            position_lower_bin_id: -70,
            position_upper_bin_id: 70,
            owner: pubkey(3),
            token_x_mint: pubkey(4),
            token_y_mint: pubkey(5),
            token_x_program: pda::TOKEN_PROGRAM_ID,
            token_y_program: pda::TOKEN_PROGRAM_ID,
            from_bin_id: -10,
            to_bin_id: 10,
            bps_to_remove: 5_000,
        }
    }

    #[test]
    fn test_matches_idl_discriminator_and_account_flags() {
        let ix =
            &build_remove_liquidity_by_range(&params(), &ComputeBudgetConfig::none()).unwrap()[0];
        assert_matches_idl(ix, "remove_liquidity_by_range2");
    }

    #[test]
    fn test_memo_program_is_the_well_known_address() {
        let ix =
            &build_remove_liquidity_by_range(&params(), &ComputeBudgetConfig::none()).unwrap()[0];
        assert_eq!(ix.accounts[12].pubkey, pda::MEMO_PROGRAM_ID);
    }

    #[test]
    fn test_remaining_accounts_are_bin_arrays_covering_the_removal_range() {
        let params = params();
        let ix =
            &build_remove_liquidity_by_range(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        let expected =
            pda::bin_arrays_covering_range(&params.lb_pair, params.from_bin_id, params.to_bin_id);
        let trailing: Vec<Pubkey> = ix.accounts[15..].iter().map(|a| a.pubkey).collect();
        assert_eq!(trailing, expected);
    }

    #[test]
    fn test_inverted_range_rejected() {
        let mut params = params();
        params.from_bin_id = 10;
        params.to_bin_id = -10;
        assert!(matches!(
            build_remove_liquidity_by_range(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::InvertedBinRange { .. })
        ));
    }

    #[test]
    fn test_zero_bps_rejected() {
        let mut params = params();
        params.bps_to_remove = 0;
        assert!(matches!(
            build_remove_liquidity_by_range(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::BpsOutOfRange { .. })
        ));
    }

    #[test]
    fn test_bps_above_max_rejected() {
        let mut params = params();
        params.bps_to_remove = 10_001;
        assert!(matches!(
            build_remove_liquidity_by_range(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::BpsOutOfRange { .. })
        ));
    }

    #[test]
    fn test_full_withdrawal_at_max_bps_is_accepted() {
        let mut params = params();
        params.bps_to_remove = 10_000;
        assert!(build_remove_liquidity_by_range(&params, &ComputeBudgetConfig::none()).is_ok());
    }

    #[test]
    fn test_args_round_trip() {
        let params = params();
        let ix =
            &build_remove_liquidity_by_range(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        let from = i32::from_le_bytes(ix.data[8..12].try_into().unwrap());
        let to = i32::from_le_bytes(ix.data[12..16].try_into().unwrap());
        let bps = u16::from_le_bytes(ix.data[16..18].try_into().unwrap());
        assert_eq!(from, params.from_bin_id);
        assert_eq!(to, params.to_bin_id);
        assert_eq!(bps, params.bps_to_remove);
    }
}
