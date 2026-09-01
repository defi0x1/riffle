use dlmm_math::{Comparator, FeeRate, MathError, PoolState, RationaleItem, Venue, VolEstimate};

use crate::rationale;

/// The forecast fee rate can never exceed this, enforced inside `dlmm_math`'s fee
/// pipeline already -- recorded here so the fee-forecast stage still contributes a
/// `RationaleItem` even when nothing is wrong.
pub const FEE_RATE_CAP: f64 = 0.10;

/// Fee-forecast stage: current and forecast fee rate through the `Venue` trait, so a
/// second venue costs nothing here beyond its own `Venue` implementation.
pub fn evaluate<V: Venue>(
    venue: &V,
    pool: &PoolState,
    vol: &VolEstimate,
) -> Result<(FeeRate, Vec<RationaleItem>), MathError> {
    let fee = venue.fee_rate(pool, vol)?;
    let rationale = vec![
        rationale::info("fee_current", fee.current),
        rationale::check(
            "fee_forecast_within_cap",
            fee.forecast,
            Comparator::Le,
            FEE_RATE_CAP,
        ),
    ];
    Ok((fee, rationale))
}
