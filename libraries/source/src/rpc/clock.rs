use eyre::WrapErr;
use solana_sdk::{account::Account, clock::Clock, pubkey, pubkey::Pubkey};

/// Clock sysvar address. Appended as the last key of every `getMultipleAccounts` batch so
/// each batch returns its own authoritative (slot, unix_timestamp) with no extra round
/// trip and no risk of skew between a state read and a separate `getSlot` call. The
/// dynamic-fee accumulator decays against this on-chain time, never wall clock.
pub const CLOCK_SYSVAR: Pubkey = pubkey!("SysvarC1ock11111111111111111111111111111111");

pub fn decode_clock(account: &Account) -> eyre::Result<Clock> {
    bincode::deserialize(&account.data).wrap_err_with(|| "Deserialising Clock sysvar")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_sysvar_matches_the_well_known_address() {
        assert_eq!(
            CLOCK_SYSVAR.to_string(),
            "SysvarC1ock11111111111111111111111111111111"
        );
    }
}
