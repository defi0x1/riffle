use borsh::BorshDeserialize;

// Wire-shape mirrors of lb_clmm::events structs (public source, MeteoraAg/dlmm-sdk), field-for-
// field in declaration order -- Borsh has no field tags, so byte layout is purely positional.
// Kept private and separate from the public Decoded* types so lb_clmm-shaped structs never
// leak across the crate boundary; Pubkey fields stay raw [u8; 32] here and get converted to
// solana_sdk::pubkey::Pubkey when mapped into the public types.

#[derive(BorshDeserialize)]
pub(crate) struct SwapWire {
    pub lb_pair: [u8; 32],
    pub from: [u8; 32],
    pub start_bin_id: i32,
    pub end_bin_id: i32,
    pub amount_in: u64,
    pub amount_out: u64,
    pub swap_for_y: bool,
    pub fee: u64,
    pub protocol_fee: u64,
    pub fee_bps: u128,
    pub host_fee: u64,
}

// Shared shape of AddLiquidity and RemoveLiquidity.
#[derive(BorshDeserialize)]
pub(crate) struct LiquidityWire {
    pub lb_pair: [u8; 32],
    pub from: [u8; 32],
    pub position: [u8; 32],
    pub amounts: [u64; 2],
    pub active_bin_id: i32,
}

#[derive(BorshDeserialize)]
pub(crate) struct ClaimFeeWire {
    pub lb_pair: [u8; 32],
    pub position: [u8; 32],
    pub owner: [u8; 32],
    pub fee_x: u64,
    pub fee_y: u64,
}

#[derive(BorshDeserialize)]
pub(crate) struct ClaimFee2Wire {
    pub lb_pair: [u8; 32],
    pub position: [u8; 32],
    pub owner: [u8; 32],
    pub fee_x: u64,
    pub fee_y: u64,
    pub active_bin_id: i32,
}

#[derive(BorshDeserialize)]
pub(crate) struct LbPairCreateWire {
    pub lb_pair: [u8; 32],
    pub bin_step: u16,
    pub token_x: [u8; 32],
    pub token_y: [u8; 32],
}

// Shared shape of PositionCreate.
#[derive(BorshDeserialize)]
pub(crate) struct PositionCreateWire {
    pub lb_pair: [u8; 32],
    pub position: [u8; 32],
    pub owner: [u8; 32],
}

#[derive(BorshDeserialize)]
pub(crate) struct PositionCloseWire {
    pub position: [u8; 32],
    pub owner: [u8; 32],
}
