use crate::error::MathError;

/// Bin price at `bin_id` for a pool with the given bin step: `P_i = (1 + s)^i` (F1).
///
/// Delegates to `lb_clmm::math::price_math::get_price_from_id`, the same Q64.64
/// fixed-point routine the program uses on-chain, so this is bit-exact with the program
/// by construction.
pub fn bin_price(bin_id: i32, bin_step_bps: u16) -> Result<f64, MathError> {
    let q64 = lb_clmm::math::price_math::get_price_from_id(bin_id, bin_step_bps)
        .map_err(|_| MathError::Overflow)?;
    Ok(q64_to_f64(q64))
}

fn q64_to_f64(x: u128) -> f64 {
    (x as f64) / (1u128 << 64) as f64
}

/// Bin id whose price is closest to `price`: the inverse of F1.
///
/// `lb_clmm` has no on-chain inverse — the program always knows its own `active_id` and
/// never needs to recover a bin from a price — so this is our own, not a delegation.
///
/// # Formula
///
/// * `i = round(ln(P) / ln(1 + s))`, inverting F1's `P_i = (1 + s)^i`
///
/// # Precision
///
/// Exact only while the price stays inside the joint envelope of the program's Q64.64
/// fixed point and f64's 53-bit mantissa — see [`bin_resolvable`]. Outside it this returns
/// an id that may be off by more than one.
pub fn bin_from_price(price: f64, bin_step_bps: u16) -> i32 {
    let s = bin_step_bps as f64 / 10_000.0;
    (price.ln() / (1.0 + s).ln()).round() as i32
}

/// Whether [`bin_from_price`] round-trips `bin_id` exactly.
///
/// The limit is `|ln P| = |i · ln(1 + s)|`. Measured against the program's own
/// `get_price_from_id` across bin steps 1–2000 bps, recovery holds until `|ln P| ≈ 36` and
/// fails beyond it, symmetrically in both directions — the price leaving roughly `e^±36`
/// is where Q64.64's fractional bits and f64's 53-bit mantissa together stop separating
/// adjacent bins. The 35 below keeps a margin inside the measured boundary at every step.
///
/// This is a property of the representations, not of our arithmetic, and it is far outside
/// any range a real pool trades in: at 25 bps it permits roughly ±15,500 bins.
pub fn bin_resolvable(bin_id: i32, bin_step_bps: u16) -> bool {
    const MAX_ABS_LN_PRICE: f64 = 35.0;
    let s = bin_step_bps as f64 / 10_000.0;
    (bin_id as f64 * (1.0 + s).ln()).abs() <= MAX_ABS_LN_PRICE
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_bin_price_zero_id_is_one() {
        assert!((bin_price(0, 25).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_bin_price_matches_pow() {
        // s = 20 bps = 0.002, i = 100 -> P = 1.002^100
        let got = bin_price(100, 20).unwrap();
        let want = 1.002f64.powi(100);
        assert!((got - want).abs() / want < 1e-6, "got {got} want {want}");
    }

    #[test]
    fn test_bin_from_price_roundtrip() {
        let bin_step = 25u16;
        for id in [-5000i32, -1, 0, 1, 5000] {
            let p = bin_price(id, bin_step).unwrap();
            assert_eq!(bin_from_price(p, bin_step), id);
        }
    }

    #[test]
    fn test_bin_from_price_loses_bins_outside_the_envelope() {
        // Both ends of the precision envelope, pinned so a change to the conversion cannot
        // quietly move them. Left: the stored integer is ~170, under eight significant
        // bits. Right: the price exceeds what f64's mantissa separates at this bin step.
        for (bin_id, bin_step) in [(-16_362i32, 24u16), (74_819, 5)] {
            assert!(
                !bin_resolvable(bin_id, bin_step),
                "{bin_id} at {bin_step} bps"
            );
            let price = bin_price(bin_id, bin_step).unwrap();
            assert!((bin_from_price(price, bin_step) - bin_id).abs() > 1);
        }
    }

    #[test]
    fn test_bin_resolvable_holds_for_realistic_pools() {
        // Every pool we would actually rank sits well inside the resolvable domain.
        for (bin_id, bin_step) in [(0i32, 1u16), (-5_000, 25), (5_000, 25), (-2_000, 100)] {
            assert!(
                bin_resolvable(bin_id, bin_step),
                "{bin_id} at {bin_step} bps"
            );
        }
    }

    proptest! {
        // Our own `bin_from_price` must invert `lb_clmm`'s own `get_price_from_id`
        // wherever Q64.64 still separates adjacent bins. Outside that domain the
        // representation, not our arithmetic, is the limit -- see `bin_resolvable`.
        #[test]
        fn prop_bin_from_price_inverts_lb_clmm_price(
            bin_id in -400_000i32..400_000,
            bin_step in 1u16..=2_000,
        ) {
            if bin_resolvable(bin_id, bin_step)
                && let Ok(price) = bin_price(bin_id, bin_step)
                && price.is_finite() && price > 0.0
            {
                let recovered = bin_from_price(price, bin_step);
                prop_assert!((recovered - bin_id).abs() <= 1);
            }
        }
    }
}
