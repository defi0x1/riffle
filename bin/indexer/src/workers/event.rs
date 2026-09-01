use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use eyre::WrapErr;
use futures::StreamExt;
use sqlx::PgPool;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use common::Worker;
use dlmm_decode::{DecodedEvent, LiquidityEventKind};
use source::{ChainEvent, EventFilter, Source};
use storage::types::liquidity_action;
use storage::write::{NewLiquidityEvent, NewSwap, insert_liquidity_events, insert_swaps};

use crate::convert::{decimal_from_u64, unix_to_datetime};
use crate::workers::progress::Progress;

/// Drives the chain event stream and writes swaps and liquidity events. Fee-parameter
/// changes are not observed here: the decoded event set has no `FeeParameterUpdate` variant
/// yet, so that is detected as a state diff by the state worker instead (see its `diff_fee_params`).
///
/// On the RPC backend the stream is permanently empty by design (a swap-level history would
/// need a signature walk per pool). This worker treats an immediately-ended stream as the
/// idle case and backs off rather than reopening it in a tight loop.
pub struct EventWorker {
    pool: PgPool,
    source: Arc<dyn Source>,
    progress: Arc<Progress>,
    flush_interval: Duration,
    flush_batch_size: usize,
}

enum BreakReason {
    Deadline,
    StreamEnded,
}

impl EventWorker {
    pub fn new(
        pool: PgPool,
        source: Arc<dyn Source>,
        progress: Arc<Progress>,
        flush_interval: Duration,
        flush_batch_size: usize,
    ) -> Self {
        Self {
            pool,
            source,
            progress,
            flush_interval,
            flush_batch_size,
        }
    }

    // ChainEvent carries no transaction signature or instruction index yet -- there is no
    // wire-level source for one until the Geyser backend lands. A synthetic key keeps
    // multiple events in the same slot from colliding under the schema's (pool, ts,
    // signature, ix_index) uniqueness rather than silently dropping all but one.
    fn synthetic_signature(event: &ChainEvent, nonce: u32) -> String {
        format!("chain-event:{}:{}:{}", event.pool, event.slot, nonce)
    }

    fn handle_event(
        event: ChainEvent,
        nonce: u32,
        swaps: &mut Vec<NewSwap>,
        liquidity: &mut Vec<NewLiquidityEvent>,
    ) {
        let pool_address = event.pool.to_string();
        let ts = unix_to_datetime(event.block_time);
        let signature = Self::synthetic_signature(&event, nonce);

        match event.event {
            DecodedEvent::Swap(swap) => {
                let fee_raw = decimal_from_u64(swap.lp_fee) + decimal_from_u64(swap.protocol_fee);
                swaps.push(NewSwap {
                    pool_address,
                    ts,
                    slot: event.slot as i64,
                    signature,
                    ix_index: 0,
                    signer: swap.trader.to_string(),
                    swap_for_y: swap.swap_for_y,
                    amount_in_raw: decimal_from_u64(swap.amount_in),
                    amount_out_raw: decimal_from_u64(swap.amount_out),
                    // Token decimals are not resolved at this layer, so the UI-scaled
                    // amounts are left equal to the raw ones for now.
                    amount_in: decimal_from_u64(swap.amount_in),
                    amount_out: decimal_from_u64(swap.amount_out),
                    start_bin_id: swap.start_bin_id,
                    end_bin_id: swap.end_bin_id,
                    start_price: None,
                    end_price: None,
                    fee_raw,
                    protocol_fee_raw: decimal_from_u64(swap.protocol_fee),
                    host_fee_raw: Some(decimal_from_u64(swap.host_fee)),
                    fee_bps: decimal_from_u64(swap.fee_bps),
                    volume_usd: None,
                    trade_fee_usd: None,
                    protocol_fee_usd: None,
                });
            }
            DecodedEvent::AddLiquidity(liq) | DecodedEvent::RemoveLiquidity(liq) => {
                let action = match liq.kind {
                    LiquidityEventKind::Add => liquidity_action::ADD,
                    LiquidityEventKind::Remove => liquidity_action::REMOVE,
                };
                liquidity.push(NewLiquidityEvent {
                    pool_address,
                    ts,
                    slot: event.slot as i64,
                    signature,
                    ix_index: 0,
                    position_address: Some(liq.position.to_string()),
                    owner: liq.from.to_string(),
                    action,
                    active_bin_id: liq.active_bin_id,
                    amount_x_raw: Some(decimal_from_u64(liq.amount_x)),
                    amount_y_raw: Some(decimal_from_u64(liq.amount_y)),
                    amount_usd: None,
                });
            }
            // ClaimFee/ClaimFee2/LbPairCreate/PositionCreate/PositionClose have no write
            // path in the current schema; a future table can pick these up without
            // changing anything upstream of here.
            other => {
                tracing::debug!(
                    pool = %pool_address,
                    kind = event_kind_label(&other),
                    "Unhandled chain event kind, skipping"
                );
            }
        }
    }

    async fn flush(
        &self,
        swaps: &mut Vec<NewSwap>,
        liquidity: &mut Vec<NewLiquidityEvent>,
    ) -> eyre::Result<()> {
        if swaps.is_empty() && liquidity.is_empty() {
            return Ok(());
        }

        let mut rows_written = 0u64;
        rows_written += insert_swaps(&self.pool, swaps)
            .await
            .wrap_err_with(|| "Inserting swaps")?;
        rows_written += insert_liquidity_events(&self.pool, liquidity)
            .await
            .wrap_err_with(|| "Inserting liquidity events")?;

        let max_slot = swaps
            .iter()
            .map(|s| s.slot)
            .chain(liquidity.iter().map(|l| l.slot))
            .max()
            .unwrap_or(0);

        swaps.clear();
        liquidity.clear();

        if max_slot > 0 {
            self.progress.record_write(max_slot as u64, 0, rows_written);
        }

        Ok(())
    }

    async fn run_once(&self, ct: &CancellationToken) -> eyre::Result<()> {
        let mut stream = self.source.event_stream(EventFilter::default());

        let mut swaps = Vec::new();
        let mut liquidity = Vec::new();
        let mut nonce = 0u32;

        let deadline = Instant::now() + self.flush_interval * 12;
        let mut flush_timer = tokio::time::interval(self.flush_interval);
        flush_timer.tick().await;

        let break_reason = loop {
            tokio::select! {
                biased;
                _ = ct.cancelled() => {
                    if let Err(e) = self.flush(&mut swaps, &mut liquidity).await {
                        tracing::error!(error = ?e, "Final event flush before shutdown failed");
                    }
                    return Ok(());
                }
                _ = tokio::time::sleep_until(deadline) => break BreakReason::Deadline,
                _ = flush_timer.tick() => {
                    if let Err(e) = self.flush(&mut swaps, &mut liquidity).await {
                        tracing::error!(error = ?e, "Flushing event buffer failed, will retry");
                    }
                }
                item = stream.next() => {
                    match item {
                        Some(event) => {
                            nonce = nonce.wrapping_add(1);
                            Self::handle_event(event, nonce, &mut swaps, &mut liquidity);
                            if swaps.len() + liquidity.len() >= self.flush_batch_size
                                && let Err(e) = self.flush(&mut swaps, &mut liquidity).await
                            {
                                tracing::error!(error = ?e, "Flushing event buffer failed, will retry");
                            }
                        }
                        None => break BreakReason::StreamEnded,
                    }
                }
            }
        };

        self.flush(&mut swaps, &mut liquidity).await?;

        if matches!(break_reason, BreakReason::StreamEnded) {
            // The RPC backend's event stream ends immediately, every time, by design --
            // idle here instead of reopening it in a tight loop.
            tokio::select! {
                _ = ct.cancelled() => {}
                _ = tokio::time::sleep(self.flush_interval) => {}
            }
        }

        Ok(())
    }
}

fn event_kind_label(event: &DecodedEvent) -> &'static str {
    match event {
        DecodedEvent::ClaimFee(_) => "claim_fee",
        DecodedEvent::ClaimFee2(_) => "claim_fee2",
        DecodedEvent::LbPairCreate(_) => "lb_pair_create",
        DecodedEvent::PositionCreate(_) => "position_create",
        DecodedEvent::PositionClose(_) => "position_close",
        DecodedEvent::Swap(_)
        | DecodedEvent::AddLiquidity(_)
        | DecodedEvent::RemoveLiquidity(_) => "handled",
    }
}

#[async_trait]
impl Worker for EventWorker {
    fn name(&self) -> &'static str {
        "event"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        while !ct.is_cancelled() {
            if let Err(e) = self.run_once(&ct).await {
                tracing::error!(error = ?e, "Event ingestion cycle failed, retrying");
            }
        }
        Ok(())
    }
}
