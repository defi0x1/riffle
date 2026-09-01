use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct WalletRow {
    pub pubkey: String,
    pub label: Option<String>,
    pub registered_at: DateTime<Utc>,
}

// A Telegram user's active wallets -- the Mini App's wallet picker. Revoked wallets are
// excluded, matching idx_wallets_telegram_user.
pub async fn active_wallets_for_user(
    pool: &PgPool,
    telegram_user_id: i64,
) -> eyre::Result<Vec<WalletRow>> {
    let rows = sqlx::query_as!(
        WalletRow,
        r#"
        SELECT pubkey, label, registered_at
        FROM wallets
        WHERE telegram_user_id = $1 AND revoked_at IS NULL
        ORDER BY registered_at
        "#,
        telegram_user_id,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying active wallets for Telegram user {telegram_user_id}"))?;

    Ok(rows)
}
