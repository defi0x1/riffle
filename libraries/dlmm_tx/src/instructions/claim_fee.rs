use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::args::RemainingAccountsInfo;
use crate::compute_budget::ComputeBudgetConfig;
use crate::error::DlmmTxError;
use crate::pda;

#[derive(Clone, Copy, Debug)]
pub struct ClaimFeeParams {
    pub lb_pair: Pubkey,
    pub position: Pubkey,
    pub position_lower_bin_id: i32,
    pub position_upper_bin_id: i32,
    pub owner: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    /// The bin range to sweep accrued fees from -- ordinarily the position's own
    /// `position_lower_bin_id`/`position_upper_bin_id`, since there is rarely a reason to leave
    /// fees uncollected on some bins and not others.
    pub min_bin_id: i32,
    pub max_bin_id: i32,
}

/// Builds `claim_fee2`. Chosen over `claim_fee` because `claim_fee2` sweeps the fees accrued
/// across an arbitrary `[min_bin_id, max_bin_id]` range in one instruction, while `claim_fee`
/// only reads two fixed bin arrays (`bin_array_lower`/`bin_array_upper`) -- fine for a position
/// no wider than two arrays, but a fee-farming position that grew past that via
/// `increase_position_length` needs the range form to collect everywhere it holds liquidity.
pub fn build_claim_fee(
    params: &ClaimFeeParams,
    compute_budget: &ComputeBudgetConfig,
) -> Result<Vec<Instruction>, DlmmTxError> {
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
    let bin_arrays =
        pda::bin_arrays_covering_range(&params.lb_pair, params.min_bin_id, params.max_bin_id);

    let mut accounts = vec![
        AccountMeta::new(params.lb_pair, false),
        AccountMeta::new(params.position, false),
        AccountMeta::new_readonly(params.owner, true),
        AccountMeta::new(reserve_x, false),
        AccountMeta::new(reserve_y, false),
        AccountMeta::new(user_token_x, false),
        AccountMeta::new(user_token_y, false),
        AccountMeta::new_readonly(params.token_x_mint, false),
        AccountMeta::new_readonly(params.token_y_mint, false),
        AccountMeta::new_readonly(params.token_x_program, false),
        AccountMeta::new_readonly(params.token_y_program, false),
        AccountMeta::new_readonly(pda::MEMO_PROGRAM_ID, false),
        AccountMeta::new_readonly(pda::event_authority(), false),
        AccountMeta::new_readonly(dlmm_decode::ID, false),
    ];
    accounts.extend(bin_arrays.into_iter().map(|pk| AccountMeta::new(pk, false)));

    let mut data = dlmm_decode::discriminator("global", "claim_fee2").to_vec();
    data.extend(borsh::to_vec(&params.min_bin_id).expect("borsh serialisation cannot fail"));
    data.extend(borsh::to_vec(&params.max_bin_id).expect("borsh serialisation cannot fail"));
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

    fn params() -> ClaimFeeParams {
        ClaimFeeParams {
            lb_pair: pubkey(1),
            position: pubkey(2),
            position_lower_bin_id: -70,
            position_upper_bin_id: 70,
            owner: pubkey(3),
            token_x_mint: pubkey(4),
            token_y_mint: pubkey(5),
            token_x_program: pda::TOKEN_PROGRAM_ID,
            token_y_program: pda::TOKEN_PROGRAM_ID,
            min_bin_id: -70,
            max_bin_id: 70,
        }
    }

    #[test]
    fn test_matches_idl_discriminator_and_account_flags() {
        let ix = &build_claim_fee(&params(), &ComputeBudgetConfig::none()).unwrap()[0];
        assert_matches_idl(ix, "claim_fee2");
    }

    #[test]
    fn test_named_accounts_are_correctly_derived() {
        let params = params();
        let ix = &build_claim_fee(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        assert_eq!(ix.accounts[0].pubkey, params.lb_pair);
        assert_eq!(ix.accounts[1].pubkey, params.position);
        assert_eq!(ix.accounts[2].pubkey, params.owner);
        assert_eq!(
            ix.accounts[3].pubkey,
            pda::reserve(&params.lb_pair, &params.token_x_mint)
        );
        assert_eq!(
            ix.accounts[4].pubkey,
            pda::reserve(&params.lb_pair, &params.token_y_mint)
        );
        assert_eq!(
            ix.accounts[5].pubkey,
            pda::associated_token_address(
                &params.owner,
                &params.token_x_mint,
                &params.token_x_program
            )
        );
        assert_eq!(
            ix.accounts[6].pubkey,
            pda::associated_token_address(
                &params.owner,
                &params.token_y_mint,
                &params.token_y_program
            )
        );
        assert_eq!(ix.accounts[11].pubkey, pda::MEMO_PROGRAM_ID);
        assert_eq!(ix.accounts[12].pubkey, pda::event_authority());
        assert_eq!(ix.accounts[13].pubkey, dlmm_decode::ID);
    }

    #[test]
    fn test_for_position_defaults_range_to_full_position() {
        let params = params();
        assert_eq!(params.min_bin_id, -70);
        assert_eq!(params.max_bin_id, 70);
    }

    #[test]
    fn test_inverted_range_rejected() {
        let mut params = params();
        params.min_bin_id = 10;
        params.max_bin_id = -10;
        assert!(matches!(
            build_claim_fee(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::InvertedBinRange { .. })
        ));
    }

    #[test]
    fn test_range_wider_than_position_rejected() {
        let mut params = params();
        params.max_bin_id = 200;
        assert!(matches!(
            build_claim_fee(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::RangeExceedsPosition { .. })
        ));
    }

    #[test]
    fn test_args_round_trip() {
        let params = params();
        let ix = &build_claim_fee(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        let min = i32::from_le_bytes(ix.data[8..12].try_into().unwrap());
        let max = i32::from_le_bytes(ix.data[12..16].try_into().unwrap());
        assert_eq!(min, params.min_bin_id);
        assert_eq!(max, params.max_bin_id);
    }
}
