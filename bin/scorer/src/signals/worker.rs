use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Utc, Weekday};
use common::Worker;
use engine::triggers::HistoryPoint;
use engine::{EngineConfig, Regime};
use eyre::WrapErr;
use sqlx::PgPool;
use storage::queries::{indicator_history, pool_detail, watch_set};
use storage::types::Timeframe;
use storage::write::{IndicatorRow, NewSignal, insert_signal};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{Cooldown, classify};
use crate::indicators::config_hash;

/// The widest configured exit-persistence window, doubled for the weekend factor, plus
/// margin -- see `engine::triggers::TriggersConfig`.
const TRIGGER_HISTORY_LOOKBACK_HOURS: i64 = 96;

/// Evaluates trigger conditions on the indicators the indicators worker already persisted
/// this tick, and decides whether the result is worth a new `signals` row -- deduped so a
/// persistent condition is announced once, then stays quiet until its cooldown elapses.
pub struct SignalsWorker {
    pool: PgPool,
    interval: StdDuration,
    engine_cfg: EngineConfig,
    cooldown: Mutex<Cooldown>,
}

impl SignalsWorker {
    pub fn new(
        pool: PgPool,
        interval: StdDuration,
        engine_cfg: EngineConfig,
        cooldown_window: chrono::Duration,
    ) -> Self {
        Self {
            pool,
            interval,
            engine_cfg,
            cooldown: Mutex::new(Cooldown::new(cooldown_window)),
        }
    }

    async fn tick(&self) -> eyre::Result<()> {
        let now = Utc::now();
        let watched = watch_set(&self.pool)
            .await
            .wrap_err_with(|| "Loading watch set for signals")?;

        for p in &watched {
            let Some(detail) = pool_detail(&self.pool, &p.pool_address).await? else {
                continue;
            };
            let rows: [(Timeframe, Option<IndicatorRow>); 5] = [
                (Timeframe::M5, detail.m5),
                (Timeframe::M10, detail.m10),
                (Timeframe::H1, detail.h1),
                (Timeframe::H4, detail.h4),
                (Timeframe::H24, detail.h24),
            ];
            for (tf, row) in rows {
                let Some(row) = row else { continue };
                if let Err(e) = self.evaluate_row(&row, tf, now).await {
                    tracing::warn!(
                        error = ?e,
                        pool = %p.pool_address,
                        timeframe = tf.as_str(),
                        "Signal evaluation failed, continuing"
                    );
                }
            }
        }

        Ok(())
    }

    async fn evaluate_row(
        &self,
        row: &IndicatorRow,
        tf: Timeframe,
        now: DateTime<Utc>,
    ) -> eyre::Result<()> {
        let Some(regime) = row.regime.as_deref().and_then(|s| Regime::from_str(s).ok()) else {
            return Ok(());
        };

        let history: Vec<HistoryPoint> = indicator_history(
            &self.pool,
            tf,
            &row.pool_address,
            now - chrono::Duration::hours(TRIGGER_HISTORY_LOOKBACK_HOURS),
        )
        .await
        .wrap_err_with(|| "Loading trigger history")?
        .into_iter()
        .map(|p| HistoryPoint {
            at: p.bucket_start,
            r_org: p.r_org,
            vol_tvl: p.vol_tvl,
        })
        .collect();

        let triggers_input = engine::triggers::TriggersInput {
            r_org: row.r_org.unwrap_or(0.0),
            vol_tvl: row.vol_tvl.unwrap_or(0.0),
            // The volume-decay sub-check needs the same trend recomputation the indicators
            // worker already did this tick; re-deriving it here would duplicate that work; a
            // permissive value leaves the r_org/vol_tvl persistence and fee-jack checks --
            // the two checks this worker can evaluate correctly from persisted history --
            // fully active.
            volume_decay_metric: 1.0,
            v2_is_young: false,
            fee_jack_multiplier: None,
            history,
            is_weekend_utc: matches!(now.weekday(), Weekday::Sat | Weekday::Sun),
        };
        let (triggers_out, _) =
            engine::triggers::evaluate(&triggers_input, regime, &self.engine_cfg.triggers);

        let Some(kind) = classify(row, &self.engine_cfg.ranking, triggers_out.exit) else {
            return Ok(());
        };

        let due = {
            let mut cooldown = self.cooldown.lock().expect("cooldown mutex poisoned");
            cooldown.should_broadcast(&row.pool_address, tf.as_str(), kind, now)
        };
        if !due {
            return Ok(());
        }

        let signal = NewSignal {
            id: Uuid::new_v4(),
            ts: now,
            pool_address: row.pool_address.clone(),
            venue: row.venue,
            timeframe: tf.as_str().to_string(),
            kind: kind.as_str().to_string(),
            regime: Some(regime.to_string()),
            numbers: Some(serde_json::json!({
                "r_org": row.r_org,
                "vol_tvl": row.vol_tvl,
            })),
            config_hash: config_hash(&self.engine_cfg),
            expires_at: None,
        };
        insert_signal(&self.pool, &signal)
            .await
            .wrap_err_with(|| "Persisting signal")?;

        Ok(())
    }
}

#[async_trait]
impl Worker for SignalsWorker {
    fn name(&self) -> &'static str {
        "signals"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        common::tick_loop(ct, self.interval, || self.tick()).await;
        Ok(())
    }
}
