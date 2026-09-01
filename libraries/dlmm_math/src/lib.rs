//! Pure quantitative maths for DLMM: bin pricing, fees, volatility estimators, LVR/IL,
//! the ranking metric and sizing. No I/O. Delegates to `lb_clmm` (public program source)
//! wherever it has the relevant integer math, so results are bit-exact with the program
//! by construction; formulas that are our own derivation are cited by `F` number against
//! `00-shared-parameters.md §0.3` and `plans/04-indicators.md`.

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
