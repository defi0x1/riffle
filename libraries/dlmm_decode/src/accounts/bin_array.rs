use eyre::WrapErr;
use solana_sdk::pubkey::Pubkey;
use std::sync::LazyLock;

use crate::accounts::wire::BinArrayWire;
use crate::discriminator::discriminator;

pub static BIN_ARRAY_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("account", "BinArray"));

// See the comment on ACCOUNT_LEN in accounts/lb_pair.rs: zero-copy accounts' in-memory size is
// their on-chain size, so size_of stands in for anchor_lang's Space::INIT_SPACE.
const ACCOUNT_LEN: usize = 8 + std::mem::size_of::<BinArrayWire>();

// 70. bin_id = bin_array.index * BINS_PER_ARRAY + offset_within_array.
const BINS_PER_ARRAY: i32 = crate::constants::MAX_BIN_PER_ARRAY as i32;

#[derive(Clone, Copy, Debug, Default)]
pub struct BinState {
    pub bin_id: i32,
    // Both amounts already exclude the protocol's uncollected fee share -- that lives on the
    // LbPair, not the bin.
    pub amount_x: u64,
    pub amount_y: u64,
    // Q64.64 fixed point, base 1 + bin_step/BASIS_POINT_MAX.
    pub price: u128,
    pub liquidity_supply: u128,
    pub fee_amount_x_per_token_stored: u128,
    pub fee_amount_y_per_token_stored: u128,
}

#[derive(Clone, Debug)]
pub struct BinArrayState {
    pub lb_pair: Pubkey,
    pub index: i64,
    pub bins: Vec<BinState>,
}

pub fn decode_bin_array(data: &[u8]) -> eyre::Result<BinArrayState> {
    if data.len() < ACCOUNT_LEN {
        eyre::bail!(
            "BinArray account data is {} bytes, expected at least {ACCOUNT_LEN}",
            data.len()
        );
    }

    let got_discriminator = &data[..8];
    if got_discriminator != BIN_ARRAY_DISCRIMINATOR.as_slice() {
        eyre::bail!(
            "BinArray discriminator mismatch: got {got_discriminator:?}, expected {:?}",
            *BIN_ARRAY_DISCRIMINATOR
        );
    }

    let raw: BinArrayWire = bytemuck::try_pod_read_unaligned(&data[8..ACCOUNT_LEN])
        .wrap_err_with(|| "Reading BinArray bytes")?;

    let index = i32::try_from(raw.index).wrap_err_with(|| "Casting bin array index to i32")?;
    let lower_bin_id = index
        .checked_mul(BINS_PER_ARRAY)
        .ok_or_else(|| eyre::eyre!("Bin array index {index} overflows lower bin id"))?;

    let bins = raw
        .bins
        .iter()
        .enumerate()
        .map(|(offset, bin)| BinState {
            bin_id: lower_bin_id + offset as i32,
            amount_x: bin.amount_x,
            amount_y: bin.amount_y,
            price: bin.price,
            liquidity_supply: bin.liquidity_supply,
            fee_amount_x_per_token_stored: bin.fee_amount_x_per_token_stored,
            fee_amount_y_per_token_stored: bin.fee_amount_y_per_token_stored,
        })
        .collect();

    Ok(BinArrayState {
        lb_pair: Pubkey::from(raw.lb_pair),
        index: raw.index,
        bins,
    })
}
