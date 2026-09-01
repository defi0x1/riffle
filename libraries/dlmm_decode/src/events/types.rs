use solana_sdk::pubkey::Pubkey;

#[derive(Clone, Debug)]
pub struct DecodedSwap {
    pub lb_pair: Pubkey,
    pub trader: Pubkey,
    pub start_bin_id: i32,
    pub end_bin_id: i32,
    pub amount_in: u64,
    pub amount_out: u64,
    pub swap_for_y: bool,
    // event.fee_bps is in FEE_PRECISION (1e9) units; converted to basis points
    // (BASIS_POINT_MAX = 10_000) here: fee_bps = event.fee_bps * 10_000 / 1e9.
    pub fee_bps: u64,
    // event.fee INCLUDES the protocol's cut -- LPs only earn fee - protocol_fee. Using
    // event.fee directly would silently overstate LP revenue everywhere downstream.
    pub lp_fee: u64,
    pub protocol_fee: u64,
    pub host_fee: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidityEventKind {
    Add,
    Remove,
}

#[derive(Clone, Debug)]
pub struct DecodedLiquidityEvent {
    pub kind: LiquidityEventKind,
    pub lb_pair: Pubkey,
    pub from: Pubkey,
    pub position: Pubkey,
    pub amount_x: u64,
    pub amount_y: u64,
    pub active_bin_id: i32,
}

#[derive(Clone, Debug)]
pub struct DecodedClaimFee {
    pub lb_pair: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub fee_x: u64,
    pub fee_y: u64,
    // ClaimFee (v1) doesn't carry this; ClaimFee2 does.
    pub active_bin_id: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct DecodedLbPairCreate {
    pub lb_pair: Pubkey,
    pub bin_step: u16,
    pub token_x: Pubkey,
    pub token_y: Pubkey,
}

#[derive(Clone, Debug)]
pub struct DecodedPositionCreate {
    pub lb_pair: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
}

#[derive(Clone, Debug)]
pub struct DecodedPositionClose {
    pub position: Pubkey,
    pub owner: Pubkey,
}

#[derive(Clone, Debug)]
pub enum DecodedEvent {
    Swap(DecodedSwap),
    AddLiquidity(DecodedLiquidityEvent),
    RemoveLiquidity(DecodedLiquidityEvent),
    ClaimFee(DecodedClaimFee),
    ClaimFee2(DecodedClaimFee),
    LbPairCreate(DecodedLbPairCreate),
    PositionCreate(DecodedPositionCreate),
    PositionClose(DecodedPositionClose),
}
