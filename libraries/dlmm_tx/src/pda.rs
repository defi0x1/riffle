use solana_sdk::pubkey::Pubkey;

// Every seed and program id below is transcribed from the public IDL
// (https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json) or, for the ATA
// program, the well-known SPL associated-token-account program. All five derivations were also
// cross-checked against live mainnet accounts for the SOL-USDC pool
// (5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6) during development: bin_array(-81), the pool's
// own reserve_x/reserve_y, event_authority, and a real owner's USDC ATA all matched what the
// running program actually uses.

/// PDA seed for a self-CPI event authority: `[b"__event_authority"]`. Every Anchor program using
/// `emit_cpi!` has exactly one of these, at a fixed address independent of any other account.
const EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";

/// PDA seed prefix for a BinArrayBitmapExtension account: `[b"bitmap", lb_pair]`.
const BIN_ARRAY_BITMAP_SEED: &[u8] = b"bitmap";

/// The extension is only needed once a pool's bin arrays run past the 512-wide default bitmap
/// window built into the LbPair account itself (IDL constant BIN_ARRAY_BITMAP_SIZE).
const DEFAULT_BITMAP_BIN_ARRAY_RANGE: i64 = 512;

pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub const TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

pub const MEMO_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

// solana_sdk::system_program::ID resolves to this same value but its module is deprecated in
// favour of a separate solana-system-interface crate; declaring it directly avoids that warning
// without adding another workspace dependency for one well-known constant.
pub const SYSTEM_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("11111111111111111111111111111111");

pub fn event_authority() -> Pubkey {
    Pubkey::find_program_address(&[EVENT_AUTHORITY_SEED], &dlmm_decode::ID).0
}

// Copied from libraries/source/src/bin_array.rs (which keeps it pub(crate)) rather than
// depending on that crate for one function -- see the report on this task for the note to
// unify the two copies later.
pub fn bin_array(lb_pair: &Pubkey, index: i64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            dlmm_decode::BIN_ARRAY,
            lb_pair.as_ref(),
            &index.to_le_bytes(),
        ],
        &dlmm_decode::ID,
    )
    .0
}

/// Every BinArray PDA a `[lower_bin_id, upper_bin_id]` range touches, one per array index in
/// that span, lowest index first -- the order `add_liquidity_by_strategy2` and its siblings
/// expect their trailing remaining accounts in.
pub fn bin_arrays_covering_range(
    lb_pair: &Pubkey,
    lower_bin_id: i32,
    upper_bin_id: i32,
) -> Vec<Pubkey> {
    let lower_index = dlmm_decode::bin_id_to_bin_array_index(lower_bin_id) as i64;
    let upper_index = dlmm_decode::bin_id_to_bin_array_index(upper_bin_id) as i64;
    (lower_index..=upper_index)
        .map(|index| bin_array(lb_pair, index))
        .collect()
}

/// `None` once the pool's default 512-wide bitmap can no longer address every array a range
/// touches, mirroring the SDK's `isOverflowDefaultBinArrayBitmap` check -- at that point the
/// account is required and the program id placeholder (see `optional_bin_array_bitmap_extension`
/// below) is no longer valid.
pub fn bin_array_bitmap_extension_required(lower_bin_id: i32, upper_bin_id: i32) -> bool {
    let lower_index = dlmm_decode::bin_id_to_bin_array_index(lower_bin_id) as i64;
    let upper_index = dlmm_decode::bin_id_to_bin_array_index(upper_bin_id) as i64;
    lower_index < -DEFAULT_BITMAP_BIN_ARRAY_RANGE
        || upper_index > DEFAULT_BITMAP_BIN_ARRAY_RANGE - 1
}

pub fn bin_array_bitmap_extension(lb_pair: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[BIN_ARRAY_BITMAP_SEED, lb_pair.as_ref()], &dlmm_decode::ID).0
}

/// The `bin_array_bitmap_extension` account is declared `optional` in the IDL; Anchor's
/// convention for a missing optional account is to pass the program's own id as a sentinel
/// rather than omitting the account entirely. Only derive the real PDA when the range actually
/// needs it.
pub fn optional_bin_array_bitmap_extension(
    lb_pair: &Pubkey,
    lower_bin_id: i32,
    upper_bin_id: i32,
) -> Pubkey {
    if bin_array_bitmap_extension_required(lower_bin_id, upper_bin_id) {
        bin_array_bitmap_extension(lb_pair)
    } else {
        dlmm_decode::ID
    }
}

/// A pool's token reserve vault: PDA of `[lb_pair, mint]` with no seed prefix at all -- unusual
/// among this program's other PDAs, but confirmed against the SOL-USDC pool's own decoded
/// LbPair.reserve_x/reserve_y fields.
pub fn reserve(lb_pair: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[lb_pair.as_ref(), mint.as_ref()], &dlmm_decode::ID).0
}

/// The associated token account for `(owner, mint)` under `token_program` -- standard SPL
/// derivation, works for both the Token and Token-2022 programs.
pub fn associated_token_address(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn pubkey(s: &str) -> Pubkey {
        Pubkey::from_str(s).unwrap()
    }

    // Live mainnet SOL-USDC pool, the same fixture libraries/dlmm_decode's golden tests use.
    const SOL_USDC_POOL: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";

    #[test]
    fn test_bin_array_matches_live_account() {
        // getAccountInfo on this address, fetched during development, is a real BinArray
        // account owned by the DLMM program whose decoded index is -81 and lb_pair is this pool.
        let pool = pubkey(SOL_USDC_POOL);
        assert_eq!(
            bin_array(&pool, -81),
            pubkey("HQH5fsUpWdDtV5m4EaJo6TNcbLq5HxFzYzGXBptgJDD3")
        );
    }

    #[test]
    fn test_event_authority_matches_live_value() {
        assert_eq!(
            event_authority(),
            pubkey("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6")
        );
    }

    #[test]
    fn test_reserve_matches_decoded_lb_pair_fields() {
        // libraries/dlmm_decode/tests/golden.rs decodes this exact pool's LbPair account and
        // asserts reserve_x/reserve_y equal these two pubkeys -- if this derivation ever drifts
        // from the program's own PDA scheme, this test and that one now disagree.
        let pool = pubkey(SOL_USDC_POOL);
        let sol_mint = pubkey("So11111111111111111111111111111111111111112");
        let usdc_mint = pubkey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

        assert_eq!(
            reserve(&pool, &sol_mint),
            pubkey("EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o")
        );
        assert_eq!(
            reserve(&pool, &usdc_mint),
            pubkey("CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz")
        );
    }

    #[test]
    fn test_associated_token_address_matches_live_account() {
        // A real position owner (from the same PositionV2 fixture) and their real USDC ATA,
        // confirmed live via getAccountInfo during development.
        let owner = pubkey("GBmnsSCtABHMHp2XUv9LaxXkT325a22X6aMR4dWoYU2");
        let usdc_mint = pubkey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

        assert_eq!(
            associated_token_address(&owner, &usdc_mint, &TOKEN_PROGRAM_ID),
            pubkey("EbXFnwhxXsydSQxzRXMTUpU5NYGqy9svK2XYZj7EeE2a")
        );
    }

    #[test]
    fn test_bin_arrays_covering_range_is_inclusive_and_ordered() {
        let pool = pubkey(SOL_USDC_POOL);
        let arrays = bin_arrays_covering_range(&pool, -5664, -5600);
        // -5664 is in array -81 (see dlmm_decode's bin_id_to_bin_array_index tests), -5600
        // rolls into array -80.
        assert_eq!(arrays, vec![bin_array(&pool, -81), bin_array(&pool, -80)]);
    }

    #[test]
    fn test_bin_array_bitmap_extension_not_required_for_small_range() {
        assert!(!bin_array_bitmap_extension_required(-5664, -5600));
    }

    #[test]
    fn test_bin_array_bitmap_extension_required_past_default_window() {
        // Array index for bin id far past +/- 512 * 70.
        assert!(bin_array_bitmap_extension_required(0, 40_000));
        assert!(bin_array_bitmap_extension_required(-40_000, 0));
    }

    #[test]
    fn test_optional_bitmap_extension_uses_program_id_placeholder_when_unneeded() {
        let pool = pubkey(SOL_USDC_POOL);
        assert_eq!(
            optional_bin_array_bitmap_extension(&pool, -5664, -5600),
            dlmm_decode::ID
        );
        assert_eq!(
            optional_bin_array_bitmap_extension(&pool, 0, 40_000),
            bin_array_bitmap_extension(&pool)
        );
    }

    #[test]
    fn test_pda_derivations_are_deterministic() {
        let pool = pubkey(SOL_USDC_POOL);
        assert_eq!(bin_array(&pool, 5), bin_array(&pool, 5));
        assert_eq!(event_authority(), event_authority());
        assert_ne!(bin_array(&pool, 5), bin_array(&pool, 6));
    }
}
