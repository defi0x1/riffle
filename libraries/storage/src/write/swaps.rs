use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewSwap {
    pub pool_address: String,
    pub ts: DateTime<Utc>,
    pub slot: i64,
    pub signature: String,
    pub ix_index: i32,
    pub signer: String,
    pub swap_for_y: bool,
    pub amount_in_raw: Decimal,
    pub amount_out_raw: Decimal,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub start_bin_id: i32,
    pub end_bin_id: i32,
    pub start_price: Option<Decimal>,
    pub end_price: Option<Decimal>,
    pub fee_raw: Decimal,
    pub protocol_fee_raw: Decimal,
    pub host_fee_raw: Option<Decimal>,
    pub fee_bps: Decimal,
    pub volume_usd: Option<Decimal>,
    pub trade_fee_usd: Option<Decimal>,
    pub protocol_fee_usd: Option<Decimal>,
}

// Signature + ix_index makes each event immutable and globally unique, so a replayed batch
// (restart, at-least-once redelivery from the stream) is a pure no-op rather than a duplicate
// row -- no locking is needed to make ingestion restart-safe.
//
// One UNNEST per batch rather than one INSERT per row: this is the highest-volume table in the
// schema, and a single round trip for N rows matters at that rate.
pub async fn insert_swaps(pool: &PgPool, rows: &[NewSwap]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let slot: Vec<i64> = rows.iter().map(|r| r.slot).collect();
    let signature: Vec<&str> = rows.iter().map(|r| r.signature.as_str()).collect();
    let ix_index: Vec<i32> = rows.iter().map(|r| r.ix_index).collect();
    let signer: Vec<&str> = rows.iter().map(|r| r.signer.as_str()).collect();
    let swap_for_y: Vec<bool> = rows.iter().map(|r| r.swap_for_y).collect();
    let amount_in_raw: Vec<Decimal> = rows.iter().map(|r| r.amount_in_raw).collect();
    let amount_out_raw: Vec<Decimal> = rows.iter().map(|r| r.amount_out_raw).collect();
    let amount_in: Vec<Decimal> = rows.iter().map(|r| r.amount_in).collect();
    let amount_out: Vec<Decimal> = rows.iter().map(|r| r.amount_out).collect();
    let start_bin_id: Vec<i32> = rows.iter().map(|r| r.start_bin_id).collect();
    let end_bin_id: Vec<i32> = rows.iter().map(|r| r.end_bin_id).collect();
    let start_price: Vec<Option<Decimal>> = rows.iter().map(|r| r.start_price).collect();
    let end_price: Vec<Option<Decimal>> = rows.iter().map(|r| r.end_price).collect();
    let fee_raw: Vec<Decimal> = rows.iter().map(|r| r.fee_raw).collect();
    let protocol_fee_raw: Vec<Decimal> = rows.iter().map(|r| r.protocol_fee_raw).collect();
    let host_fee_raw: Vec<Option<Decimal>> = rows.iter().map(|r| r.host_fee_raw).collect();
    let fee_bps: Vec<Decimal> = rows.iter().map(|r| r.fee_bps).collect();
    let volume_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.volume_usd).collect();
    let trade_fee_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.trade_fee_usd).collect();
    let protocol_fee_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.protocol_fee_usd).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO swaps (
            pool_address, ts, slot, signature, ix_index, signer, swap_for_y,
            amount_in_raw, amount_out_raw, amount_in, amount_out,
            start_bin_id, end_bin_id, start_price, end_price,
            fee_raw, protocol_fee_raw, host_fee_raw, fee_bps,
            volume_usd, trade_fee_usd, protocol_fee_usd
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::timestamptz[], $3::bigint[], $4::text[], $5::int[], $6::text[],
            $7::bool[], $8::numeric[], $9::numeric[], $10::numeric[], $11::numeric[],
            $12::int[], $13::int[], $14::numeric[], $15::numeric[],
            $16::numeric[], $17::numeric[], $18::numeric[], $19::numeric[],
            $20::numeric[], $21::numeric[], $22::numeric[]
        )
        ON CONFLICT (pool_address, ts, signature, ix_index) DO NOTHING
        "#,
        &pool_address as &[&str],
        &ts,
        &slot,
        &signature as &[&str],
        &ix_index,
        &signer as &[&str],
        &swap_for_y,
        &amount_in_raw,
        &amount_out_raw,
        &amount_in,
        &amount_out,
        &start_bin_id,
        &end_bin_id,
        &start_price as &[Option<Decimal>],
        &end_price as &[Option<Decimal>],
        &fee_raw,
        &protocol_fee_raw,
        &host_fee_raw as &[Option<Decimal>],
        &fee_bps,
        &volume_usd as &[Option<Decimal>],
        &trade_fee_usd as &[Option<Decimal>],
        &protocol_fee_usd as &[Option<Decimal>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} swaps", rows.len()))?;

    Ok(result.rows_affected())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;
    use crate::write::pools::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};

    // Fixed, not Utc::now(): ts is part of the primary key, so a deterministic value makes the
    // idempotency assertion below hold across repeated runs against a persistent database, not
    // only within one process.
    fn sample_swap(pool_address: &str, signature: &str) -> NewSwap {
        NewSwap {
            pool_address: pool_address.to_string(),
            ts: "2024-01-01T00:00:00Z".parse().unwrap(),
            slot: 123,
            signature: signature.to_string(),
            ix_index: 0,
            signer: "signer1111111111111111111111111111111111".to_string(),
            swap_for_y: true,
            amount_in_raw: Decimal::new(1_000_000, 0),
            amount_out_raw: Decimal::new(2_000_000, 0),
            amount_in: Decimal::new(1, 0),
            amount_out: Decimal::new(2, 0),
            start_bin_id: 100,
            end_bin_id: 101,
            start_price: Some(Decimal::new(150, 2)),
            end_price: Some(Decimal::new(151, 2)),
            fee_raw: Decimal::new(1_000, 0),
            protocol_fee_raw: Decimal::new(100, 0),
            host_fee_raw: None,
            fee_bps: Decimal::new(30, 2),
            volume_usd: Some(Decimal::new(15, 1)),
            trade_fee_usd: Some(Decimal::new(9, 3)),
            protocol_fee_usd: Some(Decimal::new(1, 3)),
        }
    }

    async fn ensure_pool(pool: &PgPool, pool_address: &str) {
        let now = Utc::now();
        let shared = NewPool {
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
        };
        let params = NewDlmmPoolParams {
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
        };
        upsert_dlmm_pool(pool, &shared, &params).await.unwrap();
    }

    #[tokio::test]
    async fn test_insert_swaps_is_idempotent_on_replay() {
        let pool = test_pool().await;
        let pool_address = "pool_swaps_idempotent";
        ensure_pool(&pool, pool_address).await;
        crate::test_support::reset_pool_fixture(&pool, pool_address).await;

        let rows = vec![sample_swap(pool_address, "sig_swaps_idempotent_1")];

        insert_swaps(&pool, &rows).await.unwrap();
        insert_swaps(&pool, &rows).await.unwrap();

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM swaps WHERE pool_address = $1",
            pool_address
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(count, Some(1));
    }
}
