use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewActiveBinSnapshot {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub slot: i64,
    pub bin_id: i32,
    pub amount_x: Decimal,
    pub amount_y: Decimal,
    pub liquidity_supply: Decimal,
    pub quote_value_usd: Option<Decimal>,
}

// High-frequency, single-bin table. Primary key is (pool_address, ts); a replayed poll at the
// same timestamp is a no-op.
pub async fn insert_active_bin_snapshots(
    pool: &PgPool,
    rows: &[NewActiveBinSnapshot],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let slot: Vec<i64> = rows.iter().map(|r| r.slot).collect();
    let bin_id: Vec<i32> = rows.iter().map(|r| r.bin_id).collect();
    let amount_x: Vec<Decimal> = rows.iter().map(|r| r.amount_x).collect();
    let amount_y: Vec<Decimal> = rows.iter().map(|r| r.amount_y).collect();
    let liquidity_supply: Vec<Decimal> = rows.iter().map(|r| r.liquidity_supply).collect();
    let quote_value_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.quote_value_usd).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO active_bin_snapshots (
            pool_address, ts, slot, bin_id, amount_x, amount_y, liquidity_supply, quote_value_usd
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bigint[], $4::int[],
            $5::numeric[], $6::numeric[], $7::numeric[], $8::numeric[]
        )
        ON CONFLICT (pool_address, ts) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &slot,
        &bin_id,
        &amount_x,
        &amount_y,
        &liquidity_supply,
        &quote_value_usd as &[Option<Decimal>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} active bin snapshots", rows.len()))?;

    Ok(result.rows_affected())
}

#[derive(Clone, Debug)]
pub struct NewBinState {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub slot: i64,
    pub bin_id: i32,
    pub amount_x: Decimal,
    pub amount_y: Decimal,
    pub liquidity_supply: Decimal,
    pub price_q64: Decimal,
    pub ui_price: f64,
    pub fee_x_per_token_stored: Decimal,
    pub fee_y_per_token_stored: Decimal,
}

// Full-distribution table at 5-minute cadence. Change detection (skip a bin whose state is
// unchanged since the last write) happens in the caller, not here -- this function just needs to
// be safe to call twice with the same rows.
pub async fn insert_bin_states(pool: &PgPool, rows: &[NewBinState]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let slot: Vec<i64> = rows.iter().map(|r| r.slot).collect();
    let bin_id: Vec<i32> = rows.iter().map(|r| r.bin_id).collect();
    let amount_x: Vec<Decimal> = rows.iter().map(|r| r.amount_x).collect();
    let amount_y: Vec<Decimal> = rows.iter().map(|r| r.amount_y).collect();
    let liquidity_supply: Vec<Decimal> = rows.iter().map(|r| r.liquidity_supply).collect();
    let price_q64: Vec<Decimal> = rows.iter().map(|r| r.price_q64).collect();
    let ui_price: Vec<f64> = rows.iter().map(|r| r.ui_price).collect();
    let fee_x_per_token_stored: Vec<Decimal> =
        rows.iter().map(|r| r.fee_x_per_token_stored).collect();
    let fee_y_per_token_stored: Vec<Decimal> =
        rows.iter().map(|r| r.fee_y_per_token_stored).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO bin_states (
            pool_address, ts, slot, bin_id, amount_x, amount_y, liquidity_supply,
            price_q64, ui_price, fee_x_per_token_stored, fee_y_per_token_stored
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bigint[], $4::int[],
            $5::numeric[], $6::numeric[], $7::numeric[], $8::numeric[], $9::float8[],
            $10::numeric[], $11::numeric[]
        )
        ON CONFLICT (pool_address, bin_id, ts) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &slot,
        &bin_id,
        &amount_x,
        &amount_y,
        &liquidity_supply,
        &price_q64,
        &ui_price,
        &fee_x_per_token_stored,
        &fee_y_per_token_stored,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} bin states", rows.len()))?;

    Ok(result.rows_affected())
}
