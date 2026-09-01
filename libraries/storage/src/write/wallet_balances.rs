use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewWalletBalance {
    pub wallet_address: String,
    pub mint: String,
    pub ts: DateTime<Utc>,
    pub amount_raw: Decimal,
    pub amount: Decimal,
    pub price_usd: Option<Decimal>,
    pub value_usd: Option<Decimal>,
}

// Same batch-insert / ON CONFLICT DO NOTHING shape as insert_liquidity_events: a repeated
// balance-refresh tick for a wallet/mint/timestamp already recorded is a no-op, not a duplicate
// row.
pub async fn insert_wallet_balances(pool: &PgPool, rows: &[NewWalletBalance]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let wallet_address: Vec<&str> = rows.iter().map(|r| r.wallet_address.as_str()).collect();
    let mint: Vec<&str> = rows.iter().map(|r| r.mint.as_str()).collect();
    let ts: Vec<DateTime<Utc>> = rows.iter().map(|r| r.ts).collect();
    let amount_raw: Vec<Decimal> = rows.iter().map(|r| r.amount_raw).collect();
    let amount: Vec<Decimal> = rows.iter().map(|r| r.amount).collect();
    let price_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.price_usd).collect();
    let value_usd: Vec<Option<Decimal>> = rows.iter().map(|r| r.value_usd).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO wallet_balances (wallet_address, mint, ts, amount_raw, amount, price_usd, value_usd)
        SELECT * FROM UNNEST(
            $1::text[], $2::text[], $3::timestamptz[], $4::numeric[], $5::numeric[],
            $6::numeric[], $7::numeric[]
        )
        ON CONFLICT (wallet_address, mint, ts) DO NOTHING
        "#,
        &wallet_address as &[&str],
        &mint as &[&str],
        &ts,
        &amount_raw,
        &amount,
        &price_usd as &[Option<Decimal>],
        &value_usd as &[Option<Decimal>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Inserting {} wallet balances", rows.len()))?;

    Ok(result.rows_affected())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::{reset_wallet_fixture, test_pool};
    use crate::write::{NewWallet, register_wallet};

    #[tokio::test]
    async fn test_insert_wallet_balances_is_idempotent() {
        let pool = test_pool().await;
        let wallet = "wallet_balance_idempotent_1111111111111111";
        reset_wallet_fixture(&pool, wallet).await;

        register_wallet(
            &pool,
            &NewWallet {
                pubkey: wallet.to_string(),
                telegram_user_id: 901,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        // Fixed, not `now`: ts is part of the primary key, so a deterministic value keeps the
        // idempotency assertion valid across repeated runs against a persistent database.
        let ts: DateTime<Utc> = "2024-01-01T00:00:00Z".parse().unwrap();
        let rows = vec![NewWalletBalance {
            wallet_address: wallet.to_string(),
            mint: "So11111111111111111111111111111111111111112".to_string(),
            ts,
            amount_raw: Decimal::new(1_500_000_000, 0),
            amount: Decimal::new(15, 1),
            price_usd: Some(Decimal::new(150, 2)),
            value_usd: Some(Decimal::new(225, 2)),
        }];

        insert_wallet_balances(&pool, &rows).await.unwrap();
        insert_wallet_balances(&pool, &rows).await.unwrap();

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM wallet_balances WHERE wallet_address = $1",
            wallet
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, Some(1));
    }
}
