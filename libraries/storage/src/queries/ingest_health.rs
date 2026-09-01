use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct IngestHealthStatus {
    pub source: String,
    pub ts: DateTime<Utc>,
    pub last_slot: Option<i64>,
    pub slot_gap: Option<i64>,
    pub messages: Option<String>,
    pub decode_errors: Option<i32>,
    pub write_latency_ms: Option<i32>,
}

// Latest row per source. Geyser is forward-only, so a slot gap is permanent unless backfilled --
// /status has to show it, not bury it in logs.
pub async fn ingest_health(pool: &PgPool) -> eyre::Result<Vec<IngestHealthStatus>> {
    let rows = sqlx::query_as!(
        IngestHealthStatus,
        r#"
        SELECT DISTINCT ON (source)
            source, ts, last_slot, slot_gap, messages, decode_errors, write_latency_ms
        FROM ingest_health
        ORDER BY source, ts DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying ingest health")?;

    Ok(rows)
}
