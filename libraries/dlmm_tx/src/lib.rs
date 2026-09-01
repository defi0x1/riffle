//! Builds unsigned Solana instructions for Meteora DLMM liquidity operations, transcribed from
//! the public IDL (https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json).
//!
//! This crate never handles a private key, seed phrase, or signature, and no type in its public
//! API can carry one -- every builder here takes plain `Pubkey`s and returns `Instruction`s,
//! neither of which is capable of signing anything. Transactions get assembled and signed on the
//! user's device in the Telegram Mini App; this crate only ever sees, and only ever needs, public
//! keys.

mod error;
pub use error::*;

mod pda;
pub use pda::*;

mod args;
pub use args::*;

mod compute_budget;
pub use compute_budget::*;

mod instructions;
pub use instructions::*;

#[cfg(test)]
mod test_support;
