//! Pure quantitative maths for DLMM: bin pricing, fees, volatility estimators, LVR and
//! impermanent loss, the ranking metric, and sizing. No I/O.
//!
//! Bin pricing and the base/variable fee formulas mirror the on-chain program's own integer
//! arithmetic (derived from the public IDL, MeteoraAg/dlmm-sdk, and pinned against the
//! vendored program source at the time of writing -- see price.rs and fees.rs), so those
//! results are bit-exact with it by construction. The rest — volatility estimation, the
//! organic-flow blend, sizing — is ours.

mod error;
pub use error::*;

mod price;
pub use price::*;

mod fees;
pub use fees::*;

mod volatility;
pub use volatility::*;

mod lvr;
pub use lvr::*;

mod range;
pub use range::*;

mod organic_flow;
pub use organic_flow::*;

mod ranking;
pub use ranking::*;

mod sizing;
pub use sizing::*;

#[cfg(test)]
mod worked_examples;
