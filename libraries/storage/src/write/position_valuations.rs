use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct NewPositionValuation {
    pub position_id: Uuid,
    pub ts: DateTime<Utc>,
    pub price_x_usd: Option<Decimal>,
    pub price_y_usd: Option<Decimal>,
    pub active_bin_id: Option<i32>,
    pub amount_x: Option<Decimal>,
    pub amount_y: Option<Decimal>,
    pub fees_x_uncollected: Option<Decimal>,
    pub fees_y_uncollected: Option<Decimal>,
    pub value_usd: Option<Decimal>,
    pub hold_value_usd: Option<Decimal>,
    pub in_range: Option<bool>,
}

// Mirrors insert_position_marks (write::paper_positions): UNNEST batch insert, ON CONFLICT DO
// NOTHING on the (position_id, ts) primary key so a repeated mark tick for the same timestamp
// never double-counts.
pub async fn insert_position_valuations(
    pool: &PgPool,
    rows: &[NewPositionValuation],
) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let position_id: Vec<Uuid> = rows.iter().map(|r| r.position_id).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let price_x_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.price_x_usd).collect();
    let price_y_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.price_y_usd).collect();
    let active_bin_id: Vec<Option<i32>> = rows.iter().map(|r| r.active_bin_id).collect();
    let amount_x: Vec<Option<Decimal>> = rows.iter().map(|r| r.amount_x).collect();
    let amount_y: Vec<Option<Decimal>> = rows.iter().map(|r| r.amount_y).collect();
    let fees_x_uncollected: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.fees_x_uncollected).collect();
    let fees_y_uncollected: Vec<Option<Decimal>> =
        rows.iter().map(|r| r.fees_y_uncollected).collect();
    let value_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.value_usd).collect();
    let hold_value_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.hold_value_usd).collect();
    let in_range: Vec<Option<bool>> = rows.iter().map(|r| r.in_range).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO position_valuations (
            position_id, ts, price_x_usd, price_y_usd, active_bin_id, amount_x, amount_y,
            fees_x_uncollected, fees_y_uncollected, value_usd, hold_value_usd, in_range
        )
        SELECT * FROM UNNEST(
            $1::uuid[], $2::timestamptz[], $3::numeric[], $4::numeric[], $5::int[],
            $6::numeric[], $7::numeric[], $8::numeric[], $9::numeric[], $10::numeric[],
            $11::numeric[], $12::bool[]
        )
        ON CONFLICT (position_id, ts) DO NOTHING
        "#,
        &position_id,
        &ts,
        &price_x_usd as &[Option<Decimal>],
        &price_y_usd as &[Option<Decimal>],
        &active_bin_id as &[Option<i32>],
        &amount_x as &[Option<Decimal>],
        &amount_y as &[Option<Decimal>],
        &fees_x_uncollected as &[Option<Decimal>],
        &fees_y_uncollected as &[Option<Decimal>],
        &value_usd as &[Option<Decimal>],
        &hold_value_usd as &[Option<Decimal>],
        &in_range as &[Option<bool>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} position valuations", rows.len()))?;

    Ok(result.rows_affected())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::{ensure_pool_fixture, reset_wallet_fixture, test_pool};
    use crate::write::{NewWallet, register_wallet};

    async fn ensure_position(pool: &PgPool, wallet: &str, pool_address: &str) -> Uuid {
        register_wallet(
            pool,
            &NewWallet {
                pubkey: wallet.to_string(),
                telegram_user_id: 900,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();
        ensure_pool_fixture(pool, pool_address).await;

        let position_id = Uuid::new_v4();
        sqlx::query!(
            r#"
            INSERT INTO positions (
                id, position_address, wallet_address, pool_address, venue, opened_at,
                entry_active_bin, lower_bin, upper_bin
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            position_id,
            format!("position_addr_{position_id}"),
            wallet,
            pool_address,
            crate::types::venue::DLMM,
            Utc::now(),
            100,
            90,
            110,
        )
        .execute(pool)
        .await
        .unwrap();

        position_id
    }

    #[tokio::test]
    async fn test_insert_position_valuations_is_idempotent() {
        let pool = test_pool().await;
        let wallet = "wallet_valuation_idempotent_11111111111111";
        let pool_address = "pool_valuation_idempotent";
        reset_wallet_fixture(&pool, wallet).await;
        let position_id = ensure_position(&pool, wallet, pool_address).await;

        let ts: DateTime<Utc> = "2024-01-01T00:00:00Z".parse().unwrap();
        let rows = vec![NewPositionValuation {
            position_id,
            ts,
            price_x_usd: Some(Decimal::new(150, 2)),
            price_y_usd: Some(Decimal::new(1, 0)),
            active_bin_id: Some(100),
            amount_x: Some(Decimal::new(1, 0)),
            amount_y: Some(Decimal::new(2, 0)),
            fees_x_uncollected: Some(Decimal::new(1, 3)),
            fees_y_uncollected: Some(Decimal::new(2, 3)),
            value_usd: Some(Decimal::new(35, 1)),
            hold_value_usd: Some(Decimal::new(36, 1)),
            in_range: Some(true),
        }];

        insert_position_valuations(&pool, &rows).await.unwrap();
        insert_position_valuations(&pool, &rows).await.unwrap();

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM position_valuations WHERE position_id = $1",
            position_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, Some(1));
    }
}
