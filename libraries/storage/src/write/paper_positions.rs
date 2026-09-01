use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct NewPaperPosition {
    pub id: Uuid,
    pub signal_id: Option<Uuid>,
    pub pool_address: String,
    pub venue: i16,
    pub opened_at: DateTime<Utc>,
    pub regime: Option<String>,
    pub entry_price: Option<f64>,
    pub entry_active_bin: Option<i32>,
    pub lower_bin: Option<i32>,
    pub upper_bin: Option<i32>,
    pub shape: Option<String>,
    pub size_usd: Option<Decimal>,
    pub size_per_bin: Option<Decimal>,
    pub predicted: Option<serde_json::Value>,
}

pub async fn open_paper_position(pool: &PgPool, row: &NewPaperPosition) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO paper_positions (
            id, signal_id, pool_address, venue, opened_at, regime, entry_price,
            entry_active_bin, lower_bin, upper_bin, shape, size_usd, size_per_bin, predicted
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        "#,
        row.id,
        row.signal_id,
        row.pool_address,
        row.venue,
        row.opened_at,
        row.regime,
        row.entry_price,
        row.entry_active_bin,
        row.lower_bin,
        row.upper_bin,
        row.shape,
        row.size_usd,
        row.size_per_bin,
        row.predicted,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Opening paper position {}", row.id))?;

    Ok(())
}

// Idempotent on repeated calls with the same reason: closing an already-closed position again
// leaves closed_at at its original value rather than overwriting it with a later retry's clock.
pub async fn close_paper_position(
    pool: &PgPool,
    id: Uuid,
    closed_at: DateTime<Utc>,
    close_reason: &str,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        UPDATE paper_positions
        SET closed_at = COALESCE(closed_at, $2), close_reason = COALESCE(close_reason, $3)
        WHERE id = $1
        "#,
        id,
        closed_at,
        close_reason,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Closing paper position {id}"))?;

    Ok(())
}

#[derive(Clone, Debug)]
pub struct NewPositionMark {
    pub position_id: Uuid,
    pub ts: DateTime<Utc>,
    pub price: Option<f64>,
    pub active_bin_id: Option<i32>,
    pub fees_accrued_usd: Option<Decimal>,
    pub il_usd: Option<Decimal>,
    pub value_usd: Option<Decimal>,
    pub in_range: Option<bool>,
}

pub async fn insert_position_marks(pool: &PgPool, rows: &[NewPositionMark]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let position_id: Vec<Uuid> = rows.iter().map(|r| r.position_id).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let price: Vec<Option<f64>> = rows.iter().map(|r| r.price).collect();
    let active_bin_id: Vec<Option<i32>> = rows.iter().map(|r| r.active_bin_id).collect();
    let fees_accrued_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.fees_accrued_usd).collect();
    let il_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.il_usd).collect();
    let value_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.value_usd).collect();
    let in_range: Vec<Option<bool>> = rows.iter().map(|r| r.in_range).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO position_marks (position_id, ts, price, active_bin_id, fees_accrued_usd, il_usd, value_usd, in_range)
        SELECT * FROM UNNEST(
            $1::uuid[], $2::timestamptz[], $3::float8[], $4::int[],
            $5::numeric[], $6::numeric[], $7::numeric[], $8::bool[]
        )
        ON CONFLICT (position_id, ts) DO NOTHING
        "#,
        &position_id,
        &ts,
        &price as &[Option<f64>],
        &active_bin_id as &[Option<i32>],
        &fees_accrued_usd as &[Option<Decimal>],
        &il_usd as &[Option<Decimal>],
        &value_usd as &[Option<Decimal>],
        &in_range as &[Option<bool>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} position marks", rows.len()))?;

    Ok(result.rows_affected())
}

#[derive(Clone, Debug)]
pub struct NewOutcome {
    pub position_id: Uuid,
    pub horizon: String,
    pub venue: i16,
    pub finalized_at: DateTime<Utc>,
    pub fees_real: Option<Decimal>,
    pub fees_predicted: Option<Decimal>,
    pub lvr_real: Option<Decimal>,
    pub r_real: Option<f64>,
    pub r_predicted: Option<f64>,
    pub time_in_range: Option<f64>,
    pub hit: Option<bool>,
}

pub async fn upsert_outcome(pool: &PgPool, row: &NewOutcome) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO outcomes (
            position_id, horizon, venue, finalized_at, fees_real, fees_predicted,
            lvr_real, r_real, r_predicted, time_in_range, hit
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (position_id, horizon) DO UPDATE SET
            venue          = EXCLUDED.venue,
            finalized_at   = EXCLUDED.finalized_at,
            fees_real      = EXCLUDED.fees_real,
            fees_predicted = EXCLUDED.fees_predicted,
            lvr_real       = EXCLUDED.lvr_real,
            r_real         = EXCLUDED.r_real,
            r_predicted    = EXCLUDED.r_predicted,
            time_in_range  = EXCLUDED.time_in_range,
            hit            = EXCLUDED.hit
        "#,
        row.position_id,
        row.horizon,
        row.venue,
        row.finalized_at,
        row.fees_real,
        row.fees_predicted,
        row.lvr_real,
        row.r_real,
        row.r_predicted,
        row.time_in_range,
        row.hit,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting outcome {}/{}", row.position_id, row.horizon))?;

    Ok(())
}
