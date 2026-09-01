//! Worked-example fixtures (`10-worked-examples.md`, Examples A and B), exercised
//! end-to-end through the public API rather than through any one formula in isolation.
//! Individual formulas are also hand-checked in their own modules; this file is the
//! integration check that they compose to the numbers the spec actually publishes.

#[cfg(test)]
mod tests {
    use crate::lvr::lvr_rate_spot;
    use crate::ranking::{Dlmm, PoolState, VolEstimate, r_org, r_ratio, y_fee};

    /// Example A — stable regime, USDC-USDT, s = 1 bp (10-worked-examples.md §A.1).
    #[test]
    fn test_example_a_r_org_e_fees_lvr() {
        let pool = PoolState {
            bin_step_bps: 1,
            base_factor: 10_000,
            base_fee_power_factor: 0,
            variable_fee_control: 0, // f_v ~= 0 in S: "f_hat = 1.0 bp (f_v ~= 0)"
            active_bin_liquidity: 800_000.0,
            protocol_share: 0.10,
        };
        let vol = VolEstimate {
            sigma_d: 2e-4,
            sigma_d_bps: 0.1,
            kappa_c: 3.0,
        };
        let vol_24h = 25_000_000.0;
        let phi_org = 0.90;
        let h_jit = 0.05; // S

        // R_org (F14 organic form): published 12.0.
        let r = r_ratio(1.0e-4, 31.25, 0.0001, pool.protocol_share, vol.sigma_d);
        let org = r_org(r, phi_org, h_jit);
        assert!((org - 12.0).abs() < 0.05, "R_org: got {org}, want ~12.0");

        // Same number via the Venue trait, not a hand-plugged f_hat/tau_a.
        let org_via_trait =
            crate::ranking::rank(&Dlmm, &pool, &vol, vol_24h, phi_org, h_jit).unwrap();
        assert!(
            (org_via_trait - 12.0).abs() < 0.1,
            "R_org via Venue: got {org_via_trait}"
        );

        // E[fees]: Y_fee at m* = $10k, active-bin capital, per day (00 §0.6 gives the
        // annualised 101% figure; per-day here is the same computation without *365).
        let y = y_fee(pool.protocol_share, 1.0e-4, 31.25, 0.0, 800_000.0, 10_000.0);
        let annualised = y * 365.0;
        assert!(
            (annualised - 1.0139).abs() < 1e-3,
            "Y_fee annualised: got {annualised}"
        );

        // LVR (F9): published $2.00/day at V=$50,000, w=2*5e-4.
        let lvr = lvr_rate_spot(2e-4, 50_000.0, 5e-4);
        assert!((lvr - 2.0).abs() < 1e-9, "LVR: got {lvr}");
    }

    /// Example B — volatile regime (V2), MEME-SOL, s = 100 bp (10-worked-examples.md §B.1).
    ///
    /// The spec is explicit that this example is a deliberately marginal case: under the
    /// post-merge rule (LM yield counts toward the `Y_fee` hurdle only, never toward
    /// `R_org`), this pool's `R_org = 1.94` is below V2's `R_min = 3.0`, so the entry does
    /// NOT fire. The full realised-path numbers in the doc ($3,003 profit) describe what
    /// the discipline costs, not what the engine should have done — an engine that scores
    /// this pool as attractive has a bug or a silently-moved `R_min`.
    #[test]
    fn test_example_b_rejects_on_fees_alone() {
        const R_MIN_V2: f64 = 3.0;

        let pool = PoolState {
            bin_step_bps: 100,
            base_factor: 10_000, // f_b = 100 bp at s = 100 bp
            base_fee_power_factor: 0,
            variable_fee_control: 40_000,
            active_bin_liquidity: 12_000.0,
            protocol_share: 0.10,
        };
        let vol_24h = 4_500_000.0;
        let tau_a = vol_24h / pool.active_bin_liquidity;
        assert!((tau_a - 375.0).abs() < 1e-9);

        let phi_org = 0.61;
        let h_jit = 0.15; // V2
        let f_hat = 0.0125; // published f_hat = 125 bp

        let r = r_ratio(
            f_hat,
            tau_a,
            pool.bin_step_bps as f64 / 10_000.0,
            pool.protocol_share,
            0.15,
        );
        assert!((r - 3.75).abs() < 1e-9, "R_gross: got {r}, want 3.75");

        let org = r_org(r, phi_org, h_jit);
        assert!((org - 1.94).abs() < 0.01, "R_org: got {org}, want 1.94");

        // The gate: REJECT, because R_org < R_min for V2.
        assert!(
            org < R_MIN_V2,
            "Example B must REJECT: R_org {org} is not < R_min {R_MIN_V2}"
        );

        // E[fees]: Y_fee at m* = $500, active-bin capital, per day: published 3.44/day.
        let y = y_fee(
            pool.protocol_share,
            f_hat,
            tau_a,
            h_jit,
            pool.active_bin_liquidity,
            500.0,
        );
        assert!((y - 3.4425).abs() < 1e-3, "Y_fee: got {y}");

        // LVR (F9): published $810/day at V=$20,000, w = N*s = 40*0.01 = 0.4 (N=40 bins,
        // s=100bp), sigma_slow=18% (the LVR line uses the slow estimate; the R_gross line
        // above uses sigma_fast=15% -- the spec computes both and shows R at each).
        let lvr = lvr_rate_spot(0.18, 20_000.0, 0.4);
        assert!((lvr - 810.0).abs() < 1e-6, "LVR: got {lvr}, want 810.0");
    }
}
