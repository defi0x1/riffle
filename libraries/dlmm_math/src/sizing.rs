/// the self-dilution cap: self-dilution cap — the position size at which marginal fee yield falls to the
/// daily hurdle `h`.
///
/// # Formula
///
/// * `m* = L̄_a · (√((1−ps)·f·τ_a / h) − 1)`, `h` = annual hurdle / 365
pub fn self_dilution_cap(
    active_bin_liquidity: f64,
    protocol_share: f64,
    fee_rate: f64,
    tau_a: f64,
    annual_hurdle: f64,
) -> f64 {
    let h = annual_hurdle / 365.0;
    let inner = (1.0 - protocol_share) * fee_rate * tau_a / h;
    active_bin_liquidity * (inner.sqrt() - 1.0)
}

/// Position share of active-bin liquidity cap, per bin: `m_share = θ_max · L̄_a`.
pub fn share_cap(active_bin_liquidity: f64, theta_max: f64) -> f64 {
    theta_max * active_bin_liquidity
}

/// Position share of pool TVL cap: `V_tvl = π_max · TVL`.
pub fn tvl_cap(tvl: f64, pi_max: f64) -> f64 {
    pi_max * tvl
}

/// Capital-at-risk cap: `V_car = fraction · C` (regime-bucket capital `C`).
pub fn car_cap(regime_capital: f64, fraction: f64) -> f64 {
    fraction * regime_capital
}

/// Kelly: quarter-Kelly fee-farming fraction, evaluated at the robust `σ_hi = 1.3·σ̂` (the
/// ~30% volatility estimation error,). The ¼ multiplier is a deliberate
/// haircut, not an estimate of correct leverage (Kelly note).
///
/// # Formula
///
/// * `f* = (μ_fee − μ_ARB) / σ_pos²`, `σ_pos ≈ 0.5·σ_hi`, `σ_hi = 1.3·σ̂`; return `f*/4`
pub fn quarter_kelly(mu_fee: f64, mu_arb: f64, sigma_hat: f64) -> f64 {
    let sigma_hi = 1.3 * sigma_hat;
    let sigma_pos = 0.5 * sigma_hi;
    let f_star = (mu_fee - mu_arb) / (sigma_pos * sigma_pos);
    f_star / 4.0
}

/// the self-dilution cap/Kelly composition: the position size is the minimum of every
/// applicable cap; the pool is skipped entirely if that minimum falls below `v_min`.
pub fn position_size(caps: &[f64], v_min: f64) -> Option<f64> {
    let v = caps.iter().cloned().fold(f64::INFINITY, f64::min);
    if v.is_finite() && v >= v_min {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_self_dilution_cap_hand_checked() {
        // (1-0)*0.01*250/0.1 = 25 -> sqrt = 5 -> m* = 100*(5-1) = 400
        let m = self_dilution_cap(100.0, 0.0, 0.01, 250.0, 36.5);
        assert!((m - 400.0).abs() < 1e-6, "got {m}");
    }

    #[test]
    fn test_share_cap_matches_worked_example_a() {
        // worked example A: N*m_share = 5*0.15*800k = $600k -> per-bin m_share = $120k.
        let m_share = share_cap(800_000.0, 0.15);
        assert!((m_share - 120_000.0).abs() < 1e-6);
        assert!((5.0 * m_share - 600_000.0).abs() < 1e-6);
    }

    #[test]
    fn test_tvl_cap_matches_worked_example_a() {
        let v_tvl = tvl_cap(5_000_000.0, 0.10);
        assert!((v_tvl - 500_000.0).abs() < 1e-6, "got {v_tvl}");
    }

    #[test]
    fn test_car_cap_matches_worked_example_a() {
        let v_car = car_cap(200_000.0, 0.40);
        assert!((v_car - 80_000.0).abs() < 1e-6, "got {v_car}");
    }

    #[test]
    fn test_quarter_kelly_hand_checked() {
        // sigma_hi = 1.3*0.1 = 0.13, sigma_pos = 0.065, f* = 0.08/0.065^2 = 18.9349...
        let q = quarter_kelly(0.10, 0.02, 0.10);
        assert!((q - 4.7337).abs() < 1e-3, "got {q}");
    }

    #[test]
    fn test_position_size_matches_worked_example_a() {
        // worked example A5: caps 600k/500k/80k, bucket free 50k -> V* = $50,000.
        let v = position_size(&[600_000.0, 500_000.0, 80_000.0, 50_000.0], 5_000.0);
        assert_eq!(v, Some(50_000.0));
    }

    #[test]
    fn test_position_size_skips_when_below_v_min() {
        let v = position_size(&[10_000.0, 800.0], 1_000.0);
        assert_eq!(v, None);
    }
}
