//! One-shot RPC backfill for pools that were not yet being watched when streaming ingestion
//! started. An operator tool, run by hand against a pool set and a slot or time range -- not
//! a long-lived worker, and not wired into `bin/indexer`'s continuous ingestion.

pub mod bootstrap;
pub mod checkpoint;
pub mod cli;
pub mod convert;
pub mod crawl;
pub mod pacing;
pub mod range;
pub mod rpc;

pub use cli::Args;
pub use crawl::run;
