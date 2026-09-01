// The only crate in the workspace allowed to contain SQL. `bin/bot` and any future HTTP API call
// the functions in `write` and `queries` and never construct a query of their own; CI greps the
// rest of the tree for `sqlx::query` to enforce it.

mod migrate;
pub use migrate::*;

pub mod types;

pub mod write;

pub mod queries;

#[cfg(all(test, feature = "db-tests"))]
mod test_support;
