use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use eyre::WrapErr;

#[derive(Clone, Debug)]
pub struct PositionRow {
    pub id: Uuid,
    pub position_address: String,
    pub wallet_address: String,
    pub pool_address: String,
    pub venue: i16,
    pub opened_at: DateTime<Utc>,
    pub entry_active_bin: Option<i32>,
    pub lower_bin: i32,
    pub upper_bin: i32,
    pub closed_at: Option<DateTime<Utc>>,
    pub close_reason: Option<String>,
}

// "What am I holding right now" for one wallet -- the Mini App's home screen.
pub async fn open_positions_for_wallet(
    pool: &PgPool,
    wallet_address: &str,
) -> eyre::Result<Vec<PositionRow>> {
    let rows = sqlx::query_as!(
        PositionRow,
        r#"
        SELECT id, position_address, wallet_address, pool_address, venue, opened_at,
               entry_active_bin, lower_bin, upper_bin, closed_at, close_reason
        FROM positions
        WHERE wallet_address = $1 AND closed_at IS NULL
        ORDER BY opened_at DESC
        "#,
        wallet_address,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying open positions for wallet {wallet_address}"))?;

    Ok(rows)
}

pub async fn position_by_address(
    pool: &PgPool,
    position_address: &str,
) -> eyre::Result<Option<PositionRow>> {
    let row = sqlx::query_as!(
        PositionRow,
        r#"
        SELECT id, position_address, wallet_address, pool_address, venue, opened_at,
               entry_active_bin, lower_bin, upper_bin, closed_at, close_reason
        FROM positions
        WHERE position_address = $1
        "#,
        position_address,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Querying position {position_address}"))?;

    Ok(row)
}
