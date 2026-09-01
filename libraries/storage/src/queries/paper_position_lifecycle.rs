use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct OpenPaperPosition {
    pub id: Uuid,
    pub pool_address: String,
    pub venue: i16,
    pub opened_at: DateTime<Utc>,
    pub entry_price: Option<f64>,
    pub entry_active_bin: Option<i32>,
    pub lower_bin: Option<i32>,
    pub upper_bin: Option<i32>,
    pub size_usd: Option<Decimal>,
    pub size_per_bin: Option<Decimal>,
}

// Marked every 5 minutes for the lifetime of the position, so this is the whole open set on
// every tick -- filtering further (e.g. to a specific pool) would just move the WHERE clause
// into the caller for no benefit.
pub async fn open_paper_positions(pool: &PgPool) -> eyre::Result<Vec<OpenPaperPosition>> {
    let rows = sqlx::query_as!(
        OpenPaperPosition,
        r#"
        SELECT id, pool_address, venue, opened_at, entry_price, entry_active_bin,
               lower_bin, upper_bin, size_usd, size_per_bin
        FROM paper_positions
        WHERE closed_at IS NULL
        ORDER BY opened_at
        "#,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying open paper positions")?;

    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct PositionDueForOutcome {
    pub id: Uuid,
    pub pool_address: String,
    pub venue: i16,
    pub opened_at: DateTime<Utc>,
    pub entry_price: Option<f64>,
    pub size_usd: Option<Decimal>,
    pub predicted: Option<serde_json::Value>,
}

// `cutoff` is `now - horizon`: a position qualifies once it has been open at least that long,
// regardless of whether it has since been closed by an exit trigger -- finalising the horizon
// is about elapsed time, not position lifecycle. NOT EXISTS makes this safe to call every tick.
pub async fn positions_due_for_outcome(
    pool: &PgPool,
    horizon: &str,
    cutoff: DateTime<Utc>,
) -> eyre::Result<Vec<PositionDueForOutcome>> {
    let rows = sqlx::query_as!(
        PositionDueForOutcome,
        r#"
        SELECT pp.id, pp.pool_address, pp.venue, pp.opened_at, pp.entry_price, pp.size_usd, pp.predicted
        FROM paper_positions pp
        WHERE pp.opened_at <= $2
          AND NOT EXISTS (
              SELECT 1 FROM outcomes o WHERE o.position_id = pp.id AND o.horizon = $1
          )
        ORDER BY pp.opened_at
        "#,
        horizon,
        cutoff,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying positions due for {horizon} outcome"))?;

    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct PositionMarkRow {
    pub ts: DateTime<Utc>,
    pub price: Option<f64>,
    pub fees_accrued_usd: Option<Decimal>,
    pub il_usd: Option<Decimal>,
    pub value_usd: Option<Decimal>,
    pub in_range: Option<bool>,
}

pub async fn position_marks_since(
    pool: &PgPool,
    position_id: Uuid,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<PositionMarkRow>> {
    let rows = sqlx::query_as!(
        PositionMarkRow,
        r#"
        SELECT ts, price, fees_accrued_usd, il_usd, value_usd, in_range
        FROM position_marks
        WHERE position_id = $1 AND ts >= $2
        ORDER BY ts
        "#,
        position_id,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying position marks for {position_id}"))?;

    Ok(rows)
}
