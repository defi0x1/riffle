use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::compute_budget::ComputeBudgetConfig;
use crate::error::DlmmTxError;
use crate::pda;

/// A position holds at most `MAX_BIN_PER_ARRAY` bins at open time; widening it past that needs a
/// separate `increase_position_length` instruction this crate does not build.
const MAX_INITIAL_WIDTH: i32 = dlmm_decode::MAX_BIN_PER_ARRAY as i32;

/// Domain inputs for opening a position. `position` is a fresh pubkey the caller (the Mini App,
/// on the user's device) generates and will co-sign the built transaction with -- this crate
/// only ever sees the public half of it.
#[derive(Clone, Copy, Debug)]
pub struct OpenPositionParams {
    pub lb_pair: Pubkey,
    pub owner: Pubkey,
    pub payer: Pubkey,
    pub position: Pubkey,
    pub lower_bin_id: i32,
    pub width: i32,
}

/// Builds `initialize_position2`. Chosen over `initialize_position` because it drops the
/// deprecated `rent` sysvar account the runtime no longer needs, and over `initialize_position_pda`
/// because that variant trades one required signer (`position`) for another (`base`, a second
/// throwaway keypair used only as a PDA seed) without simplifying anything for a keyless backend.
pub fn build_open_position(
    params: &OpenPositionParams,
    compute_budget: &ComputeBudgetConfig,
) -> Result<Vec<Instruction>, DlmmTxError> {
    if params.width < 1 || params.width > MAX_INITIAL_WIDTH {
        return Err(DlmmTxError::WidthOutOfRange {
            width: params.width,
            max: MAX_INITIAL_WIDTH,
        });
    }

    let accounts = vec![
        AccountMeta::new(params.payer, true),
        AccountMeta::new(params.position, true),
        AccountMeta::new_readonly(params.lb_pair, false),
        AccountMeta::new_readonly(params.owner, true),
        AccountMeta::new_readonly(pda::SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(pda::event_authority(), false),
        AccountMeta::new_readonly(dlmm_decode::ID, false),
    ];

    let mut data = dlmm_decode::discriminator("global", "initialize_position2").to_vec();
    data.extend(borsh::to_vec(&params.lower_bin_id).expect("i32 serialisation cannot fail"));
    data.extend(borsh::to_vec(&params.width).expect("i32 serialisation cannot fail"));

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
    use std::str::FromStr;

    fn params() -> OpenPositionParams {
        OpenPositionParams {
            lb_pair: pubkey(1),
            owner: pubkey(2),
            payer: pubkey(3),
            position: pubkey(4),
            lower_bin_id: -70,
            width: 70,
        }
    }

    #[test]
    fn test_matches_idl_discriminator_and_account_flags() {
        let ix = &build_open_position(&params(), &ComputeBudgetConfig::none()).unwrap()[0];
        assert_matches_idl(ix, "initialize_position2");
    }

    #[test]
    fn test_account_list_order() {
        let params = params();
        let ix = &build_open_position(&params, &ComputeBudgetConfig::none()).unwrap()[0];

        assert_eq!(ix.accounts[0].pubkey, params.payer);
        assert_eq!(ix.accounts[1].pubkey, params.position);
        assert_eq!(ix.accounts[2].pubkey, params.lb_pair);
        assert_eq!(ix.accounts[3].pubkey, params.owner);
        assert_eq!(ix.accounts[4].pubkey, pda::SYSTEM_PROGRAM_ID);
        assert_eq!(ix.accounts[5].pubkey, pda::event_authority());
        assert_eq!(ix.accounts[6].pubkey, dlmm_decode::ID);
    }

    #[test]
    fn test_args_round_trip() {
        let params = params();
        let ix = &build_open_position(&params, &ComputeBudgetConfig::none()).unwrap()[0];
        let lower = i32::from_le_bytes(ix.data[8..12].try_into().unwrap());
        let width = i32::from_le_bytes(ix.data[12..16].try_into().unwrap());
        assert_eq!(lower, params.lower_bin_id);
        assert_eq!(width, params.width);
        assert_eq!(ix.data.len(), 16);
    }

    #[test]
    fn test_zero_width_rejected() {
        let mut params = params();
        params.width = 0;
        assert!(matches!(
            build_open_position(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::WidthOutOfRange { .. })
        ));
    }

    #[test]
    fn test_width_above_max_rejected() {
        let mut params = params();
        params.width = 71;
        assert!(matches!(
            build_open_position(&params, &ComputeBudgetConfig::none()),
            Err(DlmmTxError::WidthOutOfRange { .. })
        ));
    }

    #[test]
    fn test_compute_budget_instructions_are_prepended() {
        let config = ComputeBudgetConfig {
            unit_limit: Some(100_000),
            unit_price_micro_lamports: None,
        };
        let ixs = build_open_position(&params(), &config).unwrap();
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].data[0], 2); // SetComputeUnitLimit
    }

    #[test]
    fn test_event_authority_matches_known_value() {
        assert_eq!(
            pda::event_authority(),
            Pubkey::from_str("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6").unwrap()
        );
    }
}
