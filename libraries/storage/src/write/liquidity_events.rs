use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewLiquidityEvent {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub slot: i64,
    pub signature: String,
    pub ix_index: i32,
    pub position_address: Option<String>,
    pub owner: String,
    // 0 = add, 1 = remove; see types::liquidity_action.
    pub action: i16,
    pub active_bin_id: i32,
    pub amount_x_raw: Option<Decimal>,
    pub amount_y_raw: Option<Decimal>,
    pub amount_usd: Option<Decimal>,
}

pub async fn insert_liquidity_events(
    pool: &PgPool,
    rows: &[NewLiquidityEvent],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let slot: Vec<i64> = rows.iter().map(|r| r.slot).collect();
    let signature: Vec<&str> = rows.iter().map(|r| r.signature.as_str()).collect();
    let ix_index: Vec<i32> = rows.iter().map(|r| r.ix_index).collect();
    let position_address: Vec<Option<&str>> =
        rows.iter().map(|r| r.position_address.as_deref()).collect();
    let owner: Vec<&str> = rows.iter().map(|r| r.owner.as_str()).collect();
    let action: Vec<i16> = rows.iter().map(|r| r.action).collect();
    let active_bin_id: Vec<i32> = rows.iter().map(|r| r.active_bin_id).collect();
    let amount_x_raw: Vec<Option<Decimal>> = rows.iter().map(|r| r.amount_x_raw).collect();
    let amount_y_raw: Vec<Option<Decimal>> = rows.iter().map(|r| r.amount_y_raw).collect();
    let amount_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.amount_usd).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO liquidity_events (
            pool_address, ts, slot, signature, ix_index, position_address, owner,
            action, active_bin_id, amount_x_raw, amount_y_raw, amount_usd
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bigint[], $4::text[], $5::int[], $6::text[],
            $7::text[], $8::smallint[], $9::int[], $10::numeric[], $11::numeric[], $12::numeric[]
        )
        ON CONFLICT (pool_address, ts, signature, ix_index) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &slot,
        &signature as &[&str],
        &ix_index,
        &position_address as &[Option<&str>],
        &owner as &[&str],
        &action,
        &active_bin_id,
        &amount_x_raw as &[Option<Decimal>],
        &amount_y_raw as &[Option<Decimal>],
        &amount_usd as &[Option<Decimal>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} liquidity events", rows.len()))?;

    Ok(result.rows_affected())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use crate::write::pools::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};

    #[tokio::test]
    async fn test_insert_liquidity_events_is_idempotent() {
        let pool = test_pool().await;
        let pool_address = "pool_liq_events_idempotent";
        let now = Utc::now();

        upsert_dlmm_pool(
            &pool,
            &NewPool {
                pool_address: pool_address.to_string(),
                venue: crate::types::venue::DLMM,
                token_x: "So11111111111111111111111111111111111111112".to_string(),
                token_y: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                base_fee_bps: Decimal::new(100, 2),
                protocol_share_bps: 500,
                tvl_usd: None,
                status: 0,
                creator: None,
                activation_point: None,
                created_at: now,
                first_liquidity_at: None,
                is_blacklisted: false,
                launchpad: None,
                tags: vec![],
                updated_at: now,
            },
            &NewDlmmPoolParams {
                pool_address: pool_address.to_string(),
                bin_step: 20,
                base_factor: 10_000,
                filter_period: 30,
                decay_period: 600,
                reduction_factor: 5_000,
                variable_fee_control: 40_000,
                max_volatility_accumulator: 350_000,
                collect_fee_mode: 0,
                reward_mint_x: None,
                reward_mint_y: None,
            },
        )
        .await
        .unwrap();

        let rows = vec![NewLiquidityEvent {
            pool_address: pool_address.to_string(),
            ts: now,
            slot: 42,
            signature: "sig_liq_events_idempotent".to_string(),
            ix_index: 0,
            position_address: Some("position111111111111111111111111111111111".to_string()),
            owner: "owner1111111111111111111111111111111111111".to_string(),
            action: crate::types::liquidity_action::ADD,
            active_bin_id: 100,
            amount_x_raw: Some(Decimal::new(1_000_000, 0)),
            amount_y_raw: Some(Decimal::new(2_000_000, 0)),
            amount_usd: Some(Decimal::new(15, 1)),
        }];

        insert_liquidity_events(&pool, &rows).await.unwrap();
        insert_liquidity_events(&pool, &rows).await.unwrap();

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM liquidity_events WHERE pool_address = $1",
            pool_address
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(count, Some(1));
    }
}
