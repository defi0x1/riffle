use lb_clmm::state::LbPair;
use lb_clmm::state::parameters::{StaticParameters, VariableParameters};

use crate::error::MathError;

const FEE_PRECISION: f64 = lb_clmm::constants::FEE_PRECISION as f64;

fn pair_with_params(
    bin_step_bps: u16,
    base_factor: u16,
    base_fee_power_factor: u8,
    variable_fee_control: u32,
    volatility_accumulator: u32,
) -> LbPair {
    LbPair {
        bin_step: bin_step_bps,
        parameters: StaticParameters {
            base_factor,
            base_fee_power_factor,
            variable_fee_control,
            ..Default::default()
        },
        v_parameters: VariableParameters {
            volatility_accumulator,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Base fee rate as a fraction: the base fee, `f_b = base_factor · s_bps · 10 · 10^pf / 1e9`.
///
/// Delegates to `lb_clmm`'s own `LbPair::calculate_base_fee`.
pub fn base_fee_rate(
    bin_step_bps: u16,
    base_factor: u16,
    base_fee_power_factor: u8,
) -> Result<f64, MathError> {
    let raw = LbPair::calculate_base_fee(base_factor, bin_step_bps, base_fee_power_factor)
        .map_err(|_| MathError::Overflow)?;
    Ok(raw as f64 / FEE_PRECISION)
}

/// Variable fee rate as a fraction for a given (integer) volatility accumulator: the variable fee.
/// `volatility_accumulator` is `lb_clmm`'s own unit (10,000 per bin crossed, the volatility accumulator).
///
/// Delegates to `lb_clmm`'s own fee pipeline by constructing the minimal `LbPair` state
/// the computation reads (`bin_step`, `variable_fee_control`, `volatility_accumulator`)
/// and calling its public `get_variable_fee`, so the integer rounding is bit-exact with
/// the program's.
pub fn variable_fee_rate(
    bin_step_bps: u16,
    variable_fee_control: u32,
    volatility_accumulator: u32,
) -> Result<f64, MathError> {
    let pair = pair_with_params(
        bin_step_bps,
        0,
        0,
        variable_fee_control,
        volatility_accumulator,
    );
    let raw = pair.get_variable_fee().map_err(|_| MathError::Overflow)?;
    Ok(raw as f64 / FEE_PRECISION)
}

/// the forecast fee: endogenous forecast fee `f̂` — the fee rate implied by a *forecast* volatility,
/// not the live on-chain accumulator. Our own forecast layered on `lb_clmm`'s exact
/// integer fee pipeline: we convert the forecast into an equivalent volatility
/// accumulator (`va = 10,000·k`) and let the program's own arithmetic do the rest, so
/// only the forecasting step (`E[k²]`) is ours.
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

    let pair = pair_with_params(
        bin_step_bps,
        base_factor,
        base_fee_power_factor,
        variable_fee_control,
        va_forecast,
    );
    let raw = pair.get_total_fee().map_err(|_| MathError::Overflow)?;
    Ok(raw as f64 / FEE_PRECISION)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_base_fee_rate_matches_bin_step_when_base_factor_is_10000() {
        // 00 / the base fee note: base_factor 10,000 -> base fee in bps == bin step in bps.
        let f_b = base_fee_rate(1, 10_000, 0).unwrap();
        assert!((f_b - 0.0001).abs() < 1e-12);

        let f_b = base_fee_rate(100, 10_000, 0).unwrap();
        assert!((f_b - 0.01).abs() < 1e-12);
    }

    #[test]
    fn test_endogenous_fee_rate_matches_worked_example_b_variable_component() {
        // Worked example B (worked example B): vfc = 40,000, s = 100 bps,
        // sigma_D = 150 bps, kappa_c = 3 -> f_v = 27 bp exactly:
        // E[k^2] = (150/100)^2 * 3 = 6.75; f_v = 40000 * 6.75 * 100^2 / 1e12 = 0.0027.
        let f_hat = endogenous_fee_rate(100, 0, 0, 40_000, 150.0, 3.0).unwrap();
        assert!((f_hat - 0.0027).abs() < 1e-6, "got {f_hat}");
    }

    #[test]
    fn test_endogenous_fee_rate_capped_at_ten_percent() {
        let f_hat = endogenous_fee_rate(100, 10_000, 0, 1_000_000, 100_000.0, 4.0).unwrap();
        assert!((f_hat - 0.10).abs() < 1e-9);
    }

    proptest! {
        // Our delegated `variable_fee_rate` must agree with `lb_clmm`'s own
        // `LbPair::get_variable_fee` bit-for-bit across the on-chain domain.
        #[test]
        fn prop_variable_fee_rate_matches_lb_clmm(
            bin_step in 1u16..=2_000,
            vfc in 0u32..=1_000_000,
            va in 0u32..=500_000,
        ) {
            let ours = variable_fee_rate(bin_step, vfc, va).unwrap();
            let pair = pair_with_params(bin_step, 0, 0, vfc, va);
            let theirs = pair.get_variable_fee().unwrap() as f64 / FEE_PRECISION;
            prop_assert!((ours - theirs).abs() < 1e-15);
        }

        #[test]
        fn prop_base_fee_rate_matches_lb_clmm(
            bin_step in 1u16..=2_000,
            base_factor in 0u16..=u16::MAX,
        ) {
            let ours = base_fee_rate(bin_step, base_factor, 0).unwrap();
            let theirs = LbPair::calculate_base_fee(base_factor, bin_step, 0).unwrap() as f64 / FEE_PRECISION;
            prop_assert!((ours - theirs).abs() < 1e-15);
        }
    }
}
