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
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_status_client_types::TransactionConfirmationStatus;

use crate::dto::{
    IntentParams, SubmitTxRequest, SubmitTxResponse, TxStatus, TxStatusQuery, TxStatusResponse,
};
use crate::error::ApiError;
use crate::state::AppState;
use crate::{rpc_ext, tx_build, tx_events, wallet_resolve};

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
        confirm_intent(state, intent, signature, status.slot as i64).await?;
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

/// Fetches the just-confirmed transaction and decodes its DLMM events looking for one matching
/// `position`, via `select` (one of `tx_events::{add_liquidity_amounts, remove_liquidity_amounts,
/// claim_fee_amounts}`). `None` on any failure along the way -- an RPC fetch that errors or
/// returns nothing, or a transaction with no decodable event for this position -- each logged
/// with its own reason here, since the caller's only recourse either way is to leave the amount
/// null rather than invent one (item 5: a wrong number in a profit ledger is worse than a
/// missing one, because nobody can tell it is wrong).
async fn recover_event_amounts(
    state: &AppState,
    signature: &Signature,
    position: &Pubkey,
    action_label: &'static str,
    select: fn(&[dlmm_decode::DecodedEvent], &Pubkey) -> Option<tx_events::RecoveredAmounts>,
) -> Option<tx_events::RecoveredAmounts> {
    let tx = match tx_events::fetch_transaction(&state.rpc, signature).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(
                error = ?e, %signature, action = action_label,
                "Fetching confirmed transaction to recover its amounts failed"
            );
            return None;
        }
    };

    let events = tx_events::decode_dlmm_events(&tx);
    let recovered = select(&events, position);
    if recovered.is_none() {
        tracing::error!(
            %signature, %position, action = action_label,
            "Confirmed transaction carried no decodable DLMM event for this position"
        );
    }
    recovered
}

/// Applies a recovered on-chain amount pair to a cash-flow row in progress, scaling by the
/// intent's cached token decimals the same way the request-derived path already does.
fn apply_recovered_amounts(
    cf: &mut storage::write::ConfirmedCashFlow,
    recovered: tx_events::RecoveredAmounts,
    token_x_decimals: Option<u8>,
    token_y_decimals: Option<u8>,
) {
    cf.amount_x_raw = Some(Decimal::from(recovered.amount_x_raw));
    cf.amount_y_raw = Some(Decimal::from(recovered.amount_y_raw));
    cf.amount_x = decimal_from_raw(Some(recovered.amount_x_raw), token_x_decimals);
    cf.amount_y = decimal_from_raw(Some(recovered.amount_y_raw), token_y_decimals);
}

/// The `positionAddress` an add/remove/claim intent's own original request carries, parsed --
/// or `None` when it is missing or unparsable, which callers treat exactly like a failed
/// recovery (there is nothing to look an event up against).
fn requested_position(params: &IntentParams) -> Option<Pubkey> {
    let address = params
        .request
        .get("positionAddress")
        .and_then(|v| v.as_str())?;
    tx_build::parse_pubkey(address, "positionAddress").ok()
}

/// Records the confirmed intent's position and cash-flow ledger row. `open`'s position address,
/// entry active bin and bin range come from a live re-read of the now-confirmed accounts.
/// `add`, `remove` and `claim` all attempt to recover their real amounts by decoding the
/// confirmed transaction's own DLMM events (see `recover_event_amounts` and `tx_events`); `add`
/// falls back to the originally requested amounts if that recovery fails (a strategy deposit's
/// request is a real, if possibly imprecise, number -- unlike remove/claim, which have no
/// analogous fallback and simply leave their amounts null), and `remove`/`claim` leave their
/// amounts null on a failed recovery, exactly as before this pass, rather than guess.
async fn confirm_intent(
    state: &AppState,
    intent: &storage::write::TransactionIntentRow,
    signature: &Signature,
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
            // Requested amounts, from the intent's own original request -- the fallback when
            // event recovery below finds nothing, since a strategy deposit's request is a real
            // number the caller chose, just not necessarily what the bins actually accepted.
            let requested_amount_x_raw = params
                .request
                .get("amountXRaw")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            let requested_amount_y_raw = params
                .request
                .get("amountYRaw")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());

            let mut cf = empty_cash_flow(cash_flow_kind::DEPOSIT);
            match requested_position(&params) {
                Some(position) => {
                    match recover_event_amounts(
                        state,
                        signature,
                        &position,
                        "add",
                        tx_events::add_liquidity_amounts,
                    )
                    .await
                    {
                        Some(recovered) => apply_recovered_amounts(
                            &mut cf,
                            recovered,
                            params.token_x_decimals,
                            params.token_y_decimals,
                        ),
                        None => {
                            cf.amount_x_raw = requested_amount_x_raw.map(Decimal::from);
                            cf.amount_y_raw = requested_amount_y_raw.map(Decimal::from);
                            cf.amount_x =
                                decimal_from_raw(requested_amount_x_raw, params.token_x_decimals);
                            cf.amount_y =
                                decimal_from_raw(requested_amount_y_raw, params.token_y_decimals);
                        }
                    }
                }
                None => {
                    tracing::error!(
                        intent_id = %intent.id,
                        "Add intent has no usable positionAddress in its stored request, \
                         falling back to requested amounts"
                    );
                    cf.amount_x_raw = requested_amount_x_raw.map(Decimal::from);
                    cf.amount_y_raw = requested_amount_y_raw.map(Decimal::from);
                    cf.amount_x = decimal_from_raw(requested_amount_x_raw, params.token_x_decimals);
                    cf.amount_y = decimal_from_raw(requested_amount_y_raw, params.token_y_decimals);
                }
            }
            (None, None, None, None, cf, None)
        } else if intent.action == intent_action::REMOVE {
            let mut cf = empty_cash_flow(cash_flow_kind::WITHDRAWAL);
            match requested_position(&params) {
                Some(position) => {
                    if let Some(recovered) = recover_event_amounts(
                        state,
                        signature,
                        &position,
                        "remove",
                        tx_events::remove_liquidity_amounts,
                    )
                    .await
                    {
                        apply_recovered_amounts(
                            &mut cf,
                            recovered,
                            params.token_x_decimals,
                            params.token_y_decimals,
                        );
                    }
                }
                None => tracing::error!(
                    intent_id = %intent.id,
                    "Remove intent has no usable positionAddress in its stored request, \
                     amounts stay null"
                ),
            }
            (None, None, None, None, cf, None)
        } else if intent.action == intent_action::CLAIM {
            let mut cf = empty_cash_flow(cash_flow_kind::FEE_CLAIM);
            match requested_position(&params) {
                Some(position) => {
                    if let Some(recovered) = recover_event_amounts(
                        state,
                        signature,
                        &position,
                        "claim",
                        tx_events::claim_fee_amounts,
                    )
                    .await
                    {
                        apply_recovered_amounts(
                            &mut cf,
                            recovered,
                            params.token_x_decimals,
                            params.token_y_decimals,
                        );
                    }
                }
                None => tracing::error!(
                    intent_id = %intent.id,
                    "Claim intent has no usable positionAddress in its stored request, \
                     amounts stay null"
                ),
            }
            (None, None, None, None, cf, None)
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

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::dto::{
        BuildTxResponse, RemoveLiquidityRequest, RemoveLiquiditySummary, SimulationDto, TxSummary,
    };
    use crate::test_support::{test_pool, test_state};
    use sqlx::PgPool;
    use storage::types::{cash_flow_kind, intent_action, venue};
    use storage::write::{
        ConfirmTransactionIntent, ConfirmedCashFlow, NewPool, NewTransactionIntent, NewWallet,
        confirm_transaction_intent, create_transaction_intent, mark_intent_submitted,
        register_wallet, upsert_pool,
    };
    use uuid::Uuid;

    async fn seed_wallet_and_pool(db: &PgPool, wallet: &str, pool_address: &str) {
        register_wallet(
            db,
            &NewWallet {
                pubkey: wallet.to_string(),
                telegram_user_id: 909_090,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        upsert_pool(
            db,
            &NewPool {
                pool_address: pool_address.to_string(),
                venue: venue::DLMM,
                token_x: "tokenXsubmittest1111111111111111111111111".to_string(),
                token_y: "tokenYsubmittest1111111111111111111111111".to_string(),
                base_fee_bps: Decimal::new(100, 0),
                protocol_share_bps: 500,
                tvl_usd: None,
                status: 0,
                creator: None,
                activation_point: None,
                created_at: Utc::now(),
                first_liquidity_at: None,
                is_blacklisted: false,
                launchpad: None,
                tags: vec![],
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    }

    /// Confirms a synthetic `open` intent directly against storage -- the shortest path to a
    /// real `positions` row for a `remove`/`claim` test to reference, without going through a
    /// live RPC-backed build-tx handler.
    async fn seed_open_position(db: &PgPool, wallet: &str, pool_address: &str) -> Uuid {
        let open_id = Uuid::new_v4();
        create_transaction_intent(
            db,
            &NewTransactionIntent {
                id: open_id,
                wallet_address: wallet.to_string(),
                position_id: None,
                pool_address: pool_address.to_string(),
                venue: venue::DLMM,
                action: intent_action::OPEN,
                idempotency_key: format!("open-for-{open_id}"),
                unsigned_tx_base64: "dW5zaWduZWQ=".to_string(),
                params: None,
                created_at: Utc::now(),
                expires_at: None,
            },
        )
        .await
        .unwrap();
        mark_intent_submitted(db, open_id, &format!("sig-open-{open_id}"), Utc::now())
            .await
            .unwrap();

        confirm_transaction_intent(
            db,
            &ConfirmTransactionIntent {
                intent_id: open_id,
                confirmed_at: Utc::now(),
                slot: 1,
                position_address: Some(format!("position-{open_id}")),
                entry_active_bin: Some(0),
                lower_bin: Some(-10),
                upper_bin: Some(10),
                cash_flow: ConfirmedCashFlow {
                    kind: cash_flow_kind::DEPOSIT,
                    ts: Utc::now(),
                    amount_x_raw: Some(Decimal::ZERO),
                    amount_y_raw: Some(Decimal::ZERO),
                    amount_x: Some(Decimal::ZERO),
                    amount_y: Some(Decimal::ZERO),
                    price_x_usd: None,
                    price_y_usd: None,
                    value_usd: None,
                    bin_liquidity: None,
                },
                close_reason: None,
            },
        )
        .await
        .unwrap()
    }

    // Exercises `confirm_intent`'s `remove` path end to end against a real database, but with
    // `state.rpc` pointed nowhere reachable (`test_state`'s deliberate choice) -- so event
    // recovery is guaranteed to fail, exactly like a real RPC outage would. Proves two things
    // this task cares about at once: the amount columns stay null rather than guessed (item 5),
    // and calling `confirm_intent` twice for the same intent -- the same thing a retried
    // `getSignatureStatuses` poll racing itself would do -- never produces a second cash-flow
    // row (item 4), relying entirely on `confirm_transaction_intent`'s own idempotency rather
    // than any check-then-act guard added here.
    #[tokio::test]
    async fn test_confirm_intent_remove_is_idempotent_and_leaves_amounts_null_on_unreachable_rpc() {
        // Every identifier below is derived from one fresh UUID rather than a fixed literal:
        // this test runs against a real, persistent database (not a per-test transaction that
        // rolls back), so a rerun must not collide with a previous run's rows on a unique
        // constraint (wallet pubkey, idempotency key, or signature) or silently resolve to
        // them via `create_transaction_intent`'s own idempotent-upsert behaviour.
        let run_id = Uuid::new_v4();
        let db = test_pool().await;
        let state = test_state(db.clone());
        let wallet = format!("wallet_confirm_remove_idem_{run_id}");
        let pool_address = format!("pool_confirm_remove_idem_{run_id}");
        seed_wallet_and_pool(&db, &wallet, &pool_address).await;
        let position_id = seed_open_position(&db, &wallet, &pool_address).await;
        let position_address = sqlx::query_scalar!(
            "SELECT position_address FROM positions WHERE id = $1",
            position_id
        )
        .fetch_one(&db)
        .await
        .unwrap();

        let request = RemoveLiquidityRequest {
            pool_address: pool_address.clone(),
            position_address: position_address.clone(),
            from_bin_id: -10,
            to_bin_id: 10,
            bps_to_remove: 10_000,
            idempotency_key: format!("remove-idem-{run_id}"),
        };
        let response = BuildTxResponse {
            unsigned_transaction: "dW5zaWduZWQ=".to_string(),
            expiry_blockhash: "11111111111111111111111111111111111111111".to_string(),
            expiry_last_valid_block_height: 1,
            idempotency_key: request.idempotency_key.clone(),
            simulation: SimulationDto {
                success: true,
                error: None,
                logs_tail: vec![],
            },
            estimated_network_fee_lamports: "5000".to_string(),
            summary: TxSummary::RemoveLiquidity(RemoveLiquiditySummary {
                pool_address: pool_address.to_string(),
                position_address: request.position_address.clone(),
                position_lower_bin_id: -10,
                position_upper_bin_id: 10,
                token_x_mint: "So11111111111111111111111111111111111111112".to_string(),
                token_y_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                token_x_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                token_y_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                from_bin_id: -10,
                to_bin_id: 10,
                bps_to_remove: 10_000,
            }),
        };
        let params = IntentParams {
            request: serde_json::to_value(&request).unwrap(),
            token_x_decimals: Some(9),
            token_y_decimals: Some(6),
            response,
        };

        let remove_id = Uuid::new_v4();
        create_transaction_intent(
            &db,
            &NewTransactionIntent {
                id: remove_id,
                wallet_address: wallet.to_string(),
                position_id: Some(position_id),
                pool_address: pool_address.to_string(),
                venue: venue::DLMM,
                action: intent_action::REMOVE,
                idempotency_key: request.idempotency_key.clone(),
                unsigned_tx_base64: "dW5zaWduZWQ=".to_string(),
                params: Some(serde_json::to_value(&params).unwrap()),
                created_at: Utc::now(),
                expires_at: None,
            },
        )
        .await
        .unwrap();

        // A signature derived from `run_id` rather than a fixed placeholder, for the same
        // cross-run-collision reason as the identifiers above -- `signature` is UNIQUE too.
        let signature_bytes: Vec<u8> = run_id.as_bytes().iter().cycle().take(64).copied().collect();
        let signature = Signature::try_from(signature_bytes.as_slice()).unwrap();
        mark_intent_submitted(&db, remove_id, &signature.to_string(), Utc::now())
            .await
            .unwrap();
        let intent = storage::queries::intent_by_signature(&db, &signature.to_string())
            .await
            .unwrap()
            .expect("intent was just created");

        confirm_intent(&state, &intent, &signature, 42)
            .await
            .expect("first confirmation");
        confirm_intent(&state, &intent, &signature, 42)
            .await
            .expect("second confirmation must be a harmless no-op");

        let row = sqlx::query!(
            "SELECT amount_x_raw, amount_y_raw FROM position_cash_flows \
             WHERE transaction_intent_id = $1",
            remove_id
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(
            row.amount_x_raw.is_none() && row.amount_y_raw.is_none(),
            "amounts must stay null when the confirmed transaction cannot be fetched, not be \
             guessed"
        );

        let count = sqlx::query_scalar!(
            "SELECT count(*) FROM position_cash_flows WHERE transaction_intent_id = $1",
            remove_id
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(
            count,
            Some(1),
            "re-confirming must not double the cash-flow row"
        );
    }
}
