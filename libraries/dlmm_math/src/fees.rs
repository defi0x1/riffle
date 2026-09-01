use crate::error::MathError;

// Fixed-point denominator for base/variable fee rates. The IDL calls this FEE_DENOMINATOR:
// https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json
const FEE_PRECISION: f64 = 1_000_000_000.0;

// Cap on total_fee_rate = base + variable, same FEE_PRECISION units. IDL constant MAX_FEE_RATE.
const MAX_FEE_RATE: u128 = 100_000_000;

fn checked_base_fee(
    base_factor: u16,
    bin_step_bps: u16,
    base_fee_power_factor: u8,
) -> Option<u128> {
    (base_factor as u128)
        .checked_mul(bin_step_bps as u128)?
        .checked_mul(10)?
        .checked_mul(10u128.checked_pow(base_fee_power_factor as u32)?)
}

fn checked_variable_fee(
    bin_step_bps: u16,
    variable_fee_control: u32,
    volatility_accumulator: u32,
) -> Option<u128> {
    if variable_fee_control == 0 {
        return Some(0);
    }

    let square_vfa_bin = (volatility_accumulator as u128)
        .checked_mul(bin_step_bps as u128)?
        .checked_pow(2)?;
    let v_fee = (variable_fee_control as u128).checked_mul(square_vfa_bin)?;

    // 1e20-ish raw units (variable_fee_control, volatility_accumulator and bin_step are all
    // basis-point scale) scaled down to FEE_PRECISION (1e9) units, rounded up.
    v_fee
        .checked_add(99_999_999_999)?
        .checked_div(100_000_000_000)
}

/// Base fee rate as a fraction: `f_b = base_factor · s_bps · 10 · 10^pf / 1e9`.
pub fn base_fee_rate(
    bin_step_bps: u16,
    base_factor: u16,
    base_fee_power_factor: u8,
) -> Result<f64, MathError> {
    let raw = checked_base_fee(base_factor, bin_step_bps, base_fee_power_factor)
        .ok_or(MathError::Overflow)?;
    Ok(raw as f64 / FEE_PRECISION)
}

/// Variable fee rate as a fraction for a given (integer) volatility accumulator.
/// `volatility_accumulator` is in the program's own unit (10,000 per bin crossed).
pub fn variable_fee_rate(
    bin_step_bps: u16,
    variable_fee_control: u32,
    volatility_accumulator: u32,
) -> Result<f64, MathError> {
    let raw = checked_variable_fee(bin_step_bps, variable_fee_control, volatility_accumulator)
        .ok_or(MathError::Overflow)?;
    Ok(raw as f64 / FEE_PRECISION)
}

/// The forecast fee: endogenous forecast fee `f̂` — the fee rate implied by a *forecast*
/// volatility, not the live on-chain accumulator. Our own forecast layered on the program's
/// exact integer fee pipeline: we convert the forecast into an equivalent volatility
/// accumulator (`va = 10,000·k`) and let the same arithmetic as `variable_fee_rate` do the
/// rest, so only the forecasting step (`E[k²]`) is ours.
///
/// # Formula
///
/// * `f̂ = min(f_b + f_v̂, 10%)`, `f_v̂ = vfc · E[k²] · s_bps² / 1e12`,
///   `E[k²] ≈ (σ_D/s)² · κ_c`
pub fn endogenous_fee_rate(
    bin_step_bps: u16,
    base_factor: u16,
    base_fee_power_factor: u8,
    variable_fee_control: u32,
    sigma_d_bps: f64,
    kappa_c: f64,
) -> Result<f64, MathError> {
    let s_bps = bin_step_bps as f64;
    let e_k_sq = (sigma_d_bps / s_bps).powi(2) * kappa_c;
    let k_forecast = e_k_sq.sqrt();
    let va_forecast = (10_000.0 * k_forecast).round() as u32;

    let base = checked_base_fee(base_factor, bin_step_bps, base_fee_power_factor)
        .ok_or(MathError::Overflow)?;
    let variable = checked_variable_fee(bin_step_bps, variable_fee_control, va_forecast)
        .ok_or(MathError::Overflow)?;
    let total = base.checked_add(variable).ok_or(MathError::Overflow)?;
    let capped = total.min(MAX_FEE_RATE);

    Ok(capped as f64 / FEE_PRECISION)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_base_fee_rate_matches_bin_step_when_base_factor_is_10000() {
        // base_factor 10,000 -> base fee in bps == bin step in bps.
        let f_b = base_fee_rate(1, 10_000, 0).unwrap();
        assert!((f_b - 0.0001).abs() < 1e-12);

        let f_b = base_fee_rate(100, 10_000, 0).unwrap();
        assert!((f_b - 0.01).abs() < 1e-12);
    }

    #[test]
    fn test_endogenous_fee_rate_matches_worked_example_b_variable_component() {
        // Worked example B: vfc = 40,000, s = 100 bps, sigma_D = 150 bps, kappa_c = 3 ->
        // f_v = 27 bp exactly: E[k^2] = (150/100)^2 * 3 = 6.75;
        // f_v = 40000 * 6.75 * 100^2 / 1e12 = 0.0027.
        let f_hat = endogenous_fee_rate(100, 0, 0, 40_000, 150.0, 3.0).unwrap();
        assert!((f_hat - 0.0027).abs() < 1e-6, "got {f_hat}");
    }

    #[test]
    fn test_endogenous_fee_rate_capped_at_ten_percent() {
        let f_hat = endogenous_fee_rate(100, 10_000, 0, 1_000_000, 100_000.0, 4.0).unwrap();
        assert!((f_hat - 0.10).abs() < 1e-9);
    }

    // Pinned against lb_clmm's LbPair::calculate_base_fee / get_variable_fee (the vendored
    // program source, since removed as a dependency) across a wide sweep of bin steps, base
    // factors and volatility accumulators. Captured from a direct comparison run before that
    // dependency was dropped.
    include!("fee_fixtures.rs");

    #[test]
    fn test_checked_base_fee_matches_pinned_reference_values() {
        for &(base_factor, bin_step, power_factor, expected) in PINNED_BASE_FEES {
            assert_eq!(
                checked_base_fee(base_factor, bin_step, power_factor),
                expected,
                "base_factor={base_factor} bin_step={bin_step} power_factor={power_factor}"
            );
        }
    }

    #[test]
    fn test_checked_variable_fee_matches_pinned_reference_values() {
        for &(bin_step, vfc, va, expected) in PINNED_VARIABLE_FEES {
            assert_eq!(
                checked_variable_fee(bin_step, vfc, va),
                expected,
                "bin_step={bin_step} vfc={vfc} va={va}"
            );
        }
    }

    proptest! {
        // variable_fee_rate must stay self-consistent with checked_variable_fee, which is
        // the function pinned against lb_clmm above -- this exercises the f64 conversion on
        // top of it across the on-chain domain.
        #[test]
        fn prop_variable_fee_rate_matches_checked_variable_fee(
            bin_step in 1u16..=2_000,
            vfc in 0u32..=1_000_000,
            va in 0u32..=500_000,
        ) {
            let ours = variable_fee_rate(bin_step, vfc, va).unwrap();
            let raw = checked_variable_fee(bin_step, vfc, va).unwrap();
            prop_assert!((ours - raw as f64 / FEE_PRECISION).abs() < 1e-15);
        }

        #[test]
        fn prop_base_fee_rate_matches_checked_base_fee(
            bin_step in 1u16..=2_000,
            base_factor in 0u16..=u16::MAX,
        ) {
            let ours = base_fee_rate(bin_step, base_factor, 0).unwrap();
            let raw = checked_base_fee(base_factor, bin_step, 0).unwrap();
            prop_assert!((ours - raw as f64 / FEE_PRECISION).abs() < 1e-15);
        }
    }
}
