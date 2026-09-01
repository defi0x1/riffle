use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Utc, Weekday};
use common::Worker;
use dlmm_math::{Dlmm, VenueId};
use engine::Quality;
use engine::regime::RegimeState;
use engine::triggers::HistoryPoint;
use engine::volatility::VolatilityState;
use engine::{EngineConfig, PipelineInput, rank, screen};
use eyre::WrapErr;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgPool;
use storage::queries::{
    indicator_history, latest_active_bin_snapshot, load_regime_state, load_volatility_state,
    pool_metrics_recent, scoring_universe,
};
use storage::types::{Timeframe, tier, venue};
use storage::write::{
    NewSignal, insert_signal_with_rationale, upsert_indicators, upsert_regime_state,
    upsert_volatility_state,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{config_hash, to_indicator_row, to_rationale_rows};
use crate::config::PipelineDefaultsConfig;
use crate::pipeline;
use crate::state::{
    regime_state_from_row, regime_state_to_row, volatility_state_from_row, volatility_state_to_row,
};

/// How many history rows to fetch per pool per timeframe: 7 days at 5-minute resolution plus
/// a small margin. Timeframes with fewer bars per day naturally get more calendar history for
/// the same row count, which is fine -- the volume-trend and consistency windows only use as
/// much of it as their own bar count needs.
const HISTORY_ROWS: i64 = 288 * 7 + 4;

/// Lookback for the trigger-persistence history: the widest configured exit window is S's
/// 48-hour `vol_tvl` persistence, so 72 hours leaves margin.
const TRIGGER_HISTORY_LOOKBACK_HOURS: i64 = 72;

/// Screens the whole universe and ranks the watch set, once per timeframe per tick.
/// Regime and volatility state round-trip through the database every evaluation, so
/// hysteresis survives a restart. Every evaluation -- gated or not, potential or not --
/// writes a `signals` row (kind `INFO`) with its full rationale trail; the signal worker
/// decides separately whether a particular evaluation is worth announcing.
pub struct IndicatorsWorker {
    pool: PgPool,
    interval: Duration,
    engine_cfg: EngineConfig,
    defaults: PipelineDefaultsConfig,
}

impl IndicatorsWorker {
    pub fn new(
        pool: PgPool,
        interval: Duration,
        engine_cfg: EngineConfig,
        defaults: PipelineDefaultsConfig,
    ) -> Self {
        Self {
            pool,
            interval,
            engine_cfg,
            defaults,
        }
    }

    async fn tick(&self) -> eyre::Result<()> {
        let now = Utc::now();
        let universe = scoring_universe(&self.pool, venue::DLMM)
            .await
            .wrap_err_with(|| "Loading scoring universe for indicators")?;

        for tf in Timeframe::ALL {
            for pool_meta in &universe {
                if let Err(e) = self.evaluate_pool(pool_meta, tf, now).await {
                    tracing::warn!(
                        error = ?e,
                        pool = %pool_meta.pool_address,
                        timeframe = tf.as_str(),
                        "Indicator evaluation failed, continuing"
                    );
                }
            }
        }

        Ok(())
    }

    async fn evaluate_pool(
        &self,
        pool_meta: &storage::queries::PoolForScoring,
        tf: Timeframe,
        now: DateTime<Utc>,
    ) -> eyre::Result<()> {
        let history =
            pool_metrics_recent(&self.pool, tf, &pool_meta.pool_address, now, HISTORY_ROWS)
                .await
                .wrap_err_with(|| "Fetching pool_metrics history")?;
        let Some(assembled) = pipeline::assemble(&history, tf) else {
            // No bucket at all yet for this pool/timeframe -- nothing to evaluate, and
            // nothing to explain the silence of either.
            return Ok(());
        };
        let bucket_start = history[0].bucket_start;

        let regime_row = load_regime_state(
            &self.pool,
            &pool_meta.pool_address,
            venue::DLMM,
            tf.as_str(),
        )
        .await
        .wrap_err_with(|| "Loading regime state")?;
        let mut regime_state = regime_row
            .as_ref()
            .map(regime_state_from_row)
            .unwrap_or_else(|| RegimeState::new(now));
        let previous_regime = regime_state.regime;

        let vol_row = load_volatility_state(
            &self.pool,
            &pool_meta.pool_address,
            venue::DLMM,
            tf.as_str(),
        )
        .await
        .wrap_err_with(|| "Loading volatility state")?;
        let mut vol_state = vol_row
            .as_ref()
            .map(volatility_state_from_row)
            .unwrap_or_else(|| VolatilityState::new(now));

        let trigger_history: Vec<HistoryPoint> = indicator_history(
            &self.pool,
            tf,
            &pool_meta.pool_address,
            now - chrono::Duration::hours(TRIGGER_HISTORY_LOOKBACK_HOURS),
        )
        .await
        .wrap_err_with(|| "Loading indicator trigger history")?
        .into_iter()
        .map(|p| HistoryPoint {
            at: p.bucket_start,
            r_org: p.r_org,
            vol_tvl: p.vol_tvl,
        })
        .collect();

        let is_watched = pool_meta.tier == tier::WATCHED;
        let measured_active_bin_liquidity = if is_watched {
            latest_active_bin_snapshot(&self.pool, &pool_meta.pool_address)
                .await
                .wrap_err_with(|| "Loading latest active bin snapshot")?
                .and_then(|s| s.quote_value_usd)
                .and_then(|d| d.to_f64())
        } else {
            None
        };

        let age_days = pipeline::age_days(pool_meta, now);
        let input = PipelineInput {
            pool_address: pool_meta.pool_address.clone(),
            venue: VenueId::Dlmm,
            bucket_start,
            now,
            latest_bar: assembled.latest_bar,
            autocorrelations: assembled.autocorrelations,
            log_returns_24h: assembled.log_returns_24h,
            decay_window_secs: self.defaults.decay_window_secs,
            dev_peg: None,
            is_pegged_whitelisted: false,
            is_major: false,
            age_days,
            kill_switch: false,
            risk: pipeline::risk_gate_inputs(pool_meta, now),
            bin_step_bps: pool_meta.bin_step as u16,
            base_factor: pool_meta.base_factor.clamp(0, i32::from(u16::MAX)) as u16,
            // Not carried by dlmm_pool_params; see storage::queries::scoring_universe.
            base_fee_power_factor: 0,
            variable_fee_control: pool_meta.variable_fee_control.max(0) as u32,
            protocol_share: pool_meta.protocol_share_bps as f64 / 10_000.0,
            tvl_usd: assembled.tvl_usd,
            measured_active_bin_liquidity,
            kappa_c: self.defaults.kappa_c,
            trade_sizes: Vec::new(),
            phi_time: None,
            n_trades: assembled.n_trades,
            c_fill: pipeline::c_fill_for(previous_regime),
            vol_24h: assembled.vol_24h,
            organic_class_prior_mu: self.defaults.organic_class_prior_mu,
            organic_class_prior_tau_sq: self.defaults.organic_class_prior_tau_sq,
            volume_trend: assembled.volume_trend,
            v2_is_young: age_days < 7.0,
            fee_tvl_1h: assembled.fee_tvl_1h,
            fee_tvl_24h: assembled.fee_tvl_24h,
            fee_tvl_7d: assembled.fee_tvl_7d,
            regime_capital: self.defaults.regime_capital,
            mu_fee: self.defaults.mu_fee,
            mu_arb: self.defaults.mu_arb,
            free_capital: self.defaults.free_capital,
            trigger_history,
            fee_jack_multiplier: None,
            is_weekend_utc: matches!(now.weekday(), Weekday::Sat | Weekday::Sun),
            previous: assembled.previous,
        };

        let result = if is_watched {
            rank(
                input,
                &Dlmm,
                &mut vol_state,
                &mut regime_state,
                &self.engine_cfg,
            )
        } else {
            screen(
                input,
                &Dlmm,
                &mut vol_state,
                &mut regime_state,
                &self.engine_cfg,
            )
        };

        upsert_regime_state(
            &self.pool,
            &regime_state_to_row(
                &pool_meta.pool_address,
                venue::DLMM,
                tf.as_str(),
                &regime_state,
                now,
            ),
        )
        .await
        .wrap_err_with(|| "Saving regime state")?;
        upsert_volatility_state(
            &self.pool,
            &volatility_state_to_row(
                &pool_meta.pool_address,
                venue::DLMM,
                tf.as_str(),
                &vol_state,
                now,
            ),
        )
        .await
        .wrap_err_with(|| "Saving volatility state")?;

        let row = to_indicator_row(&result.indicators);
        upsert_indicators(&self.pool, tf, std::slice::from_ref(&row))
            .await
            .wrap_err_with(|| "Persisting indicators row")?;

        let signal_id = Uuid::new_v4();
        let quality = match result.indicators.quality {
            Quality::A => "A",
            Quality::B => "B",
        };
        let numbers = serde_json::json!({
            "quality": quality,
            "r_org": result.indicators.r_org,
            "vol_tvl": result.indicators.vol_tvl,
            "phi_org": result.indicators.phi_org,
        });
        let signal = NewSignal {
            id: signal_id,
            ts: now,
            pool_address: pool_meta.pool_address.clone(),
            venue: venue::DLMM,
            timeframe: tf.as_str().to_string(),
            kind: "INFO".to_string(),
            regime: result.indicators.regime.map(|r| r.to_string()),
            numbers: Some(numbers),
            config_hash: config_hash(&self.engine_cfg),
            expires_at: None,
        };
        let rationale = to_rationale_rows(signal_id, venue::DLMM, &result.rationale);
        insert_signal_with_rationale(&self.pool, &signal, &rationale)
            .await
            .wrap_err_with(|| "Persisting signal and rationale")?;

        Ok(())
    }
}

#[async_trait]
impl Worker for IndicatorsWorker {
    fn name(&self) -> &'static str {
        "indicators"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        common::tick_loop(ct, self.interval, || self.tick()).await;
        Ok(())
    }
}
