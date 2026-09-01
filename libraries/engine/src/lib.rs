//! The decision pipeline: volatility, regime, risk gate, organic flow, fee forecast,
//! ranking, sizing, triggers. Orchestrates `dlmm_math`'s formulas against pool state and
//! produces the `Indicators` row persisted per pool per timeframe, plus a typed rationale
//! trail covering every stage whether or not it changed the outcome.

mod indicators;
pub use indicators::*;

mod rationale;

// Each stage below exposes its own `evaluate`, so these stay namespaced modules rather
// than glob re-exports -- callers reach them as `engine::regime::classify_candidate`,
// `engine::triggers::evaluate`, and so on. `pipeline` composes all of them behind
// `screen`/`rank`, which is the entry point most callers want.
pub mod fee_forecast;
pub mod organic_flow;
pub mod ranking;
pub mod regime;
pub mod risk_gate;
pub mod sizing;
pub mod triggers;
pub mod volatility;

mod config;
pub use config::*;

mod pipeline;
pub use pipeline::*;
