use borsh::BorshSerialize;

// Every type below mirrors a `types` entry in the public IDL
// (https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json) field-for-field and
// variant-for-variant. Anchor borsh-serialises a fieldless enum as its declaration-order index,
// so the variant order here must match the IDL exactly even though nothing else about the enum
// depends on it.

/// IDL `StrategyType`. Fee-farming around the active bin wants one of the `*Balanced` variants
/// (equal-value liquidity either side); the `*ImBalanced` variants read `favor_token_x` on
/// `StrategyParameters` to lean the deposit toward one token, and the `*OneSide` variants are for
/// single-sided ranges entirely above or below the active bin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize)]
pub enum StrategyType {
    SpotOneSide,
    CurveOneSide,
    BidAskOneSide,
    SpotBalanced,
    CurveBalanced,
    BidAskBalanced,
    SpotImBalanced,
    CurveImBalanced,
    BidAskImBalanced,
}

/// IDL `StrategyParameters`. `parameters` is a 64-byte scratch buffer the program only reads one
/// byte of today -- byte 0 is a 0/1 "favour token X" flag consumed by the `*ImBalanced` variants
/// -- the rest is reserved and must stay zero.
#[derive(Clone, Copy, Debug, BorshSerialize)]
pub struct StrategyParameters {
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub strategy_type: StrategyType,
    pub parameters: [u8; 64],
}

impl StrategyParameters {
    pub fn new(
        min_bin_id: i32,
        max_bin_id: i32,
        strategy_type: StrategyType,
        favor_token_x: bool,
    ) -> Self {
        let mut parameters = [0u8; 64];
        parameters[0] = u8::from(favor_token_x);
        Self {
            min_bin_id,
            max_bin_id,
            strategy_type,
            parameters,
        }
    }
}

/// IDL `LiquidityParameterByStrategy`, the argument to `add_liquidity_by_strategy2`.
#[derive(Clone, Copy, Debug, BorshSerialize)]
pub struct LiquidityParameterByStrategy {
    pub amount_x: u64,
    pub amount_y: u64,
    pub active_id: i32,
    pub max_active_bin_slippage: i32,
    pub strategy_parameters: StrategyParameters,
}

/// IDL `AccountsType`, used only to describe transfer-hook remaining accounts. This crate never
/// builds transfer-hook accounts (see the note on `RemainingAccountsInfo::none`), but the type
/// still needs to serialise correctly since every v2 instruction takes a value of it.
#[derive(Clone, Copy, Debug, BorshSerialize)]
pub enum AccountsType {
    TransferHookX,
    TransferHookY,
    TransferHookReward,
    TransferHookMultiReward(u8),
    TransferHookReferral,
}

/// IDL `RemainingAccountsSlice`.
#[derive(Clone, Copy, Debug, BorshSerialize)]
pub struct RemainingAccountsSlice {
    pub accounts_type: AccountsType,
    pub length: u8,
}

/// IDL `RemainingAccountsInfo`. Describes how many of the instruction's trailing remaining
/// accounts are transfer-hook accounts, split by which side of the pool they belong to; every
/// account after that is assumed to be a bin array.
#[derive(Clone, Debug, BorshSerialize)]
pub struct RemainingAccountsInfo {
    pub slices: Vec<RemainingAccountsSlice>,
}

impl RemainingAccountsInfo {
    /// This crate only supports SPL Token and Token-2022 mints without a transfer hook -- a
    /// mint with one would need its hook program's extra accounts appended and described here,
    /// which is out of scope for now. Matches the public SDK's own shape for a hookless pool:
    /// present-but-zero-length slices for both sides, not an empty vec.
    pub fn none() -> Self {
        Self {
            slices: vec![
                RemainingAccountsSlice {
                    accounts_type: AccountsType::TransferHookX,
                    length: 0,
                },
                RemainingAccountsSlice {
                    accounts_type: AccountsType::TransferHookY,
                    length: 0,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_type_variant_order_matches_borsh_u8_index() {
        // Anchor enums are borsh-tagged by declaration order; SpotBalanced is IDL index 3.
        assert_eq!(
            borsh::to_vec(&StrategyType::SpotBalanced).unwrap(),
            vec![3u8]
        );
        assert_eq!(
            borsh::to_vec(&StrategyType::BidAskImBalanced).unwrap(),
            vec![8u8]
        );
    }

    #[test]
    fn test_strategy_parameters_favor_flag_is_the_only_nonzero_byte() {
        let params = StrategyParameters::new(-10, 10, StrategyType::SpotImBalanced, true);
        assert_eq!(params.parameters[0], 1);
        assert!(params.parameters[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_remaining_accounts_info_none_round_trips() {
        let info = RemainingAccountsInfo::none();
        let bytes = borsh::to_vec(&info).unwrap();
        // 4-byte vec length prefix + 2 slices, each 1 byte enum tag + 1 byte length.
        assert_eq!(bytes.len(), 4 + 2 * 2);
        assert_eq!(&bytes[..4], &2u32.to_le_bytes());
    }

    #[test]
    fn test_liquidity_parameter_by_strategy_serialises_fields_in_declared_order() {
        let params = LiquidityParameterByStrategy {
            amount_x: 1_000,
            amount_y: 2_000,
            active_id: 42,
            max_active_bin_slippage: 3,
            strategy_parameters: StrategyParameters::new(
                -10,
                10,
                StrategyType::SpotBalanced,
                false,
            ),
        };
        let bytes = borsh::to_vec(&params).unwrap();
        assert_eq!(&bytes[0..8], &1_000u64.to_le_bytes());
        assert_eq!(&bytes[8..16], &2_000u64.to_le_bytes());
        assert_eq!(&bytes[16..20], &42i32.to_le_bytes());
        assert_eq!(&bytes[20..24], &3i32.to_le_bytes());
        // strategy_parameters: min_bin_id, max_bin_id, strategy_type tag, 64-byte parameters.
        assert_eq!(&bytes[24..28], &(-10i32).to_le_bytes());
        assert_eq!(&bytes[28..32], &10i32.to_le_bytes());
        assert_eq!(bytes[32], 3u8);
        assert_eq!(bytes.len(), 24 + 8 + 1 + 64);
    }
}
