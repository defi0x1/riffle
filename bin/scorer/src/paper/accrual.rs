//! Pure per-mark estimates, kept separate from the storage calls that supply their inputs.

/// Estimated in-range fee share for a paper position over one mark interval: its size
/// relative to the active-bin liquidity it would be diluting, applied to the pool's own fee
/// income over the interval. This is an estimate, not the true per-bin
/// `fee_*_per_token_stored` delta (`bin_states` would be needed for that), which is
/// reasonable here: no chain interaction happens in this worker at all, and the estimate is
/// applied only while the position is in range.
pub fn estimated_fee_share(
    pool_trade_fee_usd: f64,
    size_per_bin: f64,
    active_bin_liquidity: f64,
    in_range: bool,
) -> f64 {
    if !in_range || pool_trade_fee_usd <= 0.0 || size_per_bin <= 0.0 {
        return 0.0;
    }
    let denom = size_per_bin + active_bin_liquidity;
    if denom <= 0.0 {
        return 0.0;
    }
    pool_trade_fee_usd * (size_per_bin / denom)
}

pub fn is_in_range(active_bin: i32, lower_bin: i32, upper_bin: i32) -> bool {
    active_bin >= lower_bin && active_bin <= upper_bin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_out_of_range_earns_nothing() {
        assert_eq!(estimated_fee_share(1_000.0, 50.0, 500.0, false), 0.0);
    }

    #[test]
    fn test_in_range_share_scales_with_position_size() {
        let small = estimated_fee_share(1_000.0, 10.0, 990.0, true);
        let large = estimated_fee_share(1_000.0, 500.0, 500.0, true);
        assert!(
            large > small,
            "a larger share of active-bin liquidity should earn more"
        );
        assert!((large - 500.0).abs() < 1e-9); // 500/(500+500) * 1000 = 500
    }

    #[test]
    fn test_in_range_bounds() {
        assert!(is_in_range(100, 90, 110));
        assert!(!is_in_range(89, 90, 110));
        assert!(!is_in_range(111, 90, 110));
    }
}
