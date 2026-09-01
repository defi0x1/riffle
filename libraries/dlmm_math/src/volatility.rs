/// EWMA half-life ~2 h (`sigma_fast`).
pub const LAMBDA_FAST: f64 = 0.97;
/// EWMA half-life ~1 day (`sigma_slow`).
pub const LAMBDA_SLOW: f64 = 0.997;

/// Garman-Klass variance for a single OHLC bar.
///
/// # Formula
///
/// * `σ²_GK = ½(ln H/L)² − (2ln2 − 1)(ln C/O)²`
pub fn garman_klass_variance(open: f64, high: f64, low: f64, close: f64) -> f64 {
    let hl = (high / low).ln();
    let co = (close / open).ln();
    0.5 * hl * hl - (2.0 * std::f64::consts::LN_2 - 1.0) * co * co
}

/// One EWMA variance update: `v_t = λ·v_{t-1} + (1−λ)·x_t`.
pub fn ewma_update(prev_variance: f64, new_observation_variance: f64, lambda: f64) -> f64 {
    lambda * prev_variance + (1.0 - lambda) * new_observation_variance
}

/// Daily variance, variance-ratio corrected for bin-quantised autocorrelation, floored
/// at half the naive value (`00(ii)`,).
///
/// `autocorrelations` are `ρ_1..ρ_6` of the 5-minute return series; only the first six
/// lags are used.
///
/// # Formula
///
/// * `σ_d² = σ_5m² · 288 · (1 + 2·Σ_{k≤6} ρ_k)`, floored at `0.5 · σ_5m² · 288`
pub fn variance_ratio_corrected_daily_variance(sigma_5m_sq: f64, autocorrelations: &[f64]) -> f64 {
    let naive = sigma_5m_sq * 288.0;
    let sum_rho: f64 = autocorrelations.iter().take(6).sum();
    let corrected = naive * (1.0 + 2.0 * sum_rho);
    corrected.max(0.5 * naive)
}

/// `σ_d` from 5-minute bar variance, variance-ratio corrected.
pub fn daily_vol(sigma_5m_sq: f64, autocorrelations: &[f64]) -> f64 {
    variance_ratio_corrected_daily_variance(sigma_5m_sq, autocorrelations).sqrt()
}

/// `σ_D`, the decay-window vol that feeds the forecast fee, in bps: `σ_fast · √(decay_window/1 day)`.
pub fn decay_window_vol_bps(sigma_fast: f64, decay_window_secs: f64) -> f64 {
    sigma_fast * (decay_window_secs / 86_400.0).sqrt() * 10_000.0
}

fn bipower_variation(log_returns: &[f64]) -> f64 {
    if log_returns.len() < 2 {
        return 0.0;
    }
    let sum: f64 = log_returns
        .windows(2)
        .map(|w| w[0].abs() * w[1].abs())
        .sum();
    (std::f64::consts::FRAC_PI_2) * sum
}

fn realized_variance(log_returns: &[f64]) -> f64 {
    log_returns.iter().map(|r| r * r).sum()
}

/// Jump share of trailing-24h realized variance, via bipower variation, floored at 0.05
/// (`sigma_jump`).
///
/// # Formula
///
/// * `σ_jump = max((RV − BV) / RV, 0.05)`, `RV = Σ r_i²`, `BV = (π/2)·Σ|r_i||r_{i−1}|`
pub fn jump_share(log_returns: &[f64]) -> f64 {
    let rv = realized_variance(log_returns);
    if rv <= 0.0 {
        return 0.05;
    }
    let bv = bipower_variation(log_returns);
    ((rv - bv) / rv).max(0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_garman_klass_variance_hand_checked() {
        // O=100 H=110 L=95 C=105 -> sigma^2 ~= 0.0098267 (python cross-check)
        let sigma_sq = garman_klass_variance(100.0, 110.0, 95.0, 105.0);
        assert!(
            (sigma_sq - 0.00982672327557351).abs() < 1e-12,
            "got {sigma_sq}"
        );
    }

    #[test]
    fn test_garman_klass_variance_flat_bar_is_zero() {
        assert!((garman_klass_variance(1.0, 1.0, 1.0, 1.0) - 0.0).abs() < 1e-15);
    }

    #[test]
    fn test_ewma_update_hand_checked() {
        // v = 0.97*4 + 0.03*1 = 3.88 + 0.03 = 3.91
        assert!((ewma_update(4.0, 1.0, 0.97) - 3.91).abs() < 1e-12);
    }

    #[test]
    fn test_variance_ratio_correction_floor_binds_on_strongly_negative_autocorrelation() {
        // sum_rho = -1 -> (1 + 2*-1) = -1 -> corrected would be negative; floored at 0.5x naive.
        let sigma_5m_sq = 1e-6;
        let naive = sigma_5m_sq * 288.0;
        let got =
            variance_ratio_corrected_daily_variance(sigma_5m_sq, &[-1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((got - 0.5 * naive).abs() < 1e-15);
    }

    #[test]
    fn test_variance_ratio_correction_positive_autocorrelation_inflates() {
        let sigma_5m_sq = 1e-6;
        let naive = sigma_5m_sq * 288.0;
        // sum_rho = 0.5 over 6 lags -> factor (1 + 1.0) = 2.0
        let got =
            variance_ratio_corrected_daily_variance(sigma_5m_sq, &[0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((got - 2.0 * naive).abs() < 1e-15);
    }

    #[test]
    fn test_decay_window_vol_bps_hand_checked() {
        // sigma_fast = 18%/day, decay window 600s -> 0.18 * sqrt(600/86400) * 1e4 = 150 bp
        // (matches the sigma_D input used in worked example B, worked example B,
        // where the same value is supplied directly rather than re-derived).
        let bps = decay_window_vol_bps(0.18, 600.0);
        assert!((bps - 150.0).abs() < 1e-6, "got {bps}");
    }

    #[test]
    fn test_jump_share_no_jumps_floors_at_005() {
        // Constant-magnitude alternating-sign returns: RV ~= BV, share floors at 0.05.
        let returns = vec![0.01, -0.01, 0.01, -0.01, 0.01, -0.01, 0.01, -0.01];
        let share = jump_share(&returns);
        assert!((share - 0.05).abs() < 1e-9, "got {share}");
    }

    #[test]
    fn test_jump_share_one_big_move_among_small_ones_is_high() {
        let mut returns = vec![0.001; 50];
        returns.push(0.20);
        let share = jump_share(&returns);
        assert!(share > 0.5, "got {share}");
    }
}
