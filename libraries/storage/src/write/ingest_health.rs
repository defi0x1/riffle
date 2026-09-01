use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewIngestHealth {
    pub ts: DateTime<Utc>,
    pub source: String,
    pub last_slot: Option<i64>,
    pub slot_gap: Option<i64>,
    pub messages: Option<String>,
    pub decode_errors: Option<i32>,
    pub write_latency_ms: Option<i32>,
}

// Operational telemetry with no natural key -- Prometheus is the durable store for this data, so
// a plain insert (not an upsert) is enough; a duplicate row on retry costs nothing here.
pub async fn insert_ingest_health(pool: &PgPool, row: &NewIngestHealth) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO ingest_health (ts, source, last_slot, slot_gap, messages, decode_errors, write_latency_ms)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        row.ts,
        row.source,
        row.last_slot,
        row.slot_gap,
        row.messages,
        row.decode_errors,
        row.write_latency_ms,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting ingest health row for {}", row.source))?;

    Ok(())
}
