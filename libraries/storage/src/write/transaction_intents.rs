use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::intent_status;

#[derive(Clone, Debug)]
pub struct NewTransactionIntent {
    pub id: Uuid,
    pub wallet_address: String,
    // NULL for `open`; required for add/remove/claim/close (see 0030).
    pub position_id: Option<Uuid>,
    pub pool_address: String,
    pub venue: i16,
    pub action: i16,
    pub idempotency_key: String,
    pub unsigned_tx_base64: String,
    pub params: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct TransactionIntentRow {
    pub id: Uuid,
    pub wallet_address: String,
    pub position_id: Option<Uuid>,
    pub pool_address: String,
    pub venue: i16,
    pub action: i16,
    pub idempotency_key: String,
    pub status: i16,
    pub unsigned_tx_base64: String,
    pub params: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub signature: Option<String>,
}

// The idempotency half of the double-submission guarantee described in 0030: a retried "build
// me a transaction for this action" resolves to the same row via (wallet_address,
// idempotency_key) instead of minting a second intent. The DO UPDATE is a deliberate no-op
// (every column re-set to its own current value) purely so RETURNING can hand back the existing
// row on conflict -- ON CONFLICT DO NOTHING cannot RETURNING the row it declined to touch.
pub async fn create_transaction_intent(
    pool: &PgPool,
    row: &NewTransactionIntent,
) -> eyre::Result<TransactionIntentRow> {
    let result = sqlx::query_as!(
        TransactionIntentRow,
        r#"
        INSERT INTO transaction_intents (
            id, wallet_address, position_id, pool_address, venue, action, idempotency_key,
            unsigned_tx_base64, params, created_at, expires_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $10)
        ON CONFLICT (wallet_address, idempotency_key) DO UPDATE SET
            wallet_address = transaction_intents.wallet_address
        RETURNING id, wallet_address, position_id, pool_address, venue, action, idempotency_key,
                  status, unsigned_tx_base64, params, created_at, expires_at, signature
        "#,
        row.id,
        row.wallet_address,
        row.position_id,
        row.pool_address,
        row.venue,
        row.action,
        row.idempotency_key,
        row.unsigned_tx_base64,
        row.params,
        row.created_at,
        row.expires_at,
    )
    .fetch_one(pool)
    .await
    .wrap_err_with(|| format!("Creating transaction intent {}", row.id))?;

    Ok(result)
}

// Guarded to CREATED/SUBMITTED so a resumed flow replaying the same submission never touches an
// intent that has already reached a terminal state, and COALESCE so replaying it with the same
// signature is a harmless no-op. A genuinely different signature landing on this id would still
// be rejected by idx_transaction_intents_signature if it were ever attached to another intent --
// this function only ever narrows this one row.
pub async fn mark_intent_submitted(
    pool: &PgPool,
    id: Uuid,
    signature: &str,
    submitted_at: DateTime<Utc>,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        UPDATE transaction_intents
        SET status = $3,
            signature = COALESCE(signature, $2),
            submitted_at = COALESCE(submitted_at, $4),
            updated_at = $4
        WHERE id = $1 AND status IN ($5, $3)
        "#,
        id,
        signature,
        intent_status::SUBMITTED,
        submitted_at,
        intent_status::CREATED,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Marking transaction intent {id} submitted"))?;

    Ok(())
}

#[derive(Clone, Debug)]
pub struct ConfirmedCashFlow {
    pub kind: i16,
    pub ts: DateTime<Utc>,
    pub amount_x_raw: Option<Decimal>,
    pub amount_y_raw: Option<Decimal>,
    pub amount_x: Option<Decimal>,
    pub amount_y: Option<Decimal>,
    pub price_x_usd: Option<Decimal>,
    pub price_y_usd: Option<Decimal>,
    pub value_usd: Option<Decimal>,
    pub bin_liquidity: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct ConfirmTransactionIntent {
    pub intent_id: Uuid,
    pub confirmed_at: DateTime<Utc>,
    pub slot: i64,
    // `open` only: the on-chain position this intent created. Every other action's position
    // already exists and is read back from the intent row itself.
    pub position_address: Option<String>,
    pub entry_active_bin: Option<i32>,
    pub lower_bin: Option<i32>,
    pub upper_bin: Option<i32>,
    pub cash_flow: ConfirmedCashFlow,
    // `close` only.
    pub close_reason: Option<String>,
}

// The confirmation half of the intent lifecycle. Transactional and idempotent end to end:
//   - an intent already CONFIRMED short-circuits and just returns its known position_id, so
//     replaying a confirmation after a crash mid-flow does nothing on the second call;
//   - the `open` position insert upserts on position_address (its on-chain identity) so a
//     replayed `open` confirmation resolves to the same position row instead of a duplicate;
//   - the cash flow insert is keyed on transaction_intent_id, so a replay cannot double-count
//     the deposit/withdrawal/claim;
//   - closing a position COALESCEs closed_at/close_reason, matching close_paper_position.
// Everything happens in one transaction so a crash between steps never leaves an intent
// CONFIRMED without its position or cash flow row, or vice versa.
pub async fn confirm_transaction_intent(
    pool: &PgPool,
    input: &ConfirmTransactionIntent,
) -> eyre::Result<Uuid> {
    let mut tx = pool
        .begin()
        .await
        .wrap_err_with(|| "Starting transaction intent confirmation")?;

    let intent = sqlx::query!(
        r#"
        SELECT action, wallet_address, pool_address, venue, position_id, status
        FROM transaction_intents WHERE id = $1 FOR UPDATE
        "#,
        input.intent_id,
    )
    .fetch_one(&mut *tx)
    .await
    .wrap_err_with(|| format!("Locking transaction intent {}", input.intent_id))?;

    if intent.status == intent_status::CONFIRMED {
        let position_id = intent.position_id.ok_or_else(|| {
            eyre::eyre!(
                "Transaction intent {} is confirmed but has no position_id",
                input.intent_id
            )
        })?;
        tx.commit()
            .await
            .wrap_err_with(|| "Committing no-op transaction intent confirmation")?;
        return Ok(position_id);
    }

    use crate::types::intent_action;
    let position_id = if intent.action == intent_action::OPEN {
        let position_address = input.position_address.as_deref().ok_or_else(|| {
            eyre::eyre!(
                "Confirming open intent {} without a position_address",
                input.intent_id
            )
        })?;
        let new_id = Uuid::new_v4();
        let row = sqlx::query!(
            r#"
            INSERT INTO positions (
                id, position_address, wallet_address, pool_address, venue, opened_at,
                entry_active_bin, lower_bin, upper_bin
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (position_address) DO UPDATE SET position_address = EXCLUDED.position_address
            RETURNING id
            "#,
            new_id,
            position_address,
            intent.wallet_address,
            intent.pool_address,
            intent.venue,
            input.confirmed_at,
            input.entry_active_bin,
            input.lower_bin,
            input.upper_bin,
        )
        .fetch_one(&mut *tx)
        .await
        .wrap_err_with(|| format!("Creating position for intent {}", input.intent_id))?;

        row.id
    } else {
        intent.position_id.ok_or_else(|| {
            eyre::eyre!(
                "Confirming non-open intent {} without a position_id",
                input.intent_id
            )
        })?
    };

    sqlx::query!(
        r#"
        INSERT INTO position_cash_flows (
            transaction_intent_id, position_id, kind, ts, amount_x_raw, amount_y_raw,
            amount_x, amount_y, price_x_usd, price_y_usd, value_usd, bin_liquidity
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (transaction_intent_id) DO NOTHING
        "#,
        input.intent_id,
        position_id,
        input.cash_flow.kind,
        input.cash_flow.ts,
        input.cash_flow.amount_x_raw,
        input.cash_flow.amount_y_raw,
        input.cash_flow.amount_x,
        input.cash_flow.amount_y,
        input.cash_flow.price_x_usd,
        input.cash_flow.price_y_usd,
        input.cash_flow.value_usd,
        input.cash_flow.bin_liquidity,
    )
    .execute(&mut *tx)
    .await
    .wrap_err_with(|| format!("Recording cash flow for intent {}", input.intent_id))?;

    if intent.action == intent_action::CLOSE {
        sqlx::query!(
            r#"
            UPDATE positions
            SET closed_at = COALESCE(closed_at, $2), close_reason = COALESCE(close_reason, $3)
            WHERE id = $1
            "#,
            position_id,
            input.confirmed_at,
            input.close_reason,
        )
        .execute(&mut *tx)
        .await
        .wrap_err_with(|| format!("Closing position {position_id}"))?;
    }

    sqlx::query!(
        r#"
        UPDATE transaction_intents
        SET status = $2,
            confirmed_at = COALESCE(confirmed_at, $3),
            slot = COALESCE(slot, $4),
            position_id = COALESCE(position_id, $5),
            updated_at = $3
        WHERE id = $1
        "#,
        input.intent_id,
        intent_status::CONFIRMED,
        input.confirmed_at,
        input.slot,
        position_id,
    )
    .execute(&mut *tx)
    .await
    .wrap_err_with(|| format!("Marking transaction intent {} confirmed", input.intent_id))?;

    tx.commit()
        .await
        .wrap_err_with(|| "Committing transaction intent confirmation")?;

    Ok(position_id)
}

// Never overwrites a CONFIRMED intent -- a late failure notification for something that has
// since confirmed on-chain (a common race: submission times out client-side while the
// transaction actually lands) must not un-confirm real money movement.
pub async fn mark_intent_failed(
    pool: &PgPool,
    id: Uuid,
    failed_at: DateTime<Utc>,
    reason: &str,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        UPDATE transaction_intents
        SET status = CASE WHEN status = $3 THEN status ELSE $2 END,
            failed_at = COALESCE(failed_at, $4),
            failure_reason = COALESCE(failure_reason, $5),
            updated_at = $4
        WHERE id = $1
        "#,
        id,
        intent_status::FAILED,
        intent_status::CONFIRMED,
        failed_at,
        reason,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Marking transaction intent {id} failed"))?;

    Ok(())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::{ensure_pool_fixture, reset_wallet_fixture, test_pool};
    use crate::types::intent_action;
    use crate::write::{NewWallet, register_wallet};

    async fn ensure_wallet(pool: &PgPool, pubkey: &str, telegram_user_id: i64) {
        register_wallet(
            pool,
            &NewWallet {
                pubkey: pubkey.to_string(),
                telegram_user_id,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    }

    fn sample_open_intent(
        id: Uuid,
        wallet: &str,
        pool_address: &str,
        idem: &str,
    ) -> NewTransactionIntent {
        NewTransactionIntent {
            id,
            wallet_address: wallet.to_string(),
            position_id: None,
            pool_address: pool_address.to_string(),
            venue: crate::types::venue::DLMM,
            action: intent_action::OPEN,
            idempotency_key: idem.to_string(),
            unsigned_tx_base64: "dW5zaWduZWQ=".to_string(),
            params: None,
            created_at: Utc::now(),
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn test_create_transaction_intent_is_idempotent_on_wallet_and_idempotency_key() {
        let pool = test_pool().await;
        let wallet = "wallet_intent_create_idem_1111111111111111";
        let pool_address = "pool_intent_create_idem";
        reset_wallet_fixture(&pool, wallet).await;
        ensure_pool_fixture(&pool, pool_address).await;
        ensure_wallet(&pool, wallet, 1).await;

        let first_id = Uuid::new_v4();
        let row = sample_open_intent(first_id, wallet, pool_address, "open-button-1");

        let first = create_transaction_intent(&pool, &row).await.unwrap();
        assert_eq!(first.id, first_id);

        // A retried "build me the transaction" request with a fresh server-generated id but the
        // same client idempotency key must resolve to the original row, not create a second one.
        let retry_id = Uuid::new_v4();
        let mut retry_row = row.clone();
        retry_row.id = retry_id;
        let second = create_transaction_intent(&pool, &retry_row).await.unwrap();
        assert_eq!(second.id, first_id);

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM transaction_intents WHERE wallet_address = $1",
            wallet
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, Some(1));
    }

    #[tokio::test]
    async fn test_mark_intent_submitted_is_idempotent() {
        let pool = test_pool().await;
        let wallet = "wallet_intent_submit_idem_111111111111111";
        let pool_address = "pool_intent_submit_idem";
        reset_wallet_fixture(&pool, wallet).await;
        ensure_pool_fixture(&pool, pool_address).await;
        ensure_wallet(&pool, wallet, 2).await;

        let id = Uuid::new_v4();
        let row = sample_open_intent(id, wallet, pool_address, "open-button-2");
        create_transaction_intent(&pool, &row).await.unwrap();

        let submitted_at = Utc::now();
        mark_intent_submitted(&pool, id, "sig_intent_submit_idem", submitted_at)
            .await
            .unwrap();
        mark_intent_submitted(
            &pool,
            id,
            "sig_intent_submit_idem",
            submitted_at + chrono::Duration::seconds(5),
        )
        .await
        .unwrap();

        let stored = sqlx::query!(
            "SELECT status, signature, submitted_at FROM transaction_intents WHERE id = $1",
            id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored.status, intent_status::SUBMITTED);
        assert_eq!(stored.signature.as_deref(), Some("sig_intent_submit_idem"));
        assert_eq!(
            stored.submitted_at.unwrap().timestamp_millis(),
            submitted_at.timestamp_millis()
        );
    }

    #[tokio::test]
    async fn test_signature_cannot_be_attached_to_two_intents() {
        let pool = test_pool().await;
        let wallet = "wallet_intent_sig_unique_1111111111111111";
        let pool_address = "pool_intent_sig_unique";
        reset_wallet_fixture(&pool, wallet).await;
        ensure_pool_fixture(&pool, pool_address).await;
        ensure_wallet(&pool, wallet, 3).await;

        let first = Uuid::new_v4();
        create_transaction_intent(
            &pool,
            &sample_open_intent(first, wallet, pool_address, "open-button-3a"),
        )
        .await
        .unwrap();
        let second = Uuid::new_v4();
        create_transaction_intent(
            &pool,
            &sample_open_intent(second, wallet, pool_address, "open-button-3b"),
        )
        .await
        .unwrap();

        mark_intent_submitted(&pool, first, "sig_intent_sig_unique", Utc::now())
            .await
            .unwrap();

        // A different intent trying to claim an already-used signature must fail outright
        // rather than silently attach it -- the structural half of the double-submission
        // guarantee described in migration 0030.
        let result =
            mark_intent_submitted(&pool, second, "sig_intent_sig_unique", Utc::now()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_confirm_open_intent_creates_position_and_cash_flow_idempotently() {
        let pool = test_pool().await;
        let wallet = "wallet_intent_confirm_open_111111111111111";
        let pool_address = "pool_intent_confirm_open";
        reset_wallet_fixture(&pool, wallet).await;
        ensure_pool_fixture(&pool, pool_address).await;
        ensure_wallet(&pool, wallet, 4).await;

        let id = Uuid::new_v4();
        create_transaction_intent(
            &pool,
            &sample_open_intent(id, wallet, pool_address, "open-button-4"),
        )
        .await
        .unwrap();
        mark_intent_submitted(&pool, id, "sig_intent_confirm_open", Utc::now())
            .await
            .unwrap();

        let confirm = ConfirmTransactionIntent {
            intent_id: id,
            confirmed_at: Utc::now(),
            slot: 555,
            position_address: Some("position_confirm_open_11111111111111111111".to_string()),
            entry_active_bin: Some(100),
            lower_bin: Some(90),
            upper_bin: Some(110),
            cash_flow: ConfirmedCashFlow {
                kind: crate::types::cash_flow_kind::DEPOSIT,
                ts: Utc::now(),
                amount_x_raw: Some(Decimal::new(1_000_000, 0)),
                amount_y_raw: Some(Decimal::new(2_000_000, 0)),
                amount_x: Some(Decimal::new(1, 0)),
                amount_y: Some(Decimal::new(2, 0)),
                price_x_usd: Some(Decimal::new(150, 2)),
                price_y_usd: Some(Decimal::new(1, 0)),
                value_usd: Some(Decimal::new(35, 1)),
                bin_liquidity: None,
            },
            close_reason: None,
        };

        let position_id_first = confirm_transaction_intent(&pool, &confirm).await.unwrap();
        let position_id_second = confirm_transaction_intent(&pool, &confirm).await.unwrap();
        assert_eq!(position_id_first, position_id_second);

        let position_count = sqlx::query_scalar!(
            "SELECT count(*) FROM positions WHERE wallet_address = $1",
            wallet
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(position_count, Some(1));

        let cash_flow_count = sqlx::query_scalar!(
            "SELECT count(*) FROM position_cash_flows WHERE position_id = $1",
            position_id_first
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cash_flow_count, Some(1));

        let status =
            sqlx::query_scalar!("SELECT status FROM transaction_intents WHERE id = $1", id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, intent_status::CONFIRMED);
    }

    #[tokio::test]
    async fn test_mark_intent_failed_does_not_overwrite_confirmed() {
        let pool = test_pool().await;
        let wallet = "wallet_intent_no_downgrade_11111111111111";
        let pool_address = "pool_intent_no_downgrade";
        reset_wallet_fixture(&pool, wallet).await;
        ensure_pool_fixture(&pool, pool_address).await;
        ensure_wallet(&pool, wallet, 5).await;

        let id = Uuid::new_v4();
        create_transaction_intent(
            &pool,
            &sample_open_intent(id, wallet, pool_address, "open-button-5"),
        )
        .await
        .unwrap();
        mark_intent_submitted(&pool, id, "sig_intent_no_downgrade", Utc::now())
            .await
            .unwrap();

        confirm_transaction_intent(
            &pool,
            &ConfirmTransactionIntent {
                intent_id: id,
                confirmed_at: Utc::now(),
                slot: 1,
                position_address: Some("position_no_downgrade_1111111111111111111".to_string()),
                entry_active_bin: Some(1),
                lower_bin: Some(0),
                upper_bin: Some(10),
                cash_flow: ConfirmedCashFlow {
                    kind: crate::types::cash_flow_kind::DEPOSIT,
                    ts: Utc::now(),
                    amount_x_raw: None,
                    amount_y_raw: None,
                    amount_x: None,
                    amount_y: None,
                    price_x_usd: None,
                    price_y_usd: None,
                    value_usd: None,
                    bin_liquidity: None,
                },
                close_reason: None,
            },
        )
        .await
        .unwrap();

        // A late "it failed" notification racing a confirmation that actually landed must not
        // downgrade a confirmed, real transaction.
        mark_intent_failed(&pool, id, Utc::now(), "timed out client-side")
            .await
            .unwrap();

        let status =
            sqlx::query_scalar!("SELECT status FROM transaction_intents WHERE id = $1", id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, intent_status::CONFIRMED);
    }
}
