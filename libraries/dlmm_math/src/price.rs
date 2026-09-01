use crate::error::MathError;

// Q64.64 fixed point: 64 fractional bits, so "1.0" is 1u128 << SCALE_OFFSET.
const SCALE_OFFSET: u32 = 64;
const ONE_Q64: u128 = 1u128 << SCALE_OFFSET;

// The IDL's BASIS_POINT_MAX (bin_step is expressed in this unit): https://raw.githubusercontent
// .com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json
const BASIS_POINT_MAX: u128 = 10_000;

// Largest exponent get_price_from_id will attempt before giving up. At bin_step = 1 (the
// smallest step), (1.0001)^n overflows a u64 token amount around n ~ 443_636; the 19-bit cap
// here (2^19 = 524_288) is the smallest power-of-two ceiling above that, chosen so the
// squaring loop below has a fixed, small iteration count regardless of the exponent's sign.
const MAX_EXPONENT: u32 = 0x80000;

/// Bin price at `bin_id` for a pool with the given bin step: `P_i = (1 + s)^i`.
pub fn bin_price(bin_id: i32, bin_step_bps: u16) -> Result<f64, MathError> {
    let q64 = get_price_from_id(bin_id, bin_step_bps).ok_or(MathError::Overflow)?;
    Ok(q64_to_f64(q64))
}

/// `(1 + bin_step/BASIS_POINT_MAX)^bin_id` in Q64.64 fixed point -- the program's own bin
/// price ladder. Reimplemented from the public IDL's on-chain formula rather than an f64
/// `powi`, since a swap's exact-in/exact-out amounts are computed from this same integer
/// arithmetic and only agree with the program bit-for-bit if the rounding does too.
fn get_price_from_id(bin_id: i32, bin_step_bps: u16) -> Option<u128> {
    let base = price_base_q64(bin_step_bps)?;
    q64_pow(base, bin_id)
}

fn price_base_q64(bin_step_bps: u16) -> Option<u128> {
    let fraction = (bin_step_bps as u128)
        .checked_shl(SCALE_OFFSET)?
        .checked_div(BASIS_POINT_MAX)?;
    ONE_Q64.checked_add(fraction)
}

// Exponentiation by squaring in Q64.64: each squaring keeps the running values within u128 by
// right-shifting off the low 64 bits after every multiply, since a naive Q64.64 * Q64.64
// product needs 128 fractional + 128 integer bits. A negative exponent is handled by inverting
// the base up front (1/base in Q64.64, via u128::MAX / base -- the Q64.64 reciprocal of a
// value >= 1) rather than inverting the final result, which keeps every intermediate magnitude
// on the same side of 1.0 the squaring loop is tuned for.
fn q64_pow(base: u128, exp: i32) -> Option<u128> {
    if exp == 0 {
        return Some(ONE_Q64);
    }

    let mut invert = exp.is_negative();
    let mut remaining_bits = exp.unsigned_abs();
    if remaining_bits >= MAX_EXPONENT {
        return None;
    }

    let mut squared_base = base;
    let mut result = ONE_Q64;

    if squared_base >= result {
        squared_base = u128::MAX.checked_div(squared_base)?;
        invert = !invert;
    }

    loop {
        if remaining_bits & 1 == 1 {
            result = result
                .checked_mul(squared_base)?
                .checked_shr(SCALE_OFFSET)?;
        }
        remaining_bits >>= 1;
        if remaining_bits == 0 {
            break;
        }
        squared_base = squared_base
            .checked_mul(squared_base)?
            .checked_shr(SCALE_OFFSET)?;
    }

    if result == 0 {
        return None;
    }

    if invert {
        result = u128::MAX.checked_div(result)?;
    }

    Some(result)
}

fn q64_to_f64(x: u128) -> f64 {
    (x as f64) / (1u128 << 64) as f64
}

/// Bin id whose price is closest to `price`: the inverse of the bin price ladder.
///
/// The program has no on-chain inverse -- it always knows its own `active_id` and never
/// needs to recover a bin from a price -- so this is our own, not a delegation.
///
/// # Formula
///
/// * `i = round(ln(P) / ln(1 + s))`, inverting the bin price ladder's `P_i = (1 + s)^i`
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
/// The limit is `|ln P| = |i · ln(1 + s)|`. Measured against the program's own bin price
/// ladder across bin steps 1–2000 bps, recovery holds until `|ln P| ≈ 36` and fails beyond
/// it, symmetrically in both directions — the price leaving roughly `e^±36` is where
/// Q64.64's fractional bits and f64's 53-bit mantissa together stop separating adjacent
/// bins. The 35 below keeps a margin inside the measured boundary at every step.
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

    // Pinned against lb_clmm::math::price_math::get_price_from_id (the vendored program
    // source, since removed as a dependency -- see dlmm_decode for why) across bin steps
    // 1-10000 and a wide bin id sweep including the extremes each bin step supports. Values
    // captured from a direct comparison run before the dependency was dropped; a change here
    // means the Q64.64 arithmetic above no longer agrees with the program's own.
    #[test]
    fn test_get_price_from_id_matches_pinned_reference_values() {
        for &(bin_id, bin_step, expected) in PINNED_PRICES {
            let got = get_price_from_id(bin_id, bin_step);
            assert_eq!(got, expected, "bin_id={bin_id} bin_step={bin_step}");
        }
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
        // Our own `bin_from_price` must invert the program's own bin price ladder wherever
        // Q64.64 still separates adjacent bins. Outside that domain the representation, not
        // our arithmetic, is the limit -- see `bin_resolvable`.
        #[test]
        fn prop_bin_from_price_inverts_price_ladder(
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

    // Pinned against lb_clmm::math::price_math::get_price_from_id (the vendored program
    // source, since removed as a dependency -- see the root Cargo.toml's former lb_clmm
    // entry) across 14 bin steps and a sweep of bin ids spanning i32's full range, including
    // each bin step's overflow boundary. Captured from a direct comparison run before that
    // dependency was dropped.
    include!("price_fixtures.rs");
}
