use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::compute_budget::ComputeBudgetConfig;
use crate::pda;

#[derive(Clone, Copy, Debug)]
pub struct ClosePositionParams {
    pub position: Pubkey,
    /// The position's own owner -- the program checks `sender == position.owner` on chain.
    pub owner: Pubkey,
    pub rent_receiver: Pubkey,
}

/// Builds `close_position2`. Chosen over `close_position` because the latter also demands
/// `lb_pair` and the position's two bin array accounts even though closing never touches bin
/// data -- v2 dropped them. Both require the position to already hold zero liquidity and zero
/// pending fees; this crate does not check that on chain, so call `build_remove_liquidity_by_range`
/// and `build_claim_fee` for the position's full range first.
pub fn build_close_position(
    params: &ClosePositionParams,
    compute_budget: &ComputeBudgetConfig,
) -> Vec<Instruction> {
    let accounts = vec![
        AccountMeta::new(params.position, false),
        AccountMeta::new_readonly(params.owner, true),
        AccountMeta::new(params.rent_receiver, false),
        AccountMeta::new_readonly(pda::event_authority(), false),
        AccountMeta::new_readonly(dlmm_decode::ID, false),
    ];

    let data = dlmm_decode::discriminator("global", "close_position2").to_vec();

    let mut ixs = compute_budget.instructions();
    ixs.push(Instruction {
        program_id: dlmm_decode::ID,
        accounts,
        data,
    });
    ixs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{assert_matches_idl, pubkey};

    fn params() -> ClosePositionParams {
        ClosePositionParams {
            position: pubkey(1),
            owner: pubkey(2),
            rent_receiver: pubkey(3),
        }
    }

    #[test]
    fn test_matches_idl_discriminator_and_account_flags() {
        let ix = &build_close_position(&params(), &ComputeBudgetConfig::none())[0];
        assert_matches_idl(ix, "close_position2");
    }

    #[test]
    fn test_account_list_order_and_no_args() {
        let params = params();
        let ix = &build_close_position(&params, &ComputeBudgetConfig::none())[0];
        assert_eq!(ix.accounts[0].pubkey, params.position);
        assert_eq!(ix.accounts[1].pubkey, params.owner);
        assert_eq!(ix.accounts[2].pubkey, params.rent_receiver);
        assert_eq!(ix.accounts[3].pubkey, pda::event_authority());
        assert_eq!(ix.accounts[4].pubkey, dlmm_decode::ID);
        assert_eq!(ix.data.len(), 8);
    }
}
