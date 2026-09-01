use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::Worker;
use engine::sizing::SizingInput;
use engine::{EngineConfig, Regime};
use eyre::WrapErr;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use sqlx::PgPool;
use storage::queries::{
    OpenPaperPosition, PositionDueForOutcome, latest_active_bin_snapshot, open_paper_positions,
    pool_detail, pool_metrics_recent, position_marks_since, positions_due_for_outcome, watch_set,
};
use storage::types::Timeframe;
use storage::write::{
    NewOutcome, NewPaperPosition, NewPositionMark, insert_position_marks, open_paper_position,
    upsert_outcome,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{estimated_fee_share, is_in_range};
use crate::config::PipelineDefaultsConfig;
use crate::signals::{SignalKind, classify};

/// Horizons this pass finalises outcomes at. `04`'s S regime uses a 14-day horizon instead
/// of 72h; that variant is not implemented here -- every position gets the same two
/// horizons regardless of regime, a scoping simplification worth revisiting once the
/// evidence base has enough volume to care about the difference.
const OUTCOME_HORIZONS: &[(&str, i64)] = &[("24h", 24), ("72h", 72)];

/// Opens a paper position when a pool crosses the same attractiveness threshold a POTENTIAL
/// signal would (never touches the chain -- these are rows in a table), marks every open
/// position on a 5-minute cadence from the pool's own fee income, and finalises outcomes once
/// a position has been open long enough. Runs two independent tick cadences under one
/// `Worker` since open/mark and outcome-checking have different natural intervals.
pub struct PaperPositionWorker {
    pool: PgPool,
    mark_interval: Duration,
    outcomes_interval: Duration,
    engine_cfg: EngineConfig,
    defaults: PipelineDefaultsConfig,
}

impl PaperPositionWorker {
    pub fn new(
        pool: PgPool,
        mark_interval: Duration,
        outcomes_interval: Duration,
        engine_cfg: EngineConfig,
        defaults: PipelineDefaultsConfig,
    ) -> Self {
        Self {
            pool,
            mark_interval,
            outcomes_interval,
            engine_cfg,
            defaults,
        }
    }

    async fn open_and_mark_tick(&self) -> eyre::Result<()> {
        let now = Utc::now();
        if let Err(e) = self.open_new_positions(now).await {
            tracing::warn!(error = ?e, "Opening paper positions failed, continuing");
        }
        if let Err(e) = self.mark_open_positions(now).await {
            tracing::warn!(error = ?e, "Marking paper positions failed, continuing");
        }
        Ok(())
    }

    async fn outcomes_tick(&self) -> eyre::Result<()> {
        let now = Utc::now();
        for (horizon, hours) in OUTCOME_HORIZONS {
            let cutoff = now - chrono::Duration::hours(*hours);
            let due = positions_due_for_outcome(&self.pool, horizon, cutoff)
                .await
                .wrap_err_with(|| format!("Querying positions due for {horizon}"))?;
            for position in due {
                if let Err(e) = self
                    .finalize_outcome(&position, horizon, chrono::Duration::hours(*hours), now)
                    .await
                {
                    tracing::warn!(
                        error = ?e,
                        position = %position.id,
                        horizon,
                        "Finalising outcome failed, continuing"
                    );
                }
            }
        }
        Ok(())
    }

    async fn open_new_positions(&self, now: DateTime<Utc>) -> eyre::Result<()> {
        let watched = watch_set(&self.pool)
            .await
            .wrap_err_with(|| "Loading watch set for paper positions")?;
        let open = open_paper_positions(&self.pool)
            .await
            .wrap_err_with(|| "Loading open paper positions")?;
        let already_open: HashSet<&str> = open.iter().map(|p| p.pool_address.as_str()).collect();

        for w in &watched {
            if already_open.contains(w.pool_address.as_str()) {
                continue;
            }
            if let Err(e) = self.try_open(w, now).await {
                tracing::warn!(error = ?e, pool = %w.pool_address, "Opening paper position failed");
            }
        }
        Ok(())
    }

    // Opens against the 1h timeframe: stable enough not to flip on every 5-minute tick,
    // short enough to react to a pool that just started qualifying.
    async fn try_open(
        &self,
        watched: &storage::queries::WatchedPool,
        now: DateTime<Utc>,
    ) -> eyre::Result<()> {
        let tf = Timeframe::H1;
        let Some(detail) = pool_detail(&self.pool, &watched.pool_address).await? else {
            return Ok(());
        };
        let Some(row) = detail.h1 else { return Ok(()) };
        if classify(&row, &self.engine_cfg.ranking, false) != Some(SignalKind::Potential) {
            return Ok(());
        }
        let Some(regime) = row.regime.as_deref().and_then(|s| Regime::from_str(s).ok()) else {
            return Ok(());
        };

        let current = pool_metrics_recent(&self.pool, tf, &watched.pool_address, now, 1).await?;
        let Some(current) = current.first() else {
            return Ok(());
        };
        let (Some(price), Some(active_bin)) = (current.price_close, current.active_bin_close)
        else {
            return Ok(());
        };

        let active_bin_liquidity = latest_active_bin_snapshot(&self.pool, &watched.pool_address)
            .await?
            .and_then(|s| s.quote_value_usd)
            .and_then(|d| d.to_f64())
            .unwrap_or(0.0);

        let fee_rate = row.f_hat.and_then(|d| d.to_f64()).unwrap_or(0.0);
        let tau_a = row.tau_a.unwrap_or(0.0);
        let sigma_d = row.sigma_d.unwrap_or(0.0);
        if sigma_d <= 0.0 {
            return Ok(());
        }
        let tvl_usd = detail.pool.tvl_usd.and_then(|d| d.to_f64()).unwrap_or(0.0);
        let protocol_share = detail.pool.protocol_share_bps as f64 / 10_000.0;

        let sizing_input = SizingInput {
            active_bin_liquidity,
            protocol_share,
            fee_rate,
            tau_a,
            tvl_usd,
            regime_capital: self.defaults.regime_capital,
            mu_fee: self.defaults.mu_fee,
            mu_arb: self.defaults.mu_arb,
            sigma_hat: sigma_d,
            free_capital: self.defaults.free_capital,
        };
        let (sizing_out, _) =
            engine::sizing::evaluate(&sizing_input, regime, &self.engine_cfg.sizing);
        let Some(v_star) = sizing_out.v_star else {
            return Ok(());
        };

        let horizon_days = if regime == Regime::V2 { 0.5 } else { 1.0 };
        let w_half = dlmm_math::range_half_width(sigma_d, horizon_days);
        let bin_step_fraction = detail.pool.bin_step as f64 / 10_000.0;
        if bin_step_fraction <= 0.0 {
            return Ok(());
        }
        let n_bins = dlmm_math::bin_count_for_half_width(w_half, bin_step_fraction) as i32;
        let size_per_bin = v_star / self.engine_cfg.sizing.position_count as f64;

        let position = NewPaperPosition {
            id: Uuid::new_v4(),
            signal_id: None,
            pool_address: watched.pool_address.clone(),
            venue: watched.venue,
            opened_at: now,
            regime: Some(regime.to_string()),
            entry_price: Some(price),
            entry_active_bin: Some(active_bin),
            lower_bin: Some(active_bin - n_bins),
            upper_bin: Some(active_bin + n_bins),
            shape: Some("spot".to_string()),
            size_usd: Decimal::from_f64(v_star),
            size_per_bin: Decimal::from_f64(size_per_bin),
            predicted: Some(serde_json::json!({ "r_org": row.r_org, "y_fee": row.y_fee })),
        };
        open_paper_position(&self.pool, &position)
            .await
            .wrap_err_with(|| "Opening paper position")?;
        tracing::info!(pool = %watched.pool_address, size_usd = v_star, "Opened paper position");
        Ok(())
    }

    async fn mark_open_positions(&self, now: DateTime<Utc>) -> eyre::Result<()> {
        let open = open_paper_positions(&self.pool)
            .await
            .wrap_err_with(|| "Loading open paper positions to mark")?;

        let mut marks = Vec::with_capacity(open.len());
        for position in &open {
            match self.build_mark(position, now).await {
                Ok(Some(mark)) => marks.push(mark),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = ?e, position = %position.id, "Marking position failed")
                }
            }
        }
        insert_position_marks(&self.pool, &marks)
            .await
            .wrap_err_with(|| "Persisting position marks")?;
        Ok(())
    }

    async fn build_mark(
        &self,
        position: &OpenPaperPosition,
        now: DateTime<Utc>,
    ) -> eyre::Result<Option<NewPositionMark>> {
        let current =
            pool_metrics_recent(&self.pool, Timeframe::M5, &position.pool_address, now, 1).await?;
        let Some(current) = current.first() else {
            return Ok(None);
        };
        let (Some(price), Some(active_bin)) = (current.price_close, current.active_bin_close)
        else {
            return Ok(None);
        };
        let (Some(lower), Some(upper)) = (position.lower_bin, position.upper_bin) else {
            return Ok(None);
        };
        let in_range = is_in_range(active_bin, lower, upper);

        let active_bin_liquidity = latest_active_bin_snapshot(&self.pool, &position.pool_address)
            .await?
            .and_then(|s| s.quote_value_usd)
            .and_then(|d| d.to_f64())
            .unwrap_or(0.0);
        let pool_fee_usd = current
            .trade_fee_usd
            .and_then(|d| d.to_f64())
            .unwrap_or(0.0);
        let size_per_bin = position
            .size_per_bin
            .and_then(|d| d.to_f64())
            .unwrap_or(0.0);
        let fees_this_interval =
            estimated_fee_share(pool_fee_usd, size_per_bin, active_bin_liquidity, in_range);

        let entry_price = position.entry_price.unwrap_or(price);
        let size_usd = position.size_usd.and_then(|d| d.to_f64()).unwrap_or(0.0);
        let width = ((upper - lower) as f64) * (position.bin_step as f64 / 10_000.0);
        let il_usd = if entry_price > 0.0 && width > 0.0 {
            let delta = ((price - entry_price) / entry_price).clamp(-width / 2.0, width / 2.0);
            dlmm_math::il_spot(size_usd, delta, width)
        } else {
            0.0
        };

        let cumulative_fees: f64 =
            position_marks_since(&self.pool, position.id, position.opened_at)
                .await?
                .iter()
                .filter_map(|m| m.fees_accrued_usd.and_then(|d| d.to_f64()))
                .sum();
        let value_usd = size_usd + cumulative_fees + fees_this_interval - il_usd;

        Ok(Some(NewPositionMark {
            position_id: position.id,
            ts: now,
            price: Some(price),
            active_bin_id: Some(active_bin),
            fees_accrued_usd: Decimal::from_f64(fees_this_interval),
            il_usd: Decimal::from_f64(il_usd),
            value_usd: Decimal::from_f64(value_usd),
            in_range: Some(in_range),
        }))
    }

    async fn finalize_outcome(
        &self,
        position: &PositionDueForOutcome,
        horizon: &str,
        horizon_duration: chrono::Duration,
        now: DateTime<Utc>,
    ) -> eyre::Result<()> {
        let marks = position_marks_since(&self.pool, position.id, position.opened_at)
            .await
            .wrap_err_with(|| "Loading position marks for outcome")?;
        let cutoff = position.opened_at + horizon_duration;
        let in_horizon: Vec<_> = marks.iter().filter(|m| m.ts <= cutoff).collect();

        let fees_real: Decimal = in_horizon.iter().filter_map(|m| m.fees_accrued_usd).sum();
        let lvr_real = in_horizon.last().and_then(|m| m.il_usd);
        let time_in_range = if in_horizon.is_empty() {
            None
        } else {
            let in_range_count = in_horizon
                .iter()
                .filter(|m| m.in_range == Some(true))
                .count();
            Some(in_range_count as f64 / in_horizon.len() as f64)
        };

        let predicted_r_org = position
            .predicted
            .as_ref()
            .and_then(|v| v.get("r_org"))
            .and_then(|v| v.as_f64());
        let predicted_y_fee = position
            .predicted
            .as_ref()
            .and_then(|v| v.get("y_fee"))
            .and_then(|v| v.as_f64());
        let fees_predicted = predicted_y_fee.and_then(|y| {
            position
                .size_usd
                .and_then(|s| s.to_f64())
                .map(|s| y * s * (horizon_duration.num_hours() as f64 / 24.0))
        });

        let r_real = match (fees_real.to_f64(), lvr_real.and_then(|d| d.to_f64())) {
            (Some(fees), Some(lvr)) if lvr.abs() > 1e-9 => Some(fees / lvr),
            _ => None,
        };
        let hit = r_real.map(|r| r >= 1.0);

        let outcome = NewOutcome {
            position_id: position.id,
            horizon: horizon.to_string(),
            venue: position.venue,
            finalized_at: now,
            fees_real: Some(fees_real),
            fees_predicted: fees_predicted.and_then(Decimal::from_f64),
            lvr_real,
            r_real,
            r_predicted: predicted_r_org,
            time_in_range,
            hit,
        };
        upsert_outcome(&self.pool, &outcome)
            .await
            .wrap_err_with(|| "Persisting outcome")?;
        Ok(())
    }
}

#[async_trait]
impl Worker for PaperPositionWorker {
    fn name(&self) -> &'static str {
        "paper_positions"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        let open_and_mark =
            common::tick_loop(ct.clone(), self.mark_interval, || self.open_and_mark_tick());
        let outcomes = common::tick_loop(ct, self.outcomes_interval, || self.outcomes_tick());
        tokio::join!(open_and_mark, outcomes);
        Ok(())
    }
}
