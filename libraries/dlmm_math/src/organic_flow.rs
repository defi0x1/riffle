/// F11: mechanical toxic-volume estimator, needing no swap-level data (available from
/// day 1 on a cold pool). `c_fill` is the calibration constant ([A-02-3]: 0.5 for V1/V2,
/// 0.75 for S).
///
/// # Formula
///
/// * crossings/day `≈ σ_d² / s²`
/// * `Vol_toxic^mech ≈ (σ_d²/s²) · L̄_a · c_fill`
/// * `phi_mech = clip(1 − Vol_toxic^mech / Vol_24h, 0, 1)` (F11)
pub fn phi_mech(
    sigma_d: f64,
    bin_step: f64,
    active_bin_liquidity: f64,
    c_fill: f64,
    vol_24h: f64,
) -> f64 {
    if vol_24h <= 0.0 {
        return 0.0;
    }
    let crossings_per_day = (sigma_d / bin_step).powi(2);
    let vol_toxic = crossings_per_day * active_bin_liquidity * c_fill;
    (1.0 - vol_toxic / vol_24h).clamp(0.0, 1.0)
}

/// Two-component exponential mixture over trade sizes, fit by EM (plans/04 §4.3: "Two-
/// component mixture fit on trade sizes"). Arb flow clusters near the one-bin size, so
/// the *low-mean* component is organic; returns its weight, or `None` if there are too
/// few observations to fit.
pub fn phi_size(trade_sizes: &[f64]) -> Option<f64> {
    const ITERATIONS: usize = 200;

    if trade_sizes.len() < 4 {
        return None;
    }
    let mean = trade_sizes.iter().sum::<f64>() / trade_sizes.len() as f64;
    if mean <= 0.0 {
        return None;
    }

    let mut rate_small = 1.0 / (mean * 0.5);
    let mut rate_large = 1.0 / (mean * 1.5);
    let mut weight_small = 0.5_f64;

    for _ in 0..ITERATIONS {
        let mut sum_r = 0.0;
        let mut sum_r_x = 0.0;
        let mut sum_1mr = 0.0;
        let mut sum_1mr_x = 0.0;

        for &x in trade_sizes {
            let p_small = weight_small * rate_small * (-rate_small * x).exp();
            let p_large = (1.0 - weight_small) * rate_large * (-rate_large * x).exp();
            let r = if p_small + p_large > 0.0 {
                p_small / (p_small + p_large)
            } else {
                0.5
            };
            sum_r += r;
            sum_r_x += r * x;
            sum_1mr += 1.0 - r;
            sum_1mr_x += (1.0 - r) * x;
        }

        let n = trade_sizes.len() as f64;
        weight_small = (sum_r / n).clamp(1e-6, 1.0 - 1e-6);
        if sum_r_x > 0.0 {
            rate_small = sum_r / sum_r_x;
        }
        if sum_1mr_x > 0.0 {
            rate_large = sum_1mr / sum_1mr_x;
        }
    }

    // Organic component is whichever fitted component has the larger rate (smaller mean).
    if rate_small >= rate_large {
        Some(weight_small)
    } else {
        Some(1.0 - weight_small)
    }
}

/// Blend of the three organic-flow estimators (plans/04 §4.4). `phi_time` is `None` on
/// the ingestion backend that has no swap-level data (`Source::capabilities().swap_level_events`
/// gates it upstream); `n_trades` is then naturally 0 and the blend collapses to the
/// mech/size combination — the spec's own defined degradation, not an improvisation.
///
/// # Formula
///
/// * `w_time = 0.5 · min(1, n_trades/200)`
/// * `phi_obs = w_time·phi_time + (1−w_time)·(0.6·phi_mech + 0.4·phi_size)`
pub fn phi_org_blend(phi_time: Option<f64>, n_trades: u32, phi_mech: f64, phi_size: f64) -> f64 {
    let w_time = match phi_time {
        Some(_) => 0.5 * (n_trades as f64 / 200.0).min(1.0),
        None => 0.0,
    };
    let mech_size = 0.6 * phi_mech + 0.4 * phi_size;
    w_time * phi_time.unwrap_or(0.0) + (1.0 - w_time) * mech_size
}

/// Empirical-Bayes shrinkage of the observed organic share toward its class prior
/// (plans/04 §4.4): a pool with few classified trades is pulled toward `mu_c` rather
/// than trusted outright. `n = 0` returns the prior unchanged.
///
/// # Formula
///
/// * `phi_hat = B·μ_c + (1−B)·phi_obs`, `B = (s²/n) / (s²/n + τ_c²)`
pub fn shrink_phi_org(phi_obs: f64, n: u32, sample_variance: f64, mu_c: f64, tau_c_sq: f64) -> f64 {
    if n == 0 {
        return mu_c;
    }
    let se_sq = sample_variance / n as f64;
    let b = se_sq / (se_sq + tau_c_sq);
    b * mu_c + (1.0 - b) * phi_obs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phi_mech_matches_worked_example_a() {
        // 10-worked-examples.md A.1 / 00 §0.6: sigma_d=2bp, s=1bp, L_a=800k, c_fill=0.75 (S), Vol_24h=25M.
        let phi = phi_mech(2e-4, 1e-4, 800_000.0, 0.75, 25_000_000.0);
        assert!((phi - 0.904).abs() < 1e-3, "got {phi}");
    }

    #[test]
    fn test_phi_mech_matches_worked_example_b() {
        // 10-worked-examples.md B.1: sigma_slow=18%, s=100bp, L_a=12k, c_fill=0.5 (V2), Vol_24h=4.5M.
        let phi = phi_mech(0.18, 0.01, 12_000.0, 0.5, 4_500_000.0);
        assert!((phi - 0.568).abs() < 1e-3, "got {phi}");
    }

    #[test]
    fn test_phi_mech_clips_at_zero_when_toxic_exceeds_volume() {
        let phi = phi_mech(1.0, 0.01, 1_000_000.0, 1.0, 1.0);
        assert!((phi - 0.0).abs() < 1e-12);
    }

    #[test]
    fn test_phi_size_separates_two_clusters() {
        // Deterministic pseudo-exponential samples (inverse-CDF over a hashed sequence):
        // 700 draws with mean ~10 (organic), 300 draws with mean ~1000 (near one-bin, toxic).
        let mut samples = Vec::new();
        for i in 0..700u64 {
            samples.push(pseudo_exponential(i, 10.0));
        }
        for i in 0..300u64 {
            samples.push(pseudo_exponential(1_000_000 + i, 1000.0));
        }
        let weight = phi_size(&samples).unwrap();
        assert!((weight - 0.7).abs() < 0.07, "got {weight}");
    }

    #[test]
    fn test_phi_size_none_on_too_few_samples() {
        assert_eq!(phi_size(&[1.0, 2.0]), None);
    }

    #[test]
    fn test_phi_org_blend_matches_worked_example_a() {
        // 10-worked-examples.md A.1: time 0.88 / mech 0.90 / size 0.93, saturated w_time
        // (n_trades >= 200) -> blend 0.90.
        let phi = phi_org_blend(Some(0.88), 200, 0.90, 0.93);
        assert!((phi - 0.896).abs() < 1e-3, "got {phi}");
    }

    #[test]
    fn test_phi_org_blend_degrades_to_mech_size_when_time_unavailable() {
        // 10-worked-examples.md B.1: "time n/a — no CEX; mech 0.57 / size 0.66" -> 0.61.
        let phi = phi_org_blend(None, 0, 0.57, 0.66);
        assert!((phi - 0.606).abs() < 1e-3, "got {phi}");
    }

    #[test]
    fn test_phi_org_blend_degrades_at_n_zero_even_with_time_available() {
        // Backend A never classifies swap-level trades, so n_trades is always 0 there;
        // w_time must vanish regardless of what phi_time claims.
        let with_zero_n = phi_org_blend(Some(0.99), 0, 0.5, 0.5);
        let without_time = phi_org_blend(None, 0, 0.5, 0.5);
        assert!((with_zero_n - without_time).abs() < 1e-12);
    }

    #[test]
    fn test_shrink_phi_org_returns_prior_at_n_zero() {
        assert!((shrink_phi_org(0.9, 0, 0.05, 0.6, 0.01) - 0.6).abs() < 1e-12);
    }

    #[test]
    fn test_shrink_phi_org_pulls_toward_prior_with_few_observations() {
        let shrunk = shrink_phi_org(0.95, 12, 0.05, 0.6, 0.01);
        assert!(shrunk < 0.95 && shrunk > 0.6, "got {shrunk}");
    }

    /// Deterministic pseudo-uniform-derived exponential sample, so mixture tests don't
    /// need a `rand` dependency: a multiplicative hash feeds the inverse exponential CDF.
    fn pseudo_exponential(i: u64, mean: f64) -> f64 {
        let hashed = i.wrapping_mul(2_654_435_761).wrapping_add(1);
        let u = ((hashed % 1_000_000) as f64 + 1.0) / 1_000_001.0;
        -mean * (1.0 - u).ln()
    }
}
