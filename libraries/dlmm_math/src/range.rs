/// Minimum number of bins on each side of the active bin, in every regime
/// ("N ≥ 10 in every regime").
pub const MIN_BIN_COUNT: u32 = 10;

/// Range half-width as a fraction of price: `W = 1.5·σ_d·√T`, `T` in days.
pub fn range_half_width(sigma_d: f64, horizon_days: f64) -> f64 {
    1.5 * sigma_d * horizon_days.sqrt()
}

/// Number of bins needed to cover half-width `w_half` at bin step `bin_step` (as a
/// fraction), floored at [`MIN_BIN_COUNT`].
pub fn bin_count_for_half_width(w_half: f64, bin_step: f64) -> u32 {
    let n = (w_half / bin_step).ceil();
    let n = if n.is_finite() && n > 0.0 {
        n as u32
    } else {
        0
    };
    n.max(MIN_BIN_COUNT)
}

/// expected time in range: expected time to exit a symmetric range (Brownian, no drift, start at center).
///
/// # Formula
///
/// * `E[T_exit] = W²/σ²`
pub fn expected_time_to_exit(half_width: f64, sigma: f64) -> f64 {
    (half_width * half_width) / (sigma * sigma)
}

/// expected time in range with an early trigger at `α·W` (`α ∈ [0.6, 1]`, `02`).
///
/// # Formula
///
/// * `E[T_trigger] = (αW)²/σ²`
pub fn expected_time_to_trigger(alpha: f64, half_width: f64, sigma: f64) -> f64 {
    let a_w = alpha * half_width;
    (a_w * a_w) / (sigma * sigma)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_half_width_matches_worked_example_b() {
        // worked example B: W = 1.5*sigma_12h = 1.5*18%*sqrt(0.5) ~= 19%.
        let w = range_half_width(0.18, 0.5);
        assert!((w - 0.190919).abs() < 1e-5, "got {w}");
    }

    #[test]
    fn test_bin_count_for_half_width_matches_worked_example_b() {
        // W ~= 19.09%, s = 1% -> ceil(19.09) = 20 bins per side (N = 40 total).
        let w = range_half_width(0.18, 0.5);
        let n = bin_count_for_half_width(w, 0.01);
        assert_eq!(n, 20);
    }

    #[test]
    fn test_bin_count_floors_at_min_bin_count() {
        // A quiet pool: tiny width, coarse bin step -> would compute far below 10.
        let n = bin_count_for_half_width(0.0005, 0.01);
        assert_eq!(n, MIN_BIN_COUNT);
    }

    #[test]
    fn test_expected_time_to_exit_hand_checked() {
        // W = sigma = 0.1 -> E[T] = 0.01/0.01 = 1.0 day.
        let t = expected_time_to_exit(0.1, 0.1);
        assert!((t - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_expected_time_to_trigger_matches_worked_example_b() {
        // worked example B2: E[T_rebalance] = (0.7*0.2)^2/0.0324 ~= 0.6 d.
        let t = expected_time_to_trigger(0.7, 0.2, 0.18);
        assert!((t - 0.6049).abs() < 1e-3, "got {t}");
    }
}
