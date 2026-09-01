use crate::error::MathError;
use crate::fees;

/// Venue identifier (`pools.venue`, `0 = DLMM, 1 = DAMM_V2,...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueId {
    Dlmm,
    DammV2,
}

/// Minimal, venue-agnostic pool state needed to rank a pool. This is `dlmm_math`'s own
/// boundary type — the engine layer's richer pool state maps onto it at
/// the call site; the math crate stays free of I/O and storage types.
#[derive(Debug, Clone, Copy)]
pub struct PoolState {
    pub bin_step_bps: u16,
    pub base_factor: u16,
    pub base_fee_power_factor: u8,
    pub variable_fee_control: u32,
    pub active_bin_liquidity: f64,
    pub protocol_share: f64,
}

/// Volatility inputs a `Venue` needs to price and rank a pool.
#[derive(Debug, Clone, Copy)]
pub struct VolEstimate {
    /// Daily vol (fraction), variance-ratio corrected — the `σ_d` that sits in the ranking metric's
    /// denominator.
    pub sigma_d: f64,
    /// Decay-window vol in bps, feeding the forecast fee's `f_v̂`.
    pub sigma_d_bps: f64,
    /// Fee-clustering multiplier used by the fee forecast. Estimated, not tuned.
    pub kappa_c: f64,
}

/// Current and forecast fee rate (fractions, not bps — callers format for display).
#[derive(Debug, Clone, Copy)]
pub struct FeeRate {
    pub current: f64,
    pub forecast: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparator {
    Ge,
    Le,
    Gt,
    Lt,
}

/// One gate check, recorded whether or not it changes the outcome ("Every
/// stage writes a `RationaleItem`... whether or not it changes the outcome").
#[derive(Debug, Clone)]
pub struct RationaleItem {
    pub signal: String,
    pub observed: f64,
    pub cmp: Comparator,
    pub threshold: f64,
    pub passed: bool,
}

/// How a pool's economics are read — the narrow seam a second venue
/// extends through. `fee_rate`, `turnover_base` and `lvr_geometry` are the only
/// venue-specific inputs to ranking; everything downstream of them is shared.
pub trait Venue: Send + Sync {
    fn id(&self) -> VenueId;

    /// Fee rate now and forecast. DLMM: the base fee + the variable fee/the forecast fee.
    fn fee_rate(&self, pool: &PoolState, vol: &VolEstimate) -> Result<FeeRate, MathError>;

    /// Turnover denominator. DLMM: `L_a` (active-bin liquidity).
    fn turnover_base(&self, pool: &PoolState) -> Option<f64>;

    /// The geometry factor in the LVR denominator. DLMM Spot: `s` (bin step, as a
    /// fraction). Keeps the ranking metric and the ranged-AMM ranking metric one expression.
    fn lvr_geometry(&self, pool: &PoolState) -> f64;

    /// Venue-specific gates beyond the shared risk gate. DLMM rejects nothing here.
    fn extra_gates(&self, pool: &PoolState) -> Vec<RationaleItem>;
}

/// DLMM: fees via the base fee/the variable fee/the forecast fee, turnover base is active-bin liquidity, geometry is the bin
/// step itself.
pub struct Dlmm;

impl Venue for Dlmm {
    fn id(&self) -> VenueId {
        VenueId::Dlmm
    }

    fn fee_rate(&self, pool: &PoolState, vol: &VolEstimate) -> Result<FeeRate, MathError> {
        let current = fees::base_fee_rate(
            pool.bin_step_bps,
            pool.base_factor,
            pool.base_fee_power_factor,
        )?;
        let forecast = fees::endogenous_fee_rate(
            pool.bin_step_bps,
            pool.base_factor,
            pool.base_fee_power_factor,
            pool.variable_fee_control,
            vol.sigma_d_bps,
            vol.kappa_c,
        )?;
        Ok(FeeRate { current, forecast })
    }

    fn turnover_base(&self, pool: &PoolState) -> Option<f64> {
        if pool.active_bin_liquidity > 0.0 {
            Some(pool.active_bin_liquidity)
        } else {
            None
        }
    }

    fn lvr_geometry(&self, pool: &PoolState) -> f64 {
        pool.bin_step_bps as f64 / 10_000.0
    }

    fn extra_gates(&self, _pool: &PoolState) -> Vec<RationaleItem> {
        // DLMM rejects nothing here -- the shared risk gate covers it.
        Vec::new()
    }
}

/// the ranking metric (DLMM) / the ranged-AMM ranking metric (DAMM v2): fee/LVR ratio at the active bin.
///
/// Written once against `geometry` so the ranking metric (`geometry = s`) and the ranged-AMM ranking metric
/// (`geometry = g/2`) are literally the same expression — the
/// algebra that shows DAMM v2's `σ²V/(4g)` reduces to DLMM's `σ²V/(2w)` at narrow
/// ranges.
///
/// # Formula
///
/// * `R = 2 · f̂ · τ_a · geometry · (1 − ps) / σ_d²`, breakeven `R = 1`
pub fn r_ratio(f_hat: f64, tau_a: f64, geometry: f64, protocol_share: f64, sigma_d: f64) -> f64 {
    2.0 * f_hat * tau_a * geometry * (1.0 - protocol_share) / (sigma_d * sigma_d)
}

/// Organic form of the ranking metric/the ranged-AMM ranking metric: `R_org = R · φ_org · (1 − h_JIT)`.
pub fn r_org(r: f64, phi_org: f64, h_jit: f64) -> f64 {
    r * phi_org * (1.0 - h_jit)
}

/// Expected fee yield rate (daily fraction) at intended position size `m`:
///'s yield-at-size. Self-dilution is folded in via `L̄_a/(L̄_a + m)`, so ranking a
/// pool we would swamp is impossible by construction.
///
/// # Formula
///
/// * `Y_fee = (1 − ps) · f̂ · τ_a · (1 − h_JIT) · L̄_a/(L̄_a + m)`
pub fn y_fee(
    protocol_share: f64,
    f_hat: f64,
    tau_a: f64,
    h_jit: f64,
    active_bin_liquidity: f64,
    m: f64,
) -> f64 {
    (1.0 - protocol_share) * f_hat * tau_a * (1.0 - h_jit) * active_bin_liquidity
        / (active_bin_liquidity + m)
}

/// End-to-end ranking through a `Venue`: the one place the ranking metric/the ranged-AMM ranking metric are computed
///.
pub fn rank<V: Venue>(
    venue: &V,
    pool: &PoolState,
    vol: &VolEstimate,
    vol_24h: f64,
    phi_org: f64,
    h_jit: f64,
) -> Result<f64, MathError> {
    let fee = venue.fee_rate(pool, vol)?;
    let l_a = venue.turnover_base(pool).unwrap_or(0.0);
    let tau_a = if l_a > 0.0 { vol_24h / l_a } else { 0.0 };
    let geometry = venue.lvr_geometry(pool);
    let r = r_ratio(
        fee.forecast,
        tau_a,
        geometry,
        pool.protocol_share,
        vol.sigma_d,
    );
    Ok(r_org(r, phi_org, h_jit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_r_ratio_matches_worked_example_a() {
        // worked example A: R_gross = 14.1.
        let r = r_ratio(1e-4, 31.25, 1e-4, 0.10, 2e-4);
        assert!((r - 14.0625).abs() < 1e-3, "got {r}");
    }

    #[test]
    fn test_r_org_matches_worked_example_a() {
        let r = r_ratio(1e-4, 31.25, 1e-4, 0.10, 2e-4);
        let org = r_org(r, 0.90, 0.05); // h_JIT = 0.05 (S)
        assert!((org - 12.0).abs() < 0.05, "got {org}");
    }

    #[test]
    fn test_r_ratio_matches_worked_example_b() {
        // worked example B: R_gross (sigma_fast) = 3.75.
        let r = r_ratio(0.0125, 375.0, 0.01, 0.10, 0.15);
        assert!((r - 3.75).abs() < 1e-9, "got {r}");
    }

    /// Example B is the deliberately marginal case: on swap fees alone its organic ratio
    /// is 1.94 against a floor of 3.0, so it must be rejected. Liquidity-mining yield
    /// counts toward the yield hurdle but never toward the ratio, which is what keeps this
    /// a rejection. An engine that accepts this pool has a bug or a silently moved
    /// threshold — the single most important assertion in the crate.
    #[test]
    fn test_worked_example_b_rejects() {
        const R_MIN_V2: f64 = 3.0;

        let r = r_ratio(0.0125, 375.0, 0.01, 0.10, 0.15); // h_JIT = 0.15 (V2)
        let org = r_org(r, 0.61, 0.15);

        assert!((org - 1.94).abs() < 0.01, "got {org}, expected ~1.94");
        assert!(
            org < R_MIN_V2,
            "Example B must REJECT: R_org {org} should be < R_min {R_MIN_V2}"
        );
    }

    #[test]
    fn test_y_fee_matches_worked_example_b() {
        // worked example B: Y_fee at m* = $500 (active-bin capital, per day) = 3.44/day.
        let y = y_fee(0.10, 0.0125, 375.0, 0.15, 12_000.0, 500.0);
        assert!((y - 3.4425).abs() < 1e-3, "got {y}");
    }

    #[test]
    fn test_rank_via_venue_trait_matches_worked_example_a() {
        let pool = PoolState {
            bin_step_bps: 1,
            base_factor: 10_000,
            base_fee_power_factor: 0,
            variable_fee_control: 0,
            active_bin_liquidity: 800_000.0,
            protocol_share: 0.10,
        };
        // sigma_d_bps chosen tiny enough that f_v ~= 0, matching A's "f_hat = 1.0 bp (f_v ~= 0)".
        let vol = VolEstimate {
            sigma_d: 2e-4,
            sigma_d_bps: 0.1,
            kappa_c: 3.0,
        };

        let org = rank(&Dlmm, &pool, &vol, 25_000_000.0, 0.90, 0.05).unwrap();
        assert!((org - 12.0).abs() < 0.1, "got {org}");
    }
}
