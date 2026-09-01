use dlmm_math::{Comparator, RationaleItem};

/// Evaluate one gate condition and record it, whether or not it changes the outcome. This
/// is the one place a `RationaleItem`'s `passed` is derived from a real comparison; every
/// stage in this crate goes through it.
pub fn check(
    signal: impl Into<String>,
    observed: f64,
    cmp: Comparator,
    threshold: f64,
) -> RationaleItem {
    let passed = match cmp {
        Comparator::Ge => observed >= threshold,
        Comparator::Le => observed <= threshold,
        Comparator::Gt => observed > threshold,
        Comparator::Lt => observed < threshold,
    };
    RationaleItem {
        signal: signal.into(),
        observed,
        cmp,
        threshold,
        passed,
    }
}

/// Record a check that could not be evaluated: the input (e.g. holder concentration, an
/// organic score, a third-party risk flag) is not available to us at all, rather than a
/// normal pass or fail.
///
/// `observed = NaN` is the typed marker that this row was skipped, not evaluated and
/// found true; `passed = true` so a missing signal never blocks the gate on its own. A
/// renderer or a future backfill can distinguish this from a real pass by checking
/// `observed.is_nan()`.
pub fn unavailable(signal: impl Into<String>, cmp: Comparator, threshold: f64) -> RationaleItem {
    RationaleItem {
        signal: signal.into(),
        observed: f64::NAN,
        cmp,
        threshold,
        passed: true,
    }
}

/// Record an informational observation with no pass/fail meaning of its own (e.g. "was
/// `phi_time` available this tick"), so the pipeline's contract of one `RationaleItem`
/// per stage per relevant condition holds even off the gating path.
pub fn info(signal: impl Into<String>, observed: f64) -> RationaleItem {
    RationaleItem {
        signal: signal.into(),
        observed,
        cmp: Comparator::Ge,
        threshold: observed,
        passed: true,
    }
}
