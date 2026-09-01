use solana_sdk::pubkey::Pubkey;

// Values below are transcribed from the public IDL, not the vendored program source:
// https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json

/// The deployed DLMM program's address (IDL `address` field).
pub const ID: Pubkey = solana_sdk::pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");

/// PDA seed prefix for a BinArray account: `[BIN_ARRAY, lb_pair, index.to_le_bytes()]`.
pub const BIN_ARRAY: &[u8] = b"bin_array";

/// Basis point denominator used throughout the program's fee math.
pub const BASIS_POINT_MAX: u32 = 10_000;

/// Fixed-point denominator for base/variable fee rates -- the IDL's constant of this value
/// is named FEE_DENOMINATOR; callers here keep calling it FEE_PRECISION.
pub const FEE_PRECISION: u64 = 1_000_000_000;

/// Bins packed into one BinArray account.
pub const MAX_BIN_PER_ARRAY: usize = 70;

/// Which BinArray index a bin id falls in: `bin_id.div_euclid(MAX_BIN_PER_ARRAY)`, spelled
/// out because plain `/` truncates toward zero while the on-chain index floors -- they only
/// disagree when bin_id is negative and not an exact multiple of MAX_BIN_PER_ARRAY.
pub fn bin_id_to_bin_array_index(bin_id: i32) -> i32 {
    let size = MAX_BIN_PER_ARRAY as i32;
    let quotient = bin_id / size;
    let remainder = bin_id % size;
    if bin_id.is_negative() && remainder != 0 {
        quotient - 1
    } else {
        quotient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bin_id_to_bin_array_index_matches_known_fixture() {
        // bin -5664 (the SOL-USDC pool's active bin) sits in array -81, per the golden
        // BinArray fixture: -81 * 70 = -5670 <= -5664 <= -5601.
        assert_eq!(bin_id_to_bin_array_index(-5664), -81);
    }

    #[test]
    fn test_bin_id_to_bin_array_index_floors_at_the_boundary() {
        assert_eq!(bin_id_to_bin_array_index(-70), -1);
        assert_eq!(bin_id_to_bin_array_index(-69), -1);
        assert_eq!(bin_id_to_bin_array_index(-1), -1);
        assert_eq!(bin_id_to_bin_array_index(0), 0);
        assert_eq!(bin_id_to_bin_array_index(69), 0);
        assert_eq!(bin_id_to_bin_array_index(70), 1);
    }
}
