//! Relays an already-signed transaction (item 4): submits it, records the signature against its
//! transaction_intent, and polls to confirmation. The unique constraints on
//! `transaction_intents` (wallet_address, idempotency_key) and (signature) already make a
//! replayed submission impossible at the database level -- `mark_intent_submitted` and
//! `mark_intent_failed` are both written to be safe to call more than once for the same intent,
//! so this handler never needs a check-then-act guard of its own.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use chrono::Utc;
use rust_decimal::Decimal;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_status_client_types::TransactionConfirmationStatus;

use crate::dto::{
    IntentParams, SubmitTxRequest, SubmitTxResponse, TxStatus, TxStatusQuery, TxStatusResponse,
};
use crate::error::ApiError;
use crate::state::AppState;
use crate::{rpc_ext, tx_build, wallet_resolve};

pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SubmitTxRequest>,
) -> Result<Json<SubmitTxResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let wallet_pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    let bytes = BASE64_STANDARD
        .decode(&req.signed_transaction)
        .map_err(|_| ApiError::BadRequest("signedTransaction is not valid base64".to_string()))?;
    let tx: VersionedTransaction = bincode::deserialize(&bytes).map_err(|_| {
        ApiError::BadRequest("signedTransaction is not a valid transaction".to_string())
    })?;

    if tx.signatures.is_empty() || tx.signatures[0] == Signature::default() {
        return Err(ApiError::BadRequest(
            "signedTransaction has no fee-payer signature".to_string(),
        ));
    }
    // Not instruction-level inspection (the miniapp contract asks this relay to stay opaque to
    // that) -- just confirming the caller is submitting something they themselves are paying
    // for and signing, so this endpoint cannot be used to relay an arbitrary third party's
    // transaction through this service's RPC connection.
    let fee_payer = tx
        .message
        .static_account_keys()
        .first()
        .copied()
        .unwrap_or_default();
    if fee_payer != wallet_pubkey {
        return Err(ApiError::refused(
            "fee_payer_mismatch",
            "The signed transaction's fee payer does not match your registered wallet",
        ));
    }

    let pending = storage::queries::pending_intents_for_wallet(&state.db, &wallet.pubkey)
        .await
        .map_err(ApiError::Internal)?;
    let intent = pending
        .into_iter()
        .find(|i| i.idempotency_key == req.idempotency_key)
        .ok_or_else(|| {
            ApiError::NotFound(
                "No pending transaction intent for this idempotency key -- build one first"
                    .to_string(),
            )
        })?;

    let (signature, submit_result) = rpc_ext::submit_signed_transaction(&state.rpc, &tx).await;
    let now = Utc::now();

    // Recorded regardless of outcome: the signature is deterministic from the signed bytes the
    // moment they exist, whether or not the network ever accepted them, and GET /tx/status
    // looks intents up by signature -- it must be able to find this one even if submission
    // failed outright at preflight.
    storage::write::mark_intent_submitted(&state.db, intent.id, &signature.to_string(), now)
        .await
        .map_err(ApiError::Internal)?;

    if let Err(message) = submit_result {
        storage::write::mark_intent_failed(&state.db, intent.id, now, &message)
            .await
            .map_err(ApiError::Internal)?;
        tracing::info!(telegram_user_id = user.id, %signature, "Submission rejected by RPC");
        return Ok(Json(SubmitTxResponse {
            signature: signature.to_string(),
            status: TxStatus::Failed,
        }));
    }

    tracing::info!(telegram_user_id = user.id, %signature, "Transaction submitted");

    let deadline = tokio::time::Instant::now() + state.config.confirmation_timeout;
    loop {
        if let Some((status, _)) = check_and_settle(&state, &intent, &signature).await? {
            return Ok(Json(SubmitTxResponse {
                signature: signature.to_string(),
                status,
            }));
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(state.config.confirmation_poll_interval).await;
    }

    // Not yet observed within the timeout -- still "submitted", not a failure. The client
    // polls GET /tx/status, which runs the same check.
    Ok(Json(SubmitTxResponse {
        signature: signature.to_string(),
        status: TxStatus::Submitted,
    }))
}

pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TxStatusQuery>,
) -> Result<Json<TxStatusResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;

    let intent = storage::queries::intent_by_signature(&state.db, &query.signature)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "No transaction found for signature {}",
                query.signature
            ))
        })?;
    if intent.wallet_address != wallet.pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This transaction does not belong to your registered wallet",
        ));
    }

    if intent.status == storage::types::intent_status::CONFIRMED {
        return Ok(Json(TxStatusResponse {
            signature: query.signature,
            status: TxStatus::Confirmed,
            error: None,
        }));
    }
    if intent.status == storage::types::intent_status::FAILED {
        // `TransactionIntentRow` (the shape both write:: and queries:: share for this table)
        // does not carry `failure_reason` -- only `write::mark_intent_failed`'s own caller ever
        // sees the detailed message, at the moment it happens. A generic message is returned
        // here rather than a second storage read this crate is not scoped to add.
        return Ok(Json(TxStatusResponse {
            signature: query.signature,
            status: TxStatus::Failed,
            error: Some("transaction failed".to_string()),
        }));
    }
    if intent.status == storage::types::intent_status::EXPIRED {
        return Ok(Json(TxStatusResponse {
            signature: query.signature,
            status: TxStatus::Expired,
            error: None,
        }));
    }

    // CREATED/SUBMITTED: this call is the "poll to confirmation" loop's continuation across
    // separate HTTP requests, past whatever POST /tx/submit itself managed to observe
    // synchronously.
    let signature: Signature = query.signature.parse().map_err(|_| {
        ApiError::BadRequest("signature is not a valid transaction signature".to_string())
    })?;
    match check_and_settle(&state, &intent, &signature).await? {
        Some((status, error)) => Ok(Json(TxStatusResponse {
            signature: query.signature,
            status,
            error,
        })),
        None => Ok(Json(TxStatusResponse {
            signature: query.signature,
            status: TxStatus::Submitted,
            error: None,
        })),
    }
}

/// One `getSignatureStatuses` check, settling the intent in Postgres if it has landed since the
/// last check. `None` means still unconfirmed -- try again later, not an error.
async fn check_and_settle(
    state: &AppState,
    intent: &storage::write::TransactionIntentRow,
    signature: &Signature,
) -> Result<Option<(TxStatus, Option<String>)>, ApiError> {
    let statuses = state
        .rpc
        .get_signature_statuses(&[*signature])
        .await
        .map_err(|e| ApiError::Internal(eyre::eyre!("Polling signature status: {e}")))?
        .value;

    let Some(Some(status)) = statuses.into_iter().next() else {
        return Ok(None);
    };

    if let Some(err) = &status.err {
        let message = format!("{err:?}");
        storage::write::mark_intent_failed(&state.db, intent.id, Utc::now(), &message)
            .await
            .map_err(ApiError::Internal)?;
        return Ok(Some((TxStatus::Failed, Some(message))));
    }

    let landed = matches!(
        status.confirmation_status,
        Some(TransactionConfirmationStatus::Confirmed)
            | Some(TransactionConfirmationStatus::Finalized)
    );
    if landed {
        confirm_intent(state, intent, status.slot as i64).await?;
        return Ok(Some((TxStatus::Confirmed, None)));
    }

    Ok(None)
}

fn decimal_from_raw(raw: Option<u64>, decimals: Option<u8>) -> Option<Decimal> {
    Some(Decimal::from_i128_with_scale(
        i128::from(raw?),
        u32::from(decimals?),
    ))
}

/// Records the confirmed intent's position and cash-flow ledger row. See the report on this
/// task for what is and is not derived here: `open`'s position address, entry active bin and
/// bin range come from a live re-read of the now-confirmed accounts; `add`'s deposited amounts
/// come from the original request (exact, since the caller specified them); `remove` and
/// `claim`'s actual withdrawn/collected amounts are not derived at all -- doing so needs to
/// decode the confirmed transaction's emitted DLMM events, which this pass does not implement --
/// so those two record a correctly kinded, correctly positioned ledger row with amounts left
/// null rather than an invented number.
async fn confirm_intent(
    state: &AppState,
    intent: &storage::write::TransactionIntentRow,
    slot: i64,
) -> Result<(), ApiError> {
    use storage::types::{cash_flow_kind, intent_action};

    let now = Utc::now();
    let params: IntentParams = intent
        .params
        .clone()
        .ok_or_else(|| ApiError::Internal(eyre::eyre!("Intent {} has no params", intent.id)))
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| ApiError::Internal(eyre::eyre!("Decoding intent params: {e}")))
        })?;

    let empty_cash_flow = |kind: i16| storage::write::ConfirmedCashFlow {
        kind,
        ts: now,
        amount_x_raw: None,
        amount_y_raw: None,
        amount_x: None,
        amount_y: None,
        price_x_usd: None,
        price_y_usd: None,
        value_usd: None,
        bin_liquidity: None,
    };

    let (position_address, entry_active_bin, lower_bin, upper_bin, cash_flow, close_reason) =
        if intent.action == intent_action::OPEN {
            let ephemeral = params
                .request
                .get("ephemeralPositionPubkey")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ApiError::Internal(eyre::eyre!("open intent missing ephemeralPositionPubkey"))
                })?;
            let position_pubkey = tx_build::parse_pubkey(ephemeral, "ephemeralPositionPubkey")?;
            let live_position = rpc_ext::fetch_live_position(&state.rpc, &position_pubkey).await?;
            let (lower, upper) = match &live_position {
                Some(p) => (Some(p.lower_bin_id), Some(p.upper_bin_id)),
                None => (None, None),
            };
            // Best-effort: the pool's *current* active bin, taken moments after confirmation --
            // not the exact bin at the confirmed slot, which would need historical state this
            // service does not read.
            let entry_active_bin = match tx_build::parse_pubkey(&intent.pool_address, "poolAddress")
            {
                Ok(lb_pair) => rpc_ext::fetch_live_pool(&state.rpc, &lb_pair)
                    .await
                    .ok()
                    .flatten()
                    .map(|p| p.state.active_bin_id),
                Err(_) => None,
            };
            // A pure `initialize_position2` moves no tokens -- this ledger row marks the
            // position's creation with a zero-value deposit rather than leaving the intent
            // without any cash-flow row at all.
            let mut cf = empty_cash_flow(cash_flow_kind::DEPOSIT);
            cf.amount_x_raw = Some(Decimal::ZERO);
            cf.amount_y_raw = Some(Decimal::ZERO);
            cf.amount_x = Some(Decimal::ZERO);
            cf.amount_y = Some(Decimal::ZERO);
            (
                Some(ephemeral.to_string()),
                entry_active_bin,
                lower,
                upper,
                cf,
                None,
            )
        } else if intent.action == intent_action::ADD {
            let amount_x_raw = params
                .request
                .get("amountXRaw")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let amount_y_raw = params
                .request
                .get("amountYRaw")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let mut cf = empty_cash_flow(cash_flow_kind::DEPOSIT);
            cf.amount_x_raw = amount_x_raw.map(Decimal::from);
            cf.amount_y_raw = amount_y_raw.map(Decimal::from);
            cf.amount_x = decimal_from_raw(amount_x_raw, params.token_x_decimals);
            cf.amount_y = decimal_from_raw(amount_y_raw, params.token_y_decimals);
            (None, None, None, None, cf, None)
        } else if intent.action == intent_action::REMOVE {
            (
                None,
                None,
                None,
                None,
                empty_cash_flow(cash_flow_kind::WITHDRAWAL),
                None,
            )
        } else if intent.action == intent_action::CLAIM {
            (
                None,
                None,
                None,
                None,
                empty_cash_flow(cash_flow_kind::FEE_CLAIM),
                None,
            )
        } else {
            (
                None,
                None,
                None,
                None,
                empty_cash_flow(cash_flow_kind::WITHDRAWAL),
                Some("user_closed".to_string()),
            )
        };

    storage::write::confirm_transaction_intent(
        &state.db,
        &storage::write::ConfirmTransactionIntent {
            intent_id: intent.id,
            confirmed_at: now,
            slot,
            position_address,
            entry_active_bin,
            lower_bin,
            upper_bin,
            cash_flow,
            close_reason,
        },
    )
    .await
    .map_err(ApiError::Internal)?;

    Ok(())
}
