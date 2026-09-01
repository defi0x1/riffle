use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct NewSignal {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub pool_address: String,
    pub venue: i16,
    pub timeframe: String,
    // POTENTIAL | DEGRADING | GATE_FAIL | INFO, left open-ended rather than a CHECK enum so a
    // new kind is an application change, not a migration.
    pub kind: String,
    pub regime: Option<String>,
    pub numbers: Option<serde_json::Value>,
    pub config_hash: String,
    pub expires_at: Option<DateTime<Utc>>,
}

pub async fn insert_signal<'e, E: PgExecutor<'e>>(
    executor: E,
    row: &NewSignal,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO signals (id, ts, pool_address, venue, timeframe, kind, regime, numbers, config_hash, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
        row.id,
        row.ts,
        row.pool_address,
        row.venue,
        row.timeframe,
        row.kind,
        row.regime,
        row.numbers,
        row.config_hash,
        row.expires_at,
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Inserting signal {}", row.id))?;

    Ok(())
}

#[derive(Clone, Debug)]
pub struct NewRationaleItem {
    pub signal_id: Uuid,
    pub seq: i32,
    pub venue: i16,
    pub signal: String,
    pub observed: Option<String>,
    pub cmp: Option<String>,
    pub threshold: Option<String>,
    pub passed: bool,
    pub note: Option<String>,
}

// Written for every evaluated condition, pass or fail, including evaluations that emit no
// signal -- one row per evaluated condition is the whole point, so this is always a batch.
pub async fn insert_rationale<'e, E: PgExecutor<'e>>(
    executor: E,
    rows: &[NewRationaleItem],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let signal_id: Vec<Uuid> = rows.iter().map(|r| r.signal_id).collect();
    let seq: Vec<i32> = rows.iter().map(|r| r.seq).collect();
    let venue: Vec<i16> = rows.iter().map(|r| r.venue).collect();
    let signal: Vec<&str> = rows.iter().map(|r| r.signal.as_str()).collect();
    let observed: Vec<Option<&str>> = rows.iter().map(|r| r.observed.as_deref()).collect();
    let cmp: Vec<Option<&str>> = rows.iter().map(|r| r.cmp.as_deref()).collect();
    let threshold: Vec<Option<&str>> = rows.iter().map(|r| r.threshold.as_deref()).collect();
    let passed: Vec<bool> = rows.iter().map(|r| r.passed).collect();
    let note: Vec<Option<&str>> = rows.iter().map(|r| r.note.as_deref()).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO rationale (signal_id, seq, venue, signal, observed, cmp, threshold, passed, note)
        SELECT * FROM UNNEST(
            $1::uuid[], $2::int[], $3::smallint[], $4::text[], $5::text[],
            $6::text[], $7::text[], $8::bool[], $9::text[]
        )
        ON CONFLICT (signal_id, seq) DO NOTHING
        "#,
        &signal_id,
        &seq,
        &venue,
        &signal as &[&str],
        &observed as &[Option<&str>],
        &cmp as &[Option<&str>],
        &threshold as &[Option<&str>],
        &passed,
        &note as &[Option<&str>],
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Inserting {} rationale rows", rows.len()))?;

    Ok(result.rows_affected())
}

// A signal and its rationale trail are written together so `/why` never observes a signal with
// an incomplete explanation.
pub async fn insert_signal_with_rationale(
    pool: &PgPool,
    signal: &NewSignal,
    rationale: &[NewRationaleItem],
) -> eyre::Result<()> {
    let mut tx = pool
        .begin()
        .await
        .wrap_err_with(|| "Starting signal insert transaction")?;

    insert_signal(&mut *tx, signal).await?;
    insert_rationale(&mut *tx, rationale).await?;

    tx.commit()
        .await
        .wrap_err_with(|| "Committing signal insert transaction")?;

    Ok(())
}
