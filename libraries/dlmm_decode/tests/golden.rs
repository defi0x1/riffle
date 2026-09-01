// Golden byte tests against real mainnet account data for the SOL-USDC DLMM pool
// (5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6), fetched from api.mainnet-beta.solana.com via
// getAccountInfo. If the program is ever upgraded and a layout shifts, these fail loudly instead
// of the indexer silently writing wrong numbers.
//
// Event decoding has no equivalent live fixture here: self-CPI event bytes only exist inside a
// transaction's inner instructions, and a spot-check against a live swap transaction found a
// discriminator that didn't match any event in the vendored lb_clmm source -- i.e. the deployed
// program's event layout is not guaranteed to match this snapshot of the public repo. Rather
// than guess at an unverified layout, event tests below build their input bytes deterministically
// from known field values and check the round trip, including the two field derivations that
// must be right.

use std::str::FromStr;

use solana_sdk::pubkey::Pubkey;

use dlmm_decode::{
    BIN_ARRAY_DISCRIMINATOR, DecodedEvent, EVENT_IX_TAG, LB_PAIR_DISCRIMINATOR, LiquidityEventKind,
    POSITION_V2_DISCRIMINATOR, PoolStatus, decode_bin_array, decode_event, decode_lb_pair,
    decode_position_v2, discriminator,
};

const LB_PAIR_BYTES: &[u8] = include_bytes!("fixtures/lb_pair_sol_usdc.bin");
const BIN_ARRAY_BYTES: &[u8] = include_bytes!("fixtures/bin_array_sol_usdc_neg81.bin");
const POSITION_BYTES: &[u8] = include_bytes!("fixtures/position_v2_sol_usdc.bin");

fn pubkey(s: &str) -> Pubkey {
    Pubkey::from_str(s).expect("valid base58 pubkey in test fixture assertion")
}

#[test]
fn test_lb_pair_discriminator_matches_fixture() {
    assert_eq!(&LB_PAIR_BYTES[..8], LB_PAIR_DISCRIMINATOR.as_slice());
}

#[test]
fn test_decode_lb_pair_sol_usdc() {
    let pool = decode_lb_pair(LB_PAIR_BYTES).expect("decoding real LbPair account");

    assert_eq!(
        pool.token_x_mint,
        pubkey("So11111111111111111111111111111111111111112")
    );
    assert_eq!(
        pool.token_y_mint,
        pubkey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
    );
    assert_eq!(
        pool.reserve_x,
        pubkey("EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o")
    );
    assert_eq!(
        pool.reserve_y,
        pubkey("CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz")
    );
    assert_eq!(
        pool.oracle,
        pubkey("59YuGWPunbchD2mbi9U7qvjWQKQReGeepn4ZSr9zz9Li")
    );
    assert_eq!(pool.bin_step, 4);
    assert_eq!(pool.active_bin_id, -5664);
    assert_eq!(pool.status, PoolStatus::Enabled);
    assert_eq!(pool.base_factor, 10_000);
    assert_eq!(pool.protocol_share_bps, 1_000);
    assert_eq!(pool.protocol_fee_x, 723_026_282);
    assert_eq!(pool.protocol_fee_y, 102_948_883);
}

#[test]
fn test_decode_lb_pair_rejects_truncated_data() {
    let err = decode_lb_pair(&LB_PAIR_BYTES[..100]).unwrap_err();
    assert!(err.to_string().contains("bytes, expected at least"));
}

#[test]
fn test_decode_lb_pair_rejects_wrong_discriminator() {
    let mut corrupt = LB_PAIR_BYTES.to_vec();
    corrupt[0] ^= 0xff;
    let err = decode_lb_pair(&corrupt).unwrap_err();
    assert!(err.to_string().contains("discriminator mismatch"));
}

#[test]
fn test_bin_array_discriminator_matches_fixture() {
    assert_eq!(&BIN_ARRAY_BYTES[..8], BIN_ARRAY_DISCRIMINATOR.as_slice());
}

#[test]
fn test_decode_bin_array_sol_usdc() {
    let bin_array = decode_bin_array(BIN_ARRAY_BYTES).expect("decoding real BinArray account");

    assert_eq!(
        bin_array.lb_pair,
        pubkey("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6")
    );
    assert_eq!(bin_array.index, -81);
    assert_eq!(bin_array.bins.len(), 70);

    // index -81 * 70 bins/array = -5670, matching lb_clmm's own bin_id_to_bin_array_index.
    assert_eq!(bin_array.bins.first().unwrap().bin_id, -5670);
    assert_eq!(bin_array.bins.last().unwrap().bin_id, -5601);

    // The pool's active bin (-5664, from the LbPair fixture above) sits in this array.
    let active_bin = bin_array.bins.iter().find(|b| b.bin_id == -5664).unwrap();
    assert_eq!(active_bin.amount_x, 168_253_889_802);
    assert_eq!(active_bin.amount_y, 0);
    assert_eq!(active_bin.price, 1_915_044_540_574_782_760);
    assert_eq!(
        active_bin.liquidity_supply,
        322_146_492_608_928_984_903_610_264_474
    );

    assert!(bin_array.bins.iter().all(|b| b.liquidity_supply != 0));
}

#[test]
fn test_position_discriminator_matches_fixture() {
    assert_eq!(&POSITION_BYTES[..8], POSITION_V2_DISCRIMINATOR.as_slice());
}

#[test]
fn test_decode_position_v2_sol_usdc() {
    let position = decode_position_v2(POSITION_BYTES).expect("decoding real PositionV2 account");

    assert_eq!(
        position.lb_pair,
        pubkey("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6")
    );
    assert_eq!(
        position.owner,
        pubkey("GBmnsSCtABHMHp2XUv9LaxXkT325a22X6aMR4dWoYU2")
    );
    assert_eq!(position.lower_bin_id, -4746);
    assert_eq!(position.upper_bin_id, -4678);
    assert_eq!(position.total_claimed_fee_x_amount, 1_212_484);
    assert_eq!(position.total_claimed_fee_y_amount, 114_974);
    assert_eq!(position.operator, Pubkey::default());
    assert_eq!(position.liquidity_shares.len(), 70);
    assert_eq!(position.fee_infos.len(), 70);
}

// --- Event decoding -------------------------------------------------------------------------
//
// No live-chain fixture (see module doc comment above), so these build their own wire bytes
// from known field values, using the crate's own discriminator() function -- the same one
// production code uses -- rather than a hardcoded byte array.

fn event_bytes(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + payload.len());
    buf.extend_from_slice(EVENT_IX_TAG.as_slice());
    buf.extend_from_slice(&discriminator("event", name));
    buf.extend_from_slice(payload);
    buf
}

#[test]
fn test_decode_swap_event_fee_bps_and_lp_fee_derivation() {
    let lb_pair = [7u8; 32];
    let from = [9u8; 32];

    let mut payload = Vec::new();
    payload.extend_from_slice(&lb_pair);
    payload.extend_from_slice(&from);
    payload.extend_from_slice(&(-10i32).to_le_bytes()); // start_bin_id
    payload.extend_from_slice(&(-12i32).to_le_bytes()); // end_bin_id
    payload.extend_from_slice(&1_000_000u64.to_le_bytes()); // amount_in
    payload.extend_from_slice(&998_000u64.to_le_bytes()); // amount_out
    payload.push(1); // swap_for_y
    payload.extend_from_slice(&1_000u64.to_le_bytes()); // fee (includes protocol_fee)
    payload.extend_from_slice(&100u64.to_le_bytes()); // protocol_fee
    payload.extend_from_slice(&1_000_000u128.to_le_bytes()); // fee_bps, FEE_PRECISION units
    payload.extend_from_slice(&5u64.to_le_bytes()); // host_fee

    let bytes = event_bytes("Swap", &payload);
    let decoded = decode_event(&bytes).expect("decoding constructed Swap event");

    let DecodedEvent::Swap(swap) = decoded else {
        panic!("expected Swap variant")
    };
    assert_eq!(swap.lb_pair, Pubkey::from(lb_pair));
    assert_eq!(swap.trader, Pubkey::from(from));
    assert_eq!(swap.start_bin_id, -10);
    assert_eq!(swap.end_bin_id, -12);
    assert_eq!(swap.amount_in, 1_000_000);
    assert_eq!(swap.amount_out, 998_000);
    assert!(swap.swap_for_y);
    assert_eq!(swap.protocol_fee, 100);
    assert_eq!(swap.host_fee, 5);

    // fee_bps = event.fee_bps * BASIS_POINT_MAX / FEE_PRECISION = 1_000_000 * 10_000 / 1e9 = 10
    assert_eq!(swap.fee_bps, 10);
    // event.fee (1000) includes the protocol's cut (100); LPs earn the remainder.
    assert_eq!(swap.lp_fee, 900);
}

#[test]
fn test_decode_add_and_remove_liquidity_events() {
    let lb_pair = [1u8; 32];
    let from = [2u8; 32];
    let position = [3u8; 32];

    let mut payload = Vec::new();
    payload.extend_from_slice(&lb_pair);
    payload.extend_from_slice(&from);
    payload.extend_from_slice(&position);
    payload.extend_from_slice(&500u64.to_le_bytes());
    payload.extend_from_slice(&600u64.to_le_bytes());
    payload.extend_from_slice(&42i32.to_le_bytes());

    let add_bytes = event_bytes("AddLiquidity", &payload);
    let DecodedEvent::AddLiquidity(add) = decode_event(&add_bytes).unwrap() else {
        panic!("expected AddLiquidity variant")
    };
    assert_eq!(add.kind, LiquidityEventKind::Add);
    assert_eq!(add.amount_x, 500);
    assert_eq!(add.amount_y, 600);
    assert_eq!(add.active_bin_id, 42);

    let remove_bytes = event_bytes("RemoveLiquidity", &payload);
    let DecodedEvent::RemoveLiquidity(remove) = decode_event(&remove_bytes).unwrap() else {
        panic!("expected RemoveLiquidity variant")
    };
    assert_eq!(remove.kind, LiquidityEventKind::Remove);
    assert_eq!(remove.position, Pubkey::from(position));
}

#[test]
fn test_decode_claim_fee_and_claim_fee2_events() {
    let lb_pair = [4u8; 32];
    let position = [5u8; 32];
    let owner = [6u8; 32];

    let mut v1_payload = Vec::new();
    v1_payload.extend_from_slice(&lb_pair);
    v1_payload.extend_from_slice(&position);
    v1_payload.extend_from_slice(&owner);
    v1_payload.extend_from_slice(&11u64.to_le_bytes());
    v1_payload.extend_from_slice(&22u64.to_le_bytes());

    let DecodedEvent::ClaimFee(v1) = decode_event(&event_bytes("ClaimFee", &v1_payload)).unwrap()
    else {
        panic!("expected ClaimFee variant")
    };
    assert_eq!(v1.fee_x, 11);
    assert_eq!(v1.fee_y, 22);
    assert_eq!(v1.active_bin_id, None);

    let mut v2_payload = v1_payload.clone();
    v2_payload.extend_from_slice(&(-3i32).to_le_bytes());

    let DecodedEvent::ClaimFee2(v2) = decode_event(&event_bytes("ClaimFee2", &v2_payload)).unwrap()
    else {
        panic!("expected ClaimFee2 variant")
    };
    assert_eq!(v2.active_bin_id, Some(-3));
}

#[test]
fn test_decode_lb_pair_create_position_create_and_close_events() {
    let lb_pair = [8u8; 32];
    let token_x = [10u8; 32];
    let token_y = [11u8; 32];

    let mut create_payload = Vec::new();
    create_payload.extend_from_slice(&lb_pair);
    create_payload.extend_from_slice(&4u16.to_le_bytes());
    create_payload.extend_from_slice(&token_x);
    create_payload.extend_from_slice(&token_y);

    let DecodedEvent::LbPairCreate(created) =
        decode_event(&event_bytes("LbPairCreate", &create_payload)).unwrap()
    else {
        panic!("expected LbPairCreate variant")
    };
    assert_eq!(created.bin_step, 4);
    assert_eq!(created.token_x, Pubkey::from(token_x));
    assert_eq!(created.token_y, Pubkey::from(token_y));

    let position = [12u8; 32];
    let owner = [13u8; 32];
    let mut position_create_payload = Vec::new();
    position_create_payload.extend_from_slice(&lb_pair);
    position_create_payload.extend_from_slice(&position);
    position_create_payload.extend_from_slice(&owner);

    let DecodedEvent::PositionCreate(pc) =
        decode_event(&event_bytes("PositionCreate", &position_create_payload)).unwrap()
    else {
        panic!("expected PositionCreate variant")
    };
    assert_eq!(pc.position, Pubkey::from(position));
    assert_eq!(pc.owner, Pubkey::from(owner));

    let mut position_close_payload = Vec::new();
    position_close_payload.extend_from_slice(&position);
    position_close_payload.extend_from_slice(&owner);

    let DecodedEvent::PositionClose(closed) =
        decode_event(&event_bytes("PositionClose", &position_close_payload)).unwrap()
    else {
        panic!("expected PositionClose variant")
    };
    assert_eq!(closed.position, Pubkey::from(position));
    assert_eq!(closed.owner, Pubkey::from(owner));
}

#[test]
fn test_decode_event_rejects_missing_tag() {
    let err = decode_event(&[0u8; 20]).unwrap_err();
    assert!(err.to_string().contains("self-CPI event tag"));
}

#[test]
fn test_decode_event_rejects_unknown_discriminator() {
    let bytes = event_bytes("ThisEventDoesNotExist", &[]);
    let err = decode_event(&bytes).unwrap_err();
    assert!(err.to_string().contains("Unknown event discriminator"));
}
