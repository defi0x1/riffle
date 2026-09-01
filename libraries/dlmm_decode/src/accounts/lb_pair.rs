use eyre::WrapErr;
use solana_sdk::pubkey::Pubkey;
use std::sync::LazyLock;

use crate::discriminator::discriminator;

pub static LB_PAIR_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("account", "LbPair"));

// LbPair is a zero-copy (#[repr(C)], bytemuck::Pod) account, so its in-memory size is exactly
// its on-chain size -- no separate INIT_SPACE constant needed (that comes from anchor_lang's
// Space trait, which would pull in a dependency we don't otherwise need).
const ACCOUNT_LEN: usize = 8 + std::mem::size_of::<lb_clmm::state::lb_pair::LbPair>();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolStatus {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug)]
pub struct PoolState {
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
    pub bin_step: u16,
    pub active_bin_id: i32,
    pub status: PoolStatus,
    // Base fee rate = base_factor * bin_step * 10 * 10^base_fee_power_factor, in FEE_PRECISION
    // (1e9) units. Left as the raw components rather than pre-combined into a bps figure --
    // that conversion belongs to dlmm_math, alongside the variable fee formula it composes with.
    pub base_factor: u16,
    pub base_fee_power_factor: u8,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    // Share of swap fee routed to the protocol, in basis points (BASIS_POINT_MAX = 10_000).
    pub protocol_share_bps: u16,
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub protocol_fee_x: u64,
    pub protocol_fee_y: u64,
    pub last_updated_at: i64,
}

pub fn decode_lb_pair(data: &[u8]) -> eyre::Result<PoolState> {
    if data.len() < ACCOUNT_LEN {
        eyre::bail!(
            "LbPair account data is {} bytes, expected at least {ACCOUNT_LEN}",
            data.len()
        );
    }

    let got_discriminator = &data[..8];
    if got_discriminator != LB_PAIR_DISCRIMINATOR.as_slice() {
        eyre::bail!(
            "LbPair discriminator mismatch: got {got_discriminator:?}, expected {:?}",
            *LB_PAIR_DISCRIMINATOR
        );
    }

    let raw: lb_clmm::state::lb_pair::LbPair =
        bytemuck::try_pod_read_unaligned(&data[8..ACCOUNT_LEN])
            .wrap_err_with(|| "Reading LbPair bytes")?;

    let status = match raw.status {
        0 => PoolStatus::Enabled,
        1 => PoolStatus::Disabled,
        other => eyre::bail!("Unknown LbPair status byte {other}"),
    };

    Ok(PoolState {
        token_x_mint: raw.token_x_mint,
        token_y_mint: raw.token_y_mint,
        reserve_x: raw.reserve_x,
        reserve_y: raw.reserve_y,
        oracle: raw.oracle,
        bin_step: raw.bin_step,
        active_bin_id: raw.active_id,
        status,
        base_factor: raw.parameters.base_factor,
        base_fee_power_factor: raw.parameters.base_fee_power_factor,
        filter_period: raw.parameters.filter_period,
        decay_period: raw.parameters.decay_period,
        reduction_factor: raw.parameters.reduction_factor,
        variable_fee_control: raw.parameters.variable_fee_control,
        max_volatility_accumulator: raw.parameters.max_volatility_accumulator,
        protocol_share_bps: raw.parameters.protocol_share,
        volatility_accumulator: raw.v_parameters.volatility_accumulator,
        volatility_reference: raw.v_parameters.volatility_reference,
        index_reference: raw.v_parameters.index_reference,
        protocol_fee_x: raw.protocol_fee.amount_x,
        protocol_fee_y: raw.protocol_fee.amount_y,
        last_updated_at: raw.last_updated_at,
    })
}
