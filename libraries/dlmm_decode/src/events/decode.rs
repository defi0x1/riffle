use std::sync::LazyLock;

use borsh::BorshDeserialize;
use eyre::WrapErr;

use crate::discriminator::{EVENT_IX_TAG, discriminator};
use crate::events::types::*;
use crate::events::wire::*;

static SWAP_DISCRIMINATOR: LazyLock<[u8; 8]> = LazyLock::new(|| discriminator("event", "Swap"));
static ADD_LIQUIDITY_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "AddLiquidity"));
static REMOVE_LIQUIDITY_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "RemoveLiquidity"));
static CLAIM_FEE_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "ClaimFee"));
static CLAIM_FEE2_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "ClaimFee2"));
static LB_PAIR_CREATE_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "LbPairCreate"));
static POSITION_CREATE_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "PositionCreate"));
static POSITION_CLOSE_DISCRIMINATOR: LazyLock<[u8; 8]> =
    LazyLock::new(|| discriminator("event", "PositionClose"));

// `data` is the full self-CPI instruction data as it appears in an inner instruction of the
// transaction: EVENT_IX_TAG_LE (8 bytes), then the per-event Anchor discriminator (8 bytes),
// then the Borsh-encoded event body. for the framing.
pub fn decode_event(data: &[u8]) -> eyre::Result<DecodedEvent> {
    if data.len() < 16 {
        eyre::bail!(
            "Event data is {} bytes, expected at least 16 (tag + discriminator)",
            data.len()
        );
    }

    if &data[..8] != EVENT_IX_TAG.as_slice() {
        eyre::bail!("Data does not start with the Anchor self-CPI event tag");
    }

    let event_discriminator = &data[8..16];
    let payload = &data[16..];

    if event_discriminator == SWAP_DISCRIMINATOR.as_slice() {
        let wire = SwapWire::try_from_slice(payload).wrap_err_with(|| "Decoding Swap event")?;
        return Ok(DecodedEvent::Swap(map_swap(wire)?));
    }
    if event_discriminator == ADD_LIQUIDITY_DISCRIMINATOR.as_slice() {
        let wire = LiquidityWire::try_from_slice(payload)
            .wrap_err_with(|| "Decoding AddLiquidity event")?;
        return Ok(DecodedEvent::AddLiquidity(map_liquidity(
            wire,
            LiquidityEventKind::Add,
        )));
    }
    if event_discriminator == REMOVE_LIQUIDITY_DISCRIMINATOR.as_slice() {
        let wire = LiquidityWire::try_from_slice(payload)
            .wrap_err_with(|| "Decoding RemoveLiquidity event")?;
        return Ok(DecodedEvent::RemoveLiquidity(map_liquidity(
            wire,
            LiquidityEventKind::Remove,
        )));
    }
    if event_discriminator == CLAIM_FEE_DISCRIMINATOR.as_slice() {
        let wire =
            ClaimFeeWire::try_from_slice(payload).wrap_err_with(|| "Decoding ClaimFee event")?;
        return Ok(DecodedEvent::ClaimFee(DecodedClaimFee {
            lb_pair: wire.lb_pair.into(),
            position: wire.position.into(),
            owner: wire.owner.into(),
            fee_x: wire.fee_x,
            fee_y: wire.fee_y,
            active_bin_id: None,
        }));
    }
    if event_discriminator == CLAIM_FEE2_DISCRIMINATOR.as_slice() {
        let wire =
            ClaimFee2Wire::try_from_slice(payload).wrap_err_with(|| "Decoding ClaimFee2 event")?;
        return Ok(DecodedEvent::ClaimFee2(DecodedClaimFee {
            lb_pair: wire.lb_pair.into(),
            position: wire.position.into(),
            owner: wire.owner.into(),
            fee_x: wire.fee_x,
            fee_y: wire.fee_y,
            active_bin_id: Some(wire.active_bin_id),
        }));
    }
    if event_discriminator == LB_PAIR_CREATE_DISCRIMINATOR.as_slice() {
        let wire = LbPairCreateWire::try_from_slice(payload)
            .wrap_err_with(|| "Decoding LbPairCreate event")?;
        return Ok(DecodedEvent::LbPairCreate(DecodedLbPairCreate {
            lb_pair: wire.lb_pair.into(),
            bin_step: wire.bin_step,
            token_x: wire.token_x.into(),
            token_y: wire.token_y.into(),
        }));
    }
    if event_discriminator == POSITION_CREATE_DISCRIMINATOR.as_slice() {
        let wire = PositionCreateWire::try_from_slice(payload)
            .wrap_err_with(|| "Decoding PositionCreate event")?;
        return Ok(DecodedEvent::PositionCreate(DecodedPositionCreate {
            lb_pair: wire.lb_pair.into(),
            position: wire.position.into(),
            owner: wire.owner.into(),
        }));
    }
    if event_discriminator == POSITION_CLOSE_DISCRIMINATOR.as_slice() {
        let wire = PositionCloseWire::try_from_slice(payload)
            .wrap_err_with(|| "Decoding PositionClose event")?;
        return Ok(DecodedEvent::PositionClose(DecodedPositionClose {
            position: wire.position.into(),
            owner: wire.owner.into(),
        }));
    }

    eyre::bail!("Unknown event discriminator {event_discriminator:?}")
}

fn map_liquidity(wire: LiquidityWire, kind: LiquidityEventKind) -> DecodedLiquidityEvent {
    DecodedLiquidityEvent {
        kind,
        lb_pair: wire.lb_pair.into(),
        from: wire.from.into(),
        position: wire.position.into(),
        amount_x: wire.amounts[0],
        amount_y: wire.amounts[1],
        active_bin_id: wire.active_bin_id,
    }
}

fn map_swap(wire: SwapWire) -> eyre::Result<DecodedSwap> {
    let fee_bps = wire
        .fee_bps
        .checked_mul(crate::constants::BASIS_POINT_MAX as u128)
        .and_then(|v| v.checked_div(crate::constants::FEE_PRECISION as u128))
        .ok_or_else(|| eyre::eyre!("fee_bps conversion overflowed"))?;
    let fee_bps: u64 = fee_bps
        .try_into()
        .wrap_err_with(|| "Casting fee_bps to u64")?;

    let lp_fee = wire.fee.checked_sub(wire.protocol_fee).ok_or_else(|| {
        eyre::eyre!(
            "protocol_fee {} exceeds fee {}",
            wire.protocol_fee,
            wire.fee
        )
    })?;

    Ok(DecodedSwap {
        lb_pair: wire.lb_pair.into(),
        trader: wire.from.into(),
        start_bin_id: wire.start_bin_id,
        end_bin_id: wire.end_bin_id,
        amount_in: wire.amount_in,
        amount_out: wire.amount_out,
        swap_for_y: wire.swap_for_y,
        fee_bps,
        lp_fee,
        protocol_fee: wire.protocol_fee,
        host_fee: wire.host_fee,
    })
}
