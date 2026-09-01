use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::args::{
    LiquidityParameterByStrategy, RemainingAccountsInfo, StrategyParameters, StrategyType,
};
use crate::compute_budget::ComputeBudgetConfig;
use crate::error::DlmmTxError;
use crate::pda;

/// Domain inputs for depositing into an already-open position. `position_lower_bin_id` /
/// `position_upper_bin_id` are the position's own range (from the account you opened it with, or
/// from decoding it), used only to check the deposit range actually fits inside it.
#[derive(Clone, Copy, Debug)]
pub struct AddLiquidityByStrategyParams {
    pub lb_pair: Pubkey,
    pub position: Pubkey,
    pub position_lower_bin_id: i32,
    pub position_upper_bin_id: i32,
    pub owner: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub amount_x: u64,
    pub amount_y: u64,
    /// The pool's active bin id as the caller last observed it off chain -- the program checks
    /// the live active bin is within `max_active_bin_slippage` of this before depositing.
    pub active_id: i32,
    pub max_active_bin_slippage: i32,
    pub strategy_type: StrategyType,
    /// Only read by the `*ImBalanced` strategy variants; ignored otherwise.
    pub favor_token_x: bool,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
}

/// Builds `add_liquidity_by_strategy2`. Chosen over the plain `add_liquidity`/`add_liquidity2`
/// (which take an explicit per-bin `Vec<BinLiquidityDistribution>` the caller would have to
/// compute by hand) and over `add_liquidity_by_weight2` (an even lower-level per-bin weight
/// array): a strategy plus a bin range is what a fee-farming tool wants to express -- "deposit
/// this much, spread it across this range, balanced around the active bin" -- and it's what the
/// on-chain program itself turns any weight array into internally. The `2` suffix over the
/// original `add_liquidity_by_strategy` buys Token-2022 mint support at no cost for plain SPL
/// Token mints (empty transfer-hook slices, see `RemainingAccountsInfo::none`).
pub fn build_add_liquidity_by_strategy(
    params: &AddLiquidityByStrategyParams,
    compute_budget: &ComputeBudgetConfig,
) -> Result<Vec<Instruction>, DlmmTxError> {
    if params.amount_x == 0 && params.amount_y == 0 {
        return Err(DlmmTxError::ZeroAmount);
    }
    if params.min_bin_id > params.max_bin_id {
        return Err(DlmmTxError::InvertedBinRange {
            from: params.min_bin_id,
            to: params.max_bin_id,
        });
    }
    if params.min_bin_id < params.position_lower_bin_id
        || params.max_bin_id > params.position_upper_bin_id
    {
        return Err(DlmmTxError::RangeExceedsPosition {
            from: params.min_bin_id,
            to: params.max_bin_id,
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
        params.min_bin_id,
        params.max_bin_id,
    );
    let bin_arrays =
        pda::bin_arrays_covering_range(&params.lb_pair, params.min_bin_id, params.max_bin_id);

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
        AccountMeta::new_readonly(pda::event_authority(), false),
        AccountMeta::new_readonly(dlmm_decode::ID, false),
    ];
    accounts.extend(bin_arrays.into_iter().map(|pk| AccountMeta::new(pk, false)));

    let liquidity_parameter = LiquidityParameterByStrategy {
        amount_x: params.amount_x,
        amount_y: params.amount_y,
        active_id: params.active_id,
        max_active_bin_slippage: params.max_active_bin_slippage,
        strategy_parameters: StrategyParameters::new(
            params.min_bin_id,
            params.max_bin_id,
            params.strategy_type,
            params.favor_token_x,
        ),
    };

    let mut data = dlmm_decode::discriminator("global", "add_liquidity_by_strategy2").to_vec();
    data.extend(borsh::to_vec(&liquidity_parameter).expect("borsh serialisation cannot fail"));
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

    fn params() -> AddLiquidityByStrategyParams {
        AddLiquidityByStrategyParams {
            lb_pair: pubkey(1),
            position: pubkey(2),
            position_lower_bin_id: -70,
            position_upper_bin_id: 70,
            owner: pubkey(3),
            token_x_mint: pubkey(4),
            token_y_mint: pubkey(5),
            token_x_program: pda::TOKEN_PROGRAM_ID,
            token_y_program: pda::TOKEN_PROGRAM_ID,
            amount_x: 1_000_000,
            amount_y: 2_000_000,
            active_id: 0,
            max_active_bin_slippage: 5,
            strategy_type: StrategyType::SpotBalanced,
            favor_token_x: false,
            min_bin_id: -10,
            max_bin_id: 10,
        }
    }

    #[test]
    fn test_matches_idl_discriminator_and_account_flags() {
        let ix =
            &build_add_liquidity_by_strategy(&params(), &ComputeBudgetConfig::none()).unwrap()[0];
        assert_matches_idl(ix, "add_liquidity_by_strategy2");
    }

    #[test]
    fn test_named_accounts_are_correctly_derived() {
        let params = params();
        let ix =
            &build_add_liquidity_by_strategy(&params, &ComputeBudgetConfig::none()).unwrap()[0];

        assert_eq!(ix.accounts[0].pubkey, params.position);
        assert_eq!(ix.accounts[1].pubkey, params.lb_pair);
        assert_eq!(ix.accounts[2].pubkey, dlmm_decode::ID); // bitmap extension not needed
        assert_eq!(
            ix.accounts[3].pubkey,
            pda::associated_token_address(
                &params.owner,
                &params.token_x_mint,
                &params.token_x_program
            )
        );
        assert_eq!(
            ix.accounts[4].pubkey,
            pda::associated_token_address(
                &params.owner,
                &params.token_y_mint,
                &params.token_y_program
            )
        );
        assert_eq!(
            ix.accounts[5].pubkey,
            pda::reserve(&params.lb_pair, &params.token_x_mint)
        );
        assert_eq!(
            ix.accounts[6].pubkey,
            pda::reserve(&params.lb_pair, &params.token_y_mint)
        );
        assert_eq!(ix.accounts[7].pubkey, params.token_x_mint);
        assert_eq!(ix.accounts[8].pubkey, params.token_y_mint);
        assert_eq!(ix.accounts[9].pubkey, params.owner);
        assert_eq!(ix.accounts[10].pubkey, params.token_x_program);
        assert_eq!(ix.accounts[11].pubkey, params.token_y_program);
        assert_eq!(ix.accounts[12].pubkey, pda::event_authority());
        assert_eq!(ix.accounts[13].pubkey, dlmm_decode::ID);
    }

    #[test]
    fn test_remaining_accounts_are_bin_arrays_covering_the_strategy_range() {
        let params = params();
        let ix =
            &build_add_liquidity_by_strategy(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        let expected =
            pda::bin_arrays_covering_range(&params.lb_pair, params.min_bin_id, params.max_bin_id);
        let trailing: Vec<Pubkey> = ix.accounts[14..].iter().map(|a| a.pubkey).collect();
        assert_eq!(trailing, expected);
        assert!(
            ix.accounts[14..]
                .iter()
                .all(|a| a.is_writable && !a.is_signer)
        );
    }

    #[test]
    fn test_zero_amount_rejected() {
        let mut params = params();
        params.amount_x = 0;
        params.amount_y = 0;
        assert!(matches!(
            build_add_liquidity_by_strategy(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::ZeroAmount)
        ));
    }

    #[test]
    fn test_inverted_range_rejected() {
        let mut params = params();
        params.min_bin_id = 10;
        params.max_bin_id = -10;
        assert!(matches!(
            build_add_liquidity_by_strategy(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::InvertedBinRange { .. })
        ));
    }

    #[test]
    fn test_range_wider_than_position_rejected() {
        let mut params = params();
        params.min_bin_id = -100;
        assert!(matches!(
            build_add_liquidity_by_strategy(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::RangeExceedsPosition { .. })
        ));
    }

    #[test]
    fn test_args_round_trip() {
        let params = params();
        let ix =
            &build_add_liquidity_by_strategy(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        // discriminator(8) + LiquidityParameterByStrategy(8+8+4+4+ (4+4+1+64)) + RemainingAccountsInfo(4 + 2*2)
        assert_eq!(ix.data.len(), 8 + 24 + 73 + 8);
        assert_eq!(&ix.data[8..16], &params.amount_x.to_le_bytes());
        assert_eq!(&ix.data[16..24], &params.amount_y.to_le_bytes());
    }
}
