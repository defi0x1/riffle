/// Loss-versus-rebalancing rate for a Spot position: the cost per unit time
/// while in range; zero when out of range (caller's responsibility to gate on that).
///
/// # Formula
///
/// * `ℓ_spot = σ² V / (2w)`, `w = N·s` = range width as a fraction
pub fn lvr_rate_spot(sigma: f64, position_value: f64, width: f64) -> f64 {
    sigma * sigma * position_value / (2.0 * width)
}

/// Impermanent loss vs HODL for a symmetric Spot position after a move `Δ`, `|Δ| ≤ W`
///. At the edge (`Δ = W = w/2`) this reduces to `V·w/8`.
///
/// # Formula
///
/// * `IL(Δ) = V Δ² / (2w)`
pub fn il_spot(position_value: f64, delta: f64, width: f64) -> f64 {
    position_value * delta * delta / (2.0 * width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lvr_rate_spot_matches_00_shared_parameters_example() {
        // the shared parameters: sigma_d = 2e-4, V = 50,000, w = 2*5e-4 -> $2.00/day.
        let ell = lvr_rate_spot(2e-4, 50_000.0, 5e-4);
        assert!((ell - 2.0).abs() < 1e-9, "got {ell}");
    }

    #[test]
    fn test_il_spot_at_edge_equals_v_w_over_8() {
        let v = 1_000.0;
        let w = 0.02; // full width
        let half_width = w / 2.0;
        let il = il_spot(v, half_width, w);
        assert!((il - v * w / 8.0).abs() < 1e-12);
    }

    #[test]
    fn test_il_spot_zero_move_is_zero() {
        assert!((il_spot(1_000.0, 0.0, 0.02) - 0.0).abs() < 1e-15);
    }
}
