use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;

/// Compute unit limit and priority fee for a transaction, left to the caller rather than
/// hard-coded: the right limit depends on how many bin arrays an operation touches, and the
/// right price on network conditions at send time, neither of which this crate can know.
#[derive(Clone, Copy, Debug, Default)]
pub struct ComputeBudgetConfig {
    pub unit_limit: Option<u32>,
    pub unit_price_micro_lamports: Option<u64>,
}

impl ComputeBudgetConfig {
    pub fn none() -> Self {
        Self::default()
    }

    /// The two ComputeBudget instructions, if configured, in the order they must appear (limit
    /// before price makes no functional difference to the runtime, but keeps the instruction
    /// list stable for callers that log or hash it).
    pub fn instructions(&self) -> Vec<Instruction> {
        let mut ixs = Vec::with_capacity(2);
        if let Some(limit) = self.unit_limit {
            ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(limit));
        }
        if let Some(price) = self.unit_price_micro_lamports {
            ixs.push(ComputeBudgetInstruction::set_compute_unit_price(price));
        }
        ixs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_none_config_produces_no_instructions() {
        assert!(ComputeBudgetConfig::none().instructions().is_empty());
    }

    #[test]
    fn test_configured_budget_produces_both_instructions_in_order() {
        let config = ComputeBudgetConfig {
            unit_limit: Some(200_000),
            unit_price_micro_lamports: Some(5_000),
        };
        let ixs = config.instructions();
        assert_eq!(ixs.len(), 2);
        // ComputeBudget instruction discriminants: 2 = SetComputeUnitLimit, 3 = SetComputeUnitPrice.
        assert_eq!(ixs[0].data[0], 2);
        assert_eq!(ixs[1].data[0], 3);
    }
}
