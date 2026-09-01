use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use eyre::WrapErr;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use common::{Worker, tick_loop};
use metrics::{INGEST_LAG_SLOTS, PROCESSING_SLOT};
use storage::write::{NewIngestHealth, insert_ingest_health};

use crate::config::health::SLOT_TIME_SECS;
use crate::workers::progress::Progress;

/// Writes `ingest_health` every tick and keeps `processing_slot`/`ingest_lag_slots` current.
/// The heartbeat log line only fires when the data is both fresh (recent on-chain time) and
/// moving (more rows written than last tick) -- a wedged-but-connected process then falls
/// silent instead of logging a false "healthy", which is the point: silence becomes the
/// alert instead of a log line nobody is watching.
pub struct HealthWorker {
    pool: PgPool,
    progress: Arc<Progress>,
    interval: Duration,
    freshness_threshold: Duration,
    source_label: String,
    last_seen_rows: AtomicU64,
}

impl HealthWorker {
    pub fn new(
        pool: PgPool,
        progress: Arc<Progress>,
        interval: Duration,
        freshness_threshold: Duration,
        source_label: String,
    ) -> Self {
        Self {
            pool,
            progress,
            interval,
            freshness_threshold,
            source_label,
            last_seen_rows: AtomicU64::new(0),
        }
    }

    async fn tick(&self) -> eyre::Result<()> {
        let last_slot = self.progress.last_slot();
        let last_block_time = self.progress.last_block_time();
        let decode_errors = self.progress.take_decode_errors();

        let lag_secs = if last_block_time > 0 {
            Some((Utc::now().timestamp() - last_block_time).max(0))
        } else {
            None
        };
        let lag_slots = lag_secs.map(|s| (s as f64 / SLOT_TIME_SECS).round() as i64);

        if last_slot > 0 {
            PROCESSING_SLOT.set(last_slot as f64);
        }
        if let Some(slots) = lag_slots {
            INGEST_LAG_SLOTS.set(slots as f64);
        }

        let rows_now = self.progress.rows_written_total();
        let rows_since_last =
            rows_now.saturating_sub(self.last_seen_rows.swap(rows_now, Ordering::Relaxed));
        let fresh = lag_secs.is_some_and(|s| s <= self.freshness_threshold.as_secs() as i64);
        let progressed = rows_since_last > 0;

        if fresh && progressed {
            tracing::info!(
                lag_secs = ?lag_secs,
                rows_written = rows_since_last,
                "Heartbeat: ingest healthy and making progress"
            );
        }

        insert_ingest_health(
            &self.pool,
            &NewIngestHealth {
                ts: Utc::now(),
                source: self.source_label.clone(),
                last_slot: if last_slot > 0 { Some(last_slot) } else { None },
                slot_gap: lag_slots,
                messages: None,
                decode_errors: Some(decode_errors as i32),
                write_latency_ms: None,
            },
        )
        .await
        .wrap_err_with(|| "Writing ingest health")?;

        Ok(())
    }
}

#[async_trait]
impl Worker for HealthWorker {
    fn name(&self) -> &'static str {
        "health"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        tick_loop(ct, self.interval, || self.tick()).await;
        Ok(())
    }
}
