use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use eyre::WrapErr;
use futures::StreamExt;
use solana_sdk::pubkey::Pubkey;
use sqlx::PgPool;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use common::Worker;
use dlmm_decode::PoolState;
use source::{Source, WatchSet};
use storage::write::{
    NewActiveBinSnapshot, NewBinState, NewDlmmPoolState, NewFeeParamUpdate, NewPoolSnapshot,
    insert_active_bin_snapshots, insert_bin_states, insert_fee_param_updates, insert_pool_state,
    upsert_dlmm_pool_params,
};

use crate::convert::{decimal_from_u64, decimal_from_u128, diff_fee_params, fee_bps};
use crate::workers::progress::Progress;
use crate::workers::state_buffer::StateBuffer;

/// Drives the tier-1 (watched) state stream, coalesces updates by pool, and writes pool
/// snapshots, active-bin snapshots and the full bin distribution in batches. The watch set is
/// re-read from storage on every refresh cycle so a promotion or demotion decided elsewhere
/// takes effect without a restart.
pub struct StateWorker {
    pool: PgPool,
    source: Arc<dyn Source>,
    progress: Arc<Progress>,
    watch_refresh_interval: Duration,
    flush_interval: Duration,
    flush_batch_size: usize,
    // Last static parameters seen per pool, for fee-parameter-change detection. There is no
    // discrete on-chain event for this yet (see the event worker), so a diff between
    // consecutive reads is the only way to observe it. Reset on restart -- a missed change
    // spanning a restart is not reported, which is an acceptable gap for an operational
    // signal, not a correctness-critical one.
    last_params: tokio::sync::Mutex<HashMap<Pubkey, PoolState>>,
}

struct UpdateRows {
    snapshot: NewPoolSnapshot,
    dlmm_state: NewDlmmPoolState,
    active_bin: Option<NewActiveBinSnapshot>,
    bin_states: Vec<NewBinState>,
    fee_updates: Vec<NewFeeParamUpdate>,
    param_refresh: Option<storage::write::NewDlmmPoolParams>,
}

impl StateWorker {
    pub fn new(
        pool: PgPool,
        source: Arc<dyn Source>,
        progress: Arc<Progress>,
        watch_refresh_interval: Duration,
        flush_interval: Duration,
        flush_batch_size: usize,
    ) -> Self {
        Self {
            pool,
            source,
            progress,
            watch_refresh_interval,
            flush_interval,
            flush_batch_size,
            last_params: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    // Builds every row derived from one pool's state update. Kept separate from `flush` and
    // fallible as a whole so one pool's bad math (an overflow, an out-of-range Decimal) is a
    // `continue` for that pool rather than a reason to abort the entire batch -- see the
    // per-item isolation in the caller.
    fn build_rows(
        update: &source::StateUpdate,
        previous: Option<&PoolState>,
    ) -> eyre::Result<Option<UpdateRows>> {
        let Some(state) = &update.lb_pair else {
            return Ok(None);
        };
        let pool_address = update.pool.to_string();
        let ts = Utc::now();

        let (base_fee_bps, dynamic_fee_bps) = fee_bps(state)?;
        let price = dlmm_math::bin_price(state.active_bin_id, state.bin_step)
            .map_err(|e| eyre::eyre!("Computing active bin price: {e}"))?;

        let snapshot = NewPoolSnapshot {
            pool_address: pool_address.clone(),
            ts,
            slot: update.slot as i64,
            price,
            // The account group fetched here is LbPair + BinArrays, not the reserve token
            // accounts, so raw reserves and any USD figure derived from them are not
            // available at this layer.
            reserve_x_raw: None,
            reserve_y_raw: None,
            tvl_usd: None,
            active_tvl_usd: None,
            total_fee_bps: base_fee_bps + dynamic_fee_bps,
        };

        let dlmm_state = NewDlmmPoolState {
            pool_address: pool_address.clone(),
            ts,
            active_bin_id: state.active_bin_id,
            volatility_accumulator: state.volatility_accumulator as i32,
            volatility_reference: state.volatility_reference as i32,
            index_reference: state.index_reference,
            last_update_timestamp: state.last_updated_at,
            base_fee_bps,
            dynamic_fee_bps,
        };

        let active_bin = update
            .bin_arrays
            .iter()
            .flat_map(|ba| ba.bins.iter())
            .find(|b| b.bin_id == state.active_bin_id)
            .map(|bin| {
                eyre::Result::<_>::Ok(NewActiveBinSnapshot {
                    pool_address: pool_address.clone(),
                    ts,
                    slot: update.slot as i64,
                    bin_id: bin.bin_id,
                    amount_x: decimal_from_u64(bin.amount_x),
                    amount_y: decimal_from_u64(bin.amount_y),
                    liquidity_supply: decimal_from_u128(bin.liquidity_supply)?,
                    quote_value_usd: None,
                })
            })
            .transpose()?;
        if active_bin.is_none() {
            tracing::debug!(
                pool = %update.pool,
                active_bin_id = state.active_bin_id,
                "Active bin not present in the fetched bin arrays this tick"
            );
        }

        let mut bin_states = Vec::new();
        for bin in update.bin_arrays.iter().flat_map(|ba| ba.bins.iter()) {
            let ui_price = dlmm_math::bin_price(bin.bin_id, state.bin_step)
                .map_err(|e| eyre::eyre!("Computing bin price: {e}"))?;
            bin_states.push(NewBinState {
                pool_address: pool_address.clone(),
                ts,
                slot: update.slot as i64,
                bin_id: bin.bin_id,
                amount_x: decimal_from_u64(bin.amount_x),
                amount_y: decimal_from_u64(bin.amount_y),
                liquidity_supply: decimal_from_u128(bin.liquidity_supply)?,
                price_q64: decimal_from_u128(bin.price)?,
                ui_price,
                fee_x_per_token_stored: decimal_from_u128(bin.fee_amount_x_per_token_stored)?,
                fee_y_per_token_stored: decimal_from_u128(bin.fee_amount_y_per_token_stored)?,
            });
        }

        let mut fee_updates = Vec::new();
        let mut param_refresh = None;
        if let Some(previous) = previous {
            let changes = diff_fee_params(previous, state);
            if !changes.is_empty() {
                for change in &changes {
                    fee_updates.push(NewFeeParamUpdate {
                        pool_address: pool_address.clone(),
                        ts,
                        slot: update.slot as i64,
                        // No transaction signature is available from an account read; the
                        // pool/slot/field triple is the natural key here.
                        signature: format!("state-diff:{}:{}", update.slot, change.field),
                        field: change.field.to_string(),
                        old_value: Some(change.old_value),
                        new_value: Some(change.new_value),
                    });
                }
                param_refresh = Some(storage::write::NewDlmmPoolParams {
                    pool_address: pool_address.clone(),
                    bin_step: state.bin_step as i16,
                    base_factor: state.base_factor as i32,
                    filter_period: state.filter_period as i32,
                    decay_period: state.decay_period as i32,
                    reduction_factor: state.reduction_factor as i32,
                    variable_fee_control: state.variable_fee_control as i32,
                    max_volatility_accumulator: state.max_volatility_accumulator as i32,
                    collect_fee_mode: crate::config::DEFAULT_COLLECT_FEE_MODE,
                    reward_mint_x: None,
                    reward_mint_y: None,
                });
            }
        }

        Ok(Some(UpdateRows {
            snapshot,
            dlmm_state,
            active_bin,
            bin_states,
            fee_updates,
            param_refresh,
        }))
    }

    async fn flush(&self, buffer: &mut StateBuffer) -> eyre::Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }
        let updates = buffer.drain();

        let mut snapshots = Vec::new();
        let mut dlmm_states = Vec::new();
        let mut active_bins = Vec::new();
        let mut bin_states = Vec::new();
        let mut fee_updates = Vec::new();
        let mut param_refreshes: Vec<storage::write::NewDlmmPoolParams> = Vec::new();

        let mut max_slot = 0u64;
        let mut max_block_time = 0i64;

        {
            let mut last_params = self.last_params.lock().await;

            for update in &updates {
                max_slot = max_slot.max(update.slot);
                max_block_time = max_block_time.max(update.block_time);

                let previous = last_params.get(&update.pool).cloned();
                match Self::build_rows(update, previous.as_ref()) {
                    Ok(Some(rows)) => {
                        snapshots.push(rows.snapshot);
                        dlmm_states.push(rows.dlmm_state);
                        active_bins.extend(rows.active_bin);
                        bin_states.extend(rows.bin_states);
                        fee_updates.extend(rows.fee_updates);
                        param_refreshes.extend(rows.param_refresh);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(error = ?e, pool = %update.pool, "Failed to build state rows for pool, skipping");
                        metrics::DECODE_ERROR_TOTAL
                            .with_label_values(&["lb_pair"])
                            .inc();
                        self.progress.record_decode_error();
                    }
                }

                if let Some(state) = &update.lb_pair {
                    last_params.insert(update.pool, state.clone());
                }
            }
        }

        let mut rows_written = 0u64;
        insert_pool_state(&self.pool, &snapshots, &dlmm_states).await?;
        rows_written += (snapshots.len() + dlmm_states.len()) as u64;
        rows_written += insert_active_bin_snapshots(&self.pool, &active_bins).await?;
        rows_written += insert_bin_states(&self.pool, &bin_states).await?;
        rows_written += insert_fee_param_updates(&self.pool, &fee_updates).await?;
        for params in &param_refreshes {
            upsert_dlmm_pool_params(&self.pool, params)
                .await
                .wrap_err_with(|| format!("Refreshing dlmm params for {}", params.pool_address))?;
        }

        buffer.clear();
        self.progress
            .record_write(max_slot, max_block_time, rows_written);

        Ok(())
    }

    async fn run_once(&self, ct: &CancellationToken) -> eyre::Result<()> {
        let watched = storage::queries::watch_set(&self.pool)
            .await
            .wrap_err_with(|| "Loading watch set")?;

        if watched.is_empty() {
            tokio::select! {
                _ = ct.cancelled() => {}
                _ = tokio::time::sleep(self.watch_refresh_interval) => {}
            }
            return Ok(());
        }

        let pools: Vec<Pubkey> = watched
            .iter()
            .filter_map(|w| w.pool_address.parse().ok())
            .collect();
        let mut stream = self.source.state_stream(WatchSet { pools });

        let mut buffer = StateBuffer::new();
        let deadline = Instant::now() + self.watch_refresh_interval;
        let mut flush_timer = tokio::time::interval(self.flush_interval);
        flush_timer.tick().await;

        loop {
            tokio::select! {
                biased;
                _ = ct.cancelled() => {
                    if let Err(e) = self.flush(&mut buffer).await {
                        tracing::error!(error = ?e, "Final state flush before shutdown failed");
                    }
                    return Ok(());
                }
                _ = tokio::time::sleep_until(deadline) => break,
                _ = flush_timer.tick() => {
                    if let Err(e) = self.flush(&mut buffer).await {
                        tracing::error!(error = ?e, "Flushing state buffer failed, will retry");
                    }
                }
                item = stream.next() => {
                    match item {
                        Some(update) => {
                            buffer.offer(update);
                            if buffer.len() >= self.flush_batch_size
                                && let Err(e) = self.flush(&mut buffer).await
                            {
                                tracing::error!(error = ?e, "Flushing state buffer failed, will retry");
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        self.flush(&mut buffer).await
    }
}

#[async_trait]
impl Worker for StateWorker {
    fn name(&self) -> &'static str {
        "state"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        while !ct.is_cancelled() {
            if let Err(e) = self.run_once(&ct).await {
                tracing::error!(error = ?e, "State ingestion cycle failed, retrying");
            }
        }
        Ok(())
    }
}
