use eyre::WrapErr;
use sqlx::PgPool;

use crate::write::TransactionIntentRow;

// Not-yet-settled intents for a wallet -- what the Mini App resumes on relaunch instead of
// asking the user to start the action over (and, per 0030, instead of risking a second
// submission of the same action).
pub async fn pending_intents_for_wallet(
    pool: &PgPool,
    wallet_address: &str,
) -> eyre::Result<Vec<TransactionIntentRow>> {
    let rows = sqlx::query_as!(
        TransactionIntentRow,
        r#"
        SELECT id, wallet_address, position_id, pool_address, venue, action, idempotency_key,
               status, unsigned_tx_base64, params, created_at, expires_at, signature
        FROM transaction_intents
        WHERE wallet_address = $1 AND status IN (0, 1)
        ORDER BY created_at DESC
        "#,
        wallet_address,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying pending transaction intents for {wallet_address}"))?;

    Ok(rows)
}

pub async fn intent_by_signature(
    pool: &PgPool,
    signature: &str,
) -> eyre::Result<Option<TransactionIntentRow>> {
    let row = sqlx::query_as!(
        TransactionIntentRow,
        r#"
        SELECT id, wallet_address, position_id, pool_address, venue, action, idempotency_key,
               status, unsigned_tx_base64, params, created_at, expires_at, signature
        FROM transaction_intents
        WHERE signature = $1
        "#,
        signature,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Querying transaction intent by signature {signature}"))?;

    Ok(row)
}
