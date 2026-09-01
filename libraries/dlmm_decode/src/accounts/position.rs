use eyre::WrapErr;
use solana_sdk::pubkey::Pubkey;
use std::sync::LazyLock;

use crate::accounts::wire::PositionV2Wire;
use crate::discriminator::discriminator;

pub static POSITION_V2_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("account", "PositionV2"));

// See the comment on ACCOUNT_LEN in accounts/lb_pair.rs: zero-copy accounts' in-memory size is
// their on-chain size, so size_of stands in for anchor_lang's Space::INIT_SPACE.
const ACCOUNT_LEN: usize = 8 + std::mem::size_of::<PositionV2Wire>();

#[derive(Clone, Copy, Debug, Default)]
pub struct FeeInfoState {
    pub fee_x_per_token_complete: u128,
    pub fee_y_per_token_complete: u128,
    pub fee_x_pending: u64,
    pub fee_y_pending: u64,
}

#[derive(Clone, Debug)]
pub struct PositionState {
    pub lb_pair: Pubkey,
    pub owner: Pubkey,
    pub operator: Pubkey,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub last_updated_at: i64,
    pub total_claimed_fee_x_amount: u64,
    pub total_claimed_fee_y_amount: u64,
    pub lock_release_point: u64,
    // Per-bin liquidity share, indexed from lower_bin_id. Only the first 70 bins -- a position
    // resized past that (increase_position_length) appends PositionBinData past ACCOUNT_LEN,
    // which this decoder does not read.
    pub liquidity_shares: Vec<u128>,
    pub fee_infos: Vec<FeeInfoState>,
}

pub fn decode_position_v2(data: &[u8]) -> eyre::Result<PositionState> {
    if data.len() < ACCOUNT_LEN {
        eyre::bail!(
            "PositionV2 account data is {} bytes, expected at least {ACCOUNT_LEN}",
            data.len()
        );
    }

    let got_discriminator = &data[..8];
    if got_discriminator != POSITION_V2_DISCRIMINATOR.as_slice() {
        eyre::bail!(
            "PositionV2 discriminator mismatch: got {got_discriminator:?}, expected {:?}",
            *POSITION_V2_DISCRIMINATOR
        );
    }

    let raw: PositionV2Wire = bytemuck::try_pod_read_unaligned(&data[8..ACCOUNT_LEN])
        .wrap_err_with(|| "Reading PositionV2 bytes")?;

    let liquidity_shares = raw.liquidity_shares.to_vec();
    let fee_infos = raw
        .fee_infos
        .iter()
        .map(|f| FeeInfoState {
            fee_x_per_token_complete: f.fee_x_per_token_complete,
            fee_y_per_token_complete: f.fee_y_per_token_complete,
            fee_x_pending: f.fee_x_pending,
            fee_y_pending: f.fee_y_pending,
        })
        .collect();

    Ok(PositionState {
        lb_pair: Pubkey::from(raw.lb_pair),
        owner: Pubkey::from(raw.owner),
        operator: Pubkey::from(raw.operator),
        lower_bin_id: raw.lower_bin_id,
        upper_bin_id: raw.upper_bin_id,
        last_updated_at: raw.last_updated_at,
        total_claimed_fee_x_amount: raw.total_claimed_fee_x_amount,
        total_claimed_fee_y_amount: raw.total_claimed_fee_y_amount,
        lock_release_point: raw.lock_release_point,
        liquidity_shares,
        fee_infos,
    })
}
