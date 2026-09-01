use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::{PgExecutor, PgPool};

#[derive(Clone, Debug)]
pub struct NewPoolSnapshot {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub slot: i64,
    pub price: f64,
    pub reserve_x_raw: Option<Decimal>,
    pub reserve_y_raw: Option<Decimal>,
    pub tvl_usd: Option<Decimal>,
    pub active_tvl_usd: Option<Decimal>,
    pub total_fee_bps: Decimal,
}

pub async fn insert_pool_snapshots<'e, E: PgExecutor<'e>>(
    executor: E,
    rows: &[NewPoolSnapshot],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let slot: Vec<i64> = rows.iter().map(|r| r.slot).collect();
    let price: Vec<f64> = rows.iter().map(|r| r.price).collect();
    let reserve_x_raw: Vec<Option<Decimal>> = rows.iter().map(|r| r.reserve_x_raw).collect();
    let reserve_y_raw: Vec<Option<Decimal>> = rows.iter().map(|r| r.reserve_y_raw).collect();
    let tvl_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.tvl_usd).collect();
    let active_tvl_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.active_tvl_usd).collect();
    let total_fee_bps: Vec<Decimal> = rows.iter().map(|r| r.total_fee_bps).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO pool_snapshots (
            pool_address, ts, slot, price, reserve_x_raw, reserve_y_raw,
            tvl_usd, active_tvl_usd, total_fee_bps
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bigint[], $4::float8[],
            $5::numeric[], $6::numeric[], $7::numeric[], $8::numeric[], $9::numeric[]
        )
        ON CONFLICT (pool_address, ts) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &slot,
        &price,
        &reserve_x_raw as &[Option<Decimal>],
        &reserve_y_raw as &[Option<Decimal>],
        &tvl_usd as &[Option<Decimal>],
        &active_tvl_usd as &[Option<Decimal>],
        &total_fee_bps,
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Inserting {} pool snapshots", rows.len()))?;

    Ok(result.rows_affected())
}

#[derive(Clone, Debug)]
pub struct NewDlmmPoolState {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub active_bin_id: i32,
    pub volatility_accumulator: i32,
    pub volatility_reference: i32,
    pub index_reference: i32,
    // Unix timestamp from the on-chain clock, not wall clock: the decay computation this
    // feeds is defined against the clock the program itself used.
    pub last_update_timestamp: i64,
    pub base_fee_bps: Decimal,
    pub dynamic_fee_bps: Decimal,
}

pub async fn insert_dlmm_pool_states<'e, E: PgExecutor<'e>>(
    executor: E,
    rows: &[NewDlmmPoolState],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let active_bin_id: Vec<i32> = rows.iter().map(|r| r.active_bin_id).collect();
    let volatility_accumulator: Vec<i32> = rows.iter().map(|r| r.volatility_accumulator).collect();
    let volatility_reference: Vec<i32> = rows.iter().map(|r| r.volatility_reference).collect();
    let index_reference: Vec<i32> = rows.iter().map(|r| r.index_reference).collect();
    let last_update_timestamp: Vec<i64> = rows.iter().map(|r| r.last_update_timestamp).collect();
    let base_fee_bps: Vec<Decimal> = rows.iter().map(|r| r.base_fee_bps).collect();
    let dynamic_fee_bps: Vec<Decimal> = rows.iter().map(|r| r.dynamic_fee_bps).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO dlmm_pool_state (
            pool_address, ts, active_bin_id, volatility_accumulator, volatility_reference,
            index_reference, last_update_timestamp, base_fee_bps, dynamic_fee_bps
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::int[], $4::int[], $5::int[],
            $6::int[], $7::bigint[], $8::numeric[], $9::numeric[]
        )
        ON CONFLICT (pool_address, ts) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &active_bin_id,
        &volatility_accumulator,
        &volatility_reference,
        &index_reference,
        &last_update_timestamp,
        &base_fee_bps,
        &dynamic_fee_bps,
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Inserting {} dlmm pool states", rows.len()))?;

    Ok(result.rows_affected())
}

// The shared/satellite pair (0009_pool_snapshots.sql) written together so a reader never sees
// one half without the other.
pub async fn insert_pool_state(
    pool: &PgPool,
    snapshots: &[NewPoolSnapshot],
    dlmm_states: &[NewDlmmPoolState],
) -> eyre::Result<()> {
    let mut tx = pool
        .begin()
        .await
        .wrap_err_with(|| "Starting pool state insert transaction")?;

    if !snapshots.is_empty() {
        insert_pool_snapshots(&mut *tx, snapshots).await?;
    }
    if !dlmm_states.is_empty() {
        insert_dlmm_pool_states(&mut *tx, dlmm_states).await?;
    }

    tx.commit()
        .await
        .wrap_err_with(|| "Committing pool state insert transaction")?;

    Ok(())
}
