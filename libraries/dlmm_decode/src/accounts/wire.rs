use bytemuck::{Pod, Zeroable};

// Wire-shape mirrors of the public IDL's zero-copy account types (MeteoraAg/dlmm-sdk,
// https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json), field-for-field
// in declaration order -- same pattern as events::wire, but read via
// bytemuck::try_pod_read_unaligned off raw account bytes instead of Borsh-decoded, so every
// padding and reserved field below is load-bearing: #[repr(C)] plus bytemuck's Pod derive
// only compiles when the field list leaves no compiler-inserted padding, which is what makes
// this the same layout the program itself writes. Pubkey fields stay raw [u8; 32] here, same
// as events::wire, and get converted to solana_sdk::pubkey::Pubkey when mapped into the
// public *State types.

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ProtocolFeeWire {
    pub amount_x: u64,
    pub amount_y: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct RewardInfoWire {
    pub mint: [u8; 32],
    pub vault: [u8; 32],
    pub funder: [u8; 32],
    pub reward_duration: u64,
    pub reward_duration_end: u64,
    pub reward_rate: u128,
    pub last_update_time: u64,
    pub cumulative_seconds_with_empty_liquidity_reward: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct StaticParametersWire {
    pub base_factor: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub protocol_share: u16,
    pub base_fee_power_factor: u8,
    pub function_type: u8,
    pub collect_fee_mode: u8,
    pub _padding: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct VariableParametersWire {
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub _padding: [u8; 4],
    pub last_update_timestamp: i64,
    pub _padding_1: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct LbPairWire {
    pub parameters: StaticParametersWire,
    pub v_parameters: VariableParametersWire,
    pub bump_seed: [u8; 1],
    pub bin_step_seed: [u8; 2],
    pub pair_type: u8,
    pub active_id: i32,
    pub bin_step: u16,
    pub status: u8,
    pub require_base_factor_seed: u8,
    pub base_factor_seed: [u8; 2],
    pub activation_type: u8,
    pub creator_pool_on_off_control: u8,
    pub token_x_mint: [u8; 32],
    pub token_y_mint: [u8; 32],
    pub reserve_x: [u8; 32],
    pub reserve_y: [u8; 32],
    pub protocol_fee: ProtocolFeeWire,
    pub _padding_1: [u8; 32],
    pub reward_infos: [RewardInfoWire; 2],
    pub oracle: [u8; 32],
    pub bin_array_bitmap: [u64; 16],
    pub last_updated_at: i64,
    pub _padding_2: [u8; 32],
    pub pre_activation_swap_address: [u8; 32],
    pub base_key: [u8; 32],
    pub activation_point: u64,
    pub pre_activation_duration: u64,
    pub _padding_3: [u8; 8],
    pub _padding_4: u64,
    pub creator: [u8; 32],
    pub token_mint_x_program_flag: u8,
    pub token_mint_y_program_flag: u8,
    pub version: u8,
    pub _reserved: [u8; 21],
}

const _: () = assert!(std::mem::size_of::<LbPairWire>() == 896);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct BinWire {
    pub amount_x: u64,
    pub amount_y: u64,
    pub price: u128,
    pub liquidity_supply: u128,
    pub fulfilled_order_amount_x: u64,
    pub fulfilled_order_amount_y: u64,
    pub limit_order_fee_ask_side: u64,
    pub limit_order_fee_bid_side: u64,
    pub fee_amount_x_per_token_stored: u128,
    pub fee_amount_y_per_token_stored: u128,
    pub open_order_amount: u64,
    pub total_processing_order_amount: u64,
    pub processed_order_remaining_amount: u64,
    pub order_age: u32,
    pub limit_order_ask_side: u8,
    pub _padding_1: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct BinArrayWire {
    pub index: i64,
    pub version: u8,
    pub _padding_1: [u8; 7],
    pub lb_pair: [u8; 32],
    pub bins: [BinWire; crate::constants::MAX_BIN_PER_ARRAY],
}

const _: () = assert!(std::mem::size_of::<BinArrayWire>() == 10128);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct FeeInfoWire {
    pub fee_x_per_token_complete: u128,
    pub fee_y_per_token_complete: u128,
    pub fee_x_pending: u64,
    pub fee_y_pending: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct UserRewardInfoWire {
    pub reward_per_token_completes: [u128; 2],
    pub reward_pendings: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PositionV2Wire {
    pub lb_pair: [u8; 32],
    pub owner: [u8; 32],
    pub liquidity_shares: [u128; crate::constants::MAX_BIN_PER_ARRAY],
    pub reward_infos: [UserRewardInfoWire; crate::constants::MAX_BIN_PER_ARRAY],
    pub fee_infos: [FeeInfoWire; crate::constants::MAX_BIN_PER_ARRAY],
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub last_updated_at: i64,
    pub total_claimed_fee_x_amount: u64,
    pub total_claimed_fee_y_amount: u64,
    pub total_claimed_rewards: [u64; 2],
    pub operator: [u8; 32],
    pub lock_release_point: u64,
    pub _padding_0: u8,
    pub fee_owner: [u8; 32],
    pub version: u8,
    pub permissionless_operation_bits: u8,
    pub _reserved: [u8; 85],
}

const _: () = assert!(std::mem::size_of::<PositionV2Wire>() == 8112);
