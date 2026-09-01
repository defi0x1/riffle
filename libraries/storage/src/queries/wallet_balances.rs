use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct WalletBalanceRow {
    pub mint: String,
    pub amount: Decimal,
    pub price_usd: Option<Decimal>,
    pub value_usd: Option<Decimal>,
}

// "What do I hold before I act" -- the latest refreshed balance per mint, the same DISTINCT ON
// (... ORDER BY ..., ts DESC) shape queries::reconciliation uses for the latest pool_snapshots
// row per pool.
pub async fn latest_wallet_balances(
    pool: &PgPool,
    wallet_address: &str,
) -> eyre::Result<Vec<WalletBalanceRow>> {
    let rows = sqlx::query_as!(
        WalletBalanceRow,
        r#"
        SELECT DISTINCT ON (mint) mint, amount, price_usd, value_usd
        FROM wallet_balances
        WHERE wallet_address = $1
        ORDER BY mint, ts DESC
        "#,
        wallet_address,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying latest balances for wallet {wallet_address}"))?;

    Ok(rows)
}
