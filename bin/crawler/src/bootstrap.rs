//! Ensures a pool's row exists before any event for it is written -- `swaps.pool_address` and
//! `liquidity_events.pool_address` both carry a foreign key into `pools`. Reads the pool's
//! current `LbPair` account through `source::rpc::StatePoller`, the same batched reader the
//! indexer's state worker uses, rather than hand-rolling a second `getMultipleAccounts` call.
//! This only ever captures the pool's state as of "now"; plain RPC has no way to read an
//! account's contents as of a past slot, which is exactly the gap the transaction-log walk
//! (see `convert.rs`) exists to fill honestly instead.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use eyre::WrapErr;
use rust_decimal::Decimal;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use sqlx::PgPool;

use dlmm_decode::{PoolState, PoolStatus};
use source::rpc::StatePoller;
use storage::write::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};

fn decimal_from_f64(x: f64) -> eyre::Result<Decimal> {
    Decimal::from_f64_retain(x).ok_or_else(|| eyre::eyre!("{x} is not representable as Decimal"))
}

/// Same fee-rate derivation the indexer uses for the same reason: delegating to `dlmm_math`
/// keeps this bit-exact with what the on-chain program itself would compute, rather than a
/// second reimplementation of the fee curve drifting from the first.
fn base_fee_bps(state: &PoolState) -> eyre::Result<Decimal> {
    let base = dlmm_math::base_fee_rate(
        state.bin_step,
        state.base_factor,
        state.base_fee_power_factor,
    )
    .map_err(|e| eyre::eyre!("Computing base fee rate: {e}"))?;
    decimal_from_f64(base * 10_000.0)
}

fn pool_rows(
    pool_address: &Pubkey,
    state: &PoolState,
) -> eyre::Result<(NewPool, NewDlmmPoolParams)> {
    let status = match state.status {
        PoolStatus::Enabled => 0,
        PoolStatus::Disabled => 1,
    };
    let now = Utc::now();

    let shared = NewPool {
        pool_address: pool_address.to_string(),
        venue: storage::types::venue::DLMM,
        token_x: state.token_x_mint.to_string(),
        token_y: state.token_y_mint.to_string(),
        base_fee_bps: base_fee_bps(state)?,
        protocol_share_bps: state.protocol_share_bps as i32,
        // Left unset rather than guessed: a crawler backfill has no flow-metrics source of
        // its own, and a stale/zero value here would be actively misleading to a screening
        // query. The live indexer's discovery worker fills this in once it picks the pool up.
        tvl_usd: None,
        status,
        creator: None,
        activation_point: None,
        created_at: now,
        first_liquidity_at: None,
        is_blacklisted: false,
        launchpad: None,
        tags: Vec::new(),
        updated_at: now,
    };

    let params = NewDlmmPoolParams {
        pool_address: pool_address.to_string(),
        bin_step: state.bin_step as i16,
        base_factor: state.base_factor as i32,
        filter_period: state.filter_period as i32,
        decay_period: state.decay_period as i32,
        reduction_factor: state.reduction_factor as i32,
        variable_fee_control: state.variable_fee_control as i32,
        max_volatility_accumulator: state.max_volatility_accumulator as i32,
        // DLMM's `collect_fee_mode` is not yet surfaced by the account decoder; 0 matches
        // the indexer's own placeholder for the same field.
        collect_fee_mode: 0,
        reward_mint_x: None,
        reward_mint_y: None,
    };

    Ok((shared, params))
}

/// Fetches the pool's current on-chain state and upserts its `pools` + `dlmm_pool_params`
/// rows. Safe to call every run: `upsert_dlmm_pool` is a plain upsert, so a pool already known
/// to the live indexer is only ever refreshed, never duplicated or reset.
pub async fn ensure_pool_row(
    db: &PgPool,
    rpc_client: &Arc<RpcClient>,
    max_concurrent: usize,
    max_retries: usize,
    pool: &Pubkey,
) -> eyre::Result<()> {
    let poller = StatePoller::new(rpc_client.clone(), max_concurrent, max_retries);
    let updates = poller
        .poll_all(std::slice::from_ref(pool), &HashMap::new())
        .await
        .wrap_err_with(|| format!("Reading current state for pool {pool}"))?;

    let Some(update) = updates.into_iter().find(|u| u.pool == *pool) else {
        eyre::bail!("Pool {pool} did not come back from getMultipleAccounts -- does it exist?");
    };
    let Some(state) = update.lb_pair else {
        eyre::bail!("Pool {pool} account did not decode as an LbPair");
    };

    let (shared, params) = pool_rows(pool, &state)?;
    upsert_dlmm_pool(db, &shared, &params)
        .await
        .wrap_err_with(|| format!("Upserting pool row for {pool}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> PoolState {
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
    fn test_pool_rows_maps_status_and_params() {
        let pool = Pubkey::new_unique();
        let state = sample_state();
        let (shared, params) = pool_rows(&pool, &state).unwrap();

        assert_eq!(shared.pool_address, pool.to_string());
        assert_eq!(shared.status, 0);
        assert_eq!(shared.venue, storage::types::venue::DLMM);
        assert_eq!(params.bin_step, 20);
        assert_eq!(params.collect_fee_mode, 0);
    }

    #[test]
    fn test_pool_rows_maps_disabled_status() {
        let pool = Pubkey::new_unique();
        let mut state = sample_state();
        state.status = PoolStatus::Disabled;
        let (shared, _) = pool_rows(&pool, &state).unwrap();
        assert_eq!(shared.status, 1);
    }
}
