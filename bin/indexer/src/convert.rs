use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;

use dlmm_decode::{PoolState, PoolStatus};
use storage::write::{NewDlmmPoolParams, NewPool};

use crate::config::DEFAULT_COLLECT_FEE_MODE;

pub fn decimal_from_f64(x: f64) -> eyre::Result<Decimal> {
    Decimal::from_f64_retain(x).ok_or_else(|| eyre::eyre!("{x} is not representable as Decimal"))
}

pub fn decimal_from_u64(x: u64) -> Decimal {
    Decimal::from(x)
}

pub fn decimal_from_u128(x: u128) -> eyre::Result<Decimal> {
    Ok(Decimal::from(x))
}

pub fn unix_to_datetime(unix_secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(unix_secs, 0).unwrap_or_else(Utc::now)
}

/// Base and dynamic (variable) fee rate, in basis points, for the pool's current on-chain
/// parameters. Delegates to `lb_clmm`'s own fee pipeline via `dlmm_math`, so this is
/// bit-exact with what the program itself would compute.
pub fn fee_bps(state: &PoolState) -> eyre::Result<(Decimal, Decimal)> {
    let base = dlmm_math::base_fee_rate(
        state.bin_step,
        state.base_factor,
        state.base_fee_power_factor,
    )
    .map_err(|e| eyre::eyre!("Computing base fee rate: {e}"))?;
    let dynamic = dlmm_math::variable_fee_rate(
        state.bin_step,
        state.variable_fee_control,
        state.volatility_accumulator,
    )
    .map_err(|e| eyre::eyre!("Computing variable fee rate: {e}"))?;

    Ok((
        decimal_from_f64(base * 10_000.0)?,
        decimal_from_f64(dynamic * 10_000.0)?,
    ))
}

pub struct PoolMetadata {
    pub tvl_usd: Option<Decimal>,
    pub is_blacklisted: bool,
    pub launchpad: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Builds the shared `pools` row and the DLMM satellite row from a decoded `LbPair`. Every
/// field either comes straight off the account or is derived by `dlmm_math` from fields that
/// do -- nothing here is estimated.
pub fn pool_rows(
    pool_address: &Pubkey,
    state: &PoolState,
    meta: &PoolMetadata,
) -> eyre::Result<(NewPool, NewDlmmPoolParams)> {
    let (base_fee_bps, dynamic_fee_bps) = fee_bps(state)?;
    let status = match state.status {
        PoolStatus::Enabled => 0,
        PoolStatus::Disabled => 1,
    };

    let shared = NewPool {
        pool_address: pool_address.to_string(),
        venue: storage::types::venue::DLMM,
        token_x: state.token_x_mint.to_string(),
        token_y: state.token_y_mint.to_string(),
        // dynamic_fee_bps is left out here: pools.base_fee_bps is the pool's standing rate,
        // the variable component moves with volatility and is tracked per-snapshot instead.
        base_fee_bps,
        protocol_share_bps: state.protocol_share_bps as i32,
        tvl_usd: meta.tvl_usd,
        status,
        creator: None,
        activation_point: None,
        created_at: meta.created_at,
        first_liquidity_at: None,
        is_blacklisted: meta.is_blacklisted,
        launchpad: meta.launchpad.clone(),
        tags: Vec::new(),
        updated_at: meta.updated_at,
    };

    let _ = dynamic_fee_bps; // computed for completeness; consumed by dlmm_pool_state, not here

    let params = NewDlmmPoolParams {
        pool_address: pool_address.to_string(),
        bin_step: state.bin_step as i16,
        base_factor: state.base_factor as i32,
        filter_period: state.filter_period as i32,
        decay_period: state.decay_period as i32,
        reduction_factor: state.reduction_factor as i32,
        variable_fee_control: state.variable_fee_control as i32,
        max_volatility_accumulator: state.max_volatility_accumulator as i32,
        collect_fee_mode: DEFAULT_COLLECT_FEE_MODE,
        reward_mint_x: None,
        reward_mint_y: None,
    };

    Ok((shared, params))
}

/// A single changed field between two reads of the same pool's static parameters, with the
/// storage-schema field name and its old/new value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamChange {
    pub field: &'static str,
    pub old_value: i64,
    pub new_value: i64,
}

/// Detects a fee-parameter change between two account reads of the same pool. There is no
/// discrete on-chain event for this yet (see the event worker), so this is the only place
/// such a change is ever observed.
pub fn diff_fee_params(old: &PoolState, new: &PoolState) -> Vec<ParamChange> {
    let mut changes = Vec::new();

    macro_rules! check {
        ($field:literal, $accessor:ident) => {
            let (o, n) = (old.$accessor as i64, new.$accessor as i64);
            if o != n {
                changes.push(ParamChange {
                    field: $field,
                    old_value: o,
                    new_value: n,
                });
            }
        };
    }

    check!("base_factor", base_factor);
    check!("base_fee_power_factor", base_fee_power_factor);
    check!("filter_period", filter_period);
    check!("decay_period", decay_period);
    check!("reduction_factor", reduction_factor);
    check!("variable_fee_control", variable_fee_control);
    check!("max_volatility_accumulator", max_volatility_accumulator);
    check!("protocol_share_bps", protocol_share_bps);

    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_state() -> PoolState {
        PoolState {
            token_x_mint: Pubkey::new_unique(),
            token_y_mint: Pubkey::new_unique(),
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_step: 20,
            active_bin_id: 100,
            status: PoolStatus::Enabled,
            base_factor: 10_000,
            base_fee_power_factor: 0,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5_000,
            variable_fee_control: 40_000,
            max_volatility_accumulator: 350_000,
            protocol_share_bps: 500,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 100,
            protocol_fee_x: 0,
            protocol_fee_y: 0,
            last_updated_at: 0,
        }
    }

    #[test]
    fn test_diff_fee_params_reports_only_changed_fields() {
        let old = base_state();
        let mut new = base_state();
        new.reduction_factor = 6_000;

        let changes = diff_fee_params(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].field, "reduction_factor");
        assert_eq!(changes[0].old_value, 5_000);
        assert_eq!(changes[0].new_value, 6_000);
    }

    #[test]
    fn test_diff_fee_params_ignores_volatility_state() {
        let old = base_state();
        let mut new = base_state();
        new.volatility_accumulator = 12_345;
        new.index_reference = 999;

        assert!(diff_fee_params(&old, &new).is_empty());
    }

    #[test]
    fn test_diff_fee_params_empty_when_identical() {
        let state = base_state();
        assert!(diff_fee_params(&state, &state).is_empty());
    }
}
