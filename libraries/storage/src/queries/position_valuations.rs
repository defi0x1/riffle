use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PositionValuationRow {
    pub ts: DateTime<Utc>,
    pub price_x_usd: Option<Decimal>,
    pub price_y_usd: Option<Decimal>,
    pub amount_x: Option<Decimal>,
    pub amount_y: Option<Decimal>,
    pub fees_x_uncollected: Option<Decimal>,
    pub fees_y_uncollected: Option<Decimal>,
    pub value_usd: Option<Decimal>,
    pub hold_value_usd: Option<Decimal>,
    pub in_range: Option<bool>,
}

// "What is this position worth right now" -- the latest mark, if one has ever been taken.
pub async fn latest_position_valuation(
    pool: &PgPool,
    position_id: Uuid,
) -> eyre::Result<Option<PositionValuationRow>> {
    let row = sqlx::query_as!(
        PositionValuationRow,
        r#"
        SELECT ts, price_x_usd, price_y_usd, amount_x, amount_y, fees_x_uncollected,
               fees_y_uncollected, value_usd, hold_value_usd, in_range
        FROM position_valuations
        WHERE position_id = $1
        ORDER BY ts DESC
        LIMIT 1
        "#,
        position_id,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Querying latest valuation for position {position_id}"))?;

    Ok(row)
}

pub async fn position_valuations_since(
    pool: &PgPool,
    position_id: Uuid,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<PositionValuationRow>> {
    let rows = sqlx::query_as!(
        PositionValuationRow,
        r#"
        SELECT ts, price_x_usd, price_y_usd, amount_x, amount_y, fees_x_uncollected,
               fees_y_uncollected, value_usd, hold_value_usd, in_range
        FROM position_valuations
        WHERE position_id = $1 AND ts >= $2
        ORDER BY ts
        "#,
        position_id,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying valuations for position {position_id}"))?;

    Ok(rows)
}
