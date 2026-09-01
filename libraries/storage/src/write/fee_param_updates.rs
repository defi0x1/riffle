use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewFeeParamUpdate {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub slot: i64,
    pub signature: String,
    pub field: String,
    pub old_value: Option<i64>,
    pub new_value: Option<i64>,
}

// A single transaction can change several fields at once, so the natural key includes `field`
// (see 0006_fee_param_updates.sql) -- one UNNEST row per changed field, not per transaction.
pub async fn insert_fee_param_updates(
    pool: &PgPool,
    rows: &[NewFeeParamUpdate],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let slot: Vec<i64> = rows.iter().map(|r| r.slot).collect();
    let signature: Vec<&str> = rows.iter().map(|r| r.signature.as_str()).collect();
    let field: Vec<&str> = rows.iter().map(|r| r.field.as_str()).collect();
    let old_value: Vec<Option<i64>> = rows.iter().map(|r| r.old_value).collect();
    let new_value: Vec<Option<i64>> = rows.iter().map(|r| r.new_value).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO fee_param_updates (pool_address, ts, slot, signature, field, old_value, new_value)
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bigint[], $4::text[], $5::text[],
            $6::bigint[], $7::bigint[]
        )
        ON CONFLICT (pool_address, ts, signature, field) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &slot,
        &signature as &[&str],
        &field as &[&str],
        &old_value as &[Option<i64>],
        &new_value as &[Option<i64>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} fee param updates", rows.len()))?;

    Ok(result.rows_affected())
}
