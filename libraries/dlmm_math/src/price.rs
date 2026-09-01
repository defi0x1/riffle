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
pub fn bin_from_price(price: f64, bin_step_bps: u16) -> i32 {
    let s = bin_step_bps as f64 / 10_000.0;
    (price.ln() / (1.0 + s).ln()).round() as i32
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

    proptest! {
        // Our own `bin_from_price` must invert `lb_clmm`'s own `get_price_from_id`
        // across the domain a real pool can use.
        #[test]
        fn prop_bin_from_price_inverts_lb_clmm_price(
            bin_id in -400_000i32..400_000,
            bin_step in 1u16..=2_000,
        ) {
            if let Ok(price) = bin_price(bin_id, bin_step)
                && price.is_finite() && price > 0.0
            {
                let recovered = bin_from_price(price, bin_step);
                prop_assert!((recovered - bin_id).abs() <= 1);
            }
        }
    }
}
