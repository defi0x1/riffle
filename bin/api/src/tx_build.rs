//! Shared plumbing every one of the five tx-build handlers (`routes::tx`) uses: parsing
//! helpers, and the idempotent-create dance that makes `POST /tx/*` honour the miniapp
//! contract's idempotency rule -- "the same idempotency key from the same wallet must return
//! the same intent and the same bytes, never build a second one."
//!
//! The storage layer's own `create_transaction_intent` already resolves that via
//! `INSERT ... ON CONFLICT (wallet_address, idempotency_key) DO UPDATE ... RETURNING`: a retry
//! always gets back the *first* row's data regardless of what this service just built. So every
//! handler here always builds a full candidate (parses, risk-gates, decodes live chain state,
//! compiles, simulates) and then asks Postgres which row is authoritative -- trading a wasted
//! RPC round trip on a retry for never needing a separate check-then-act read path, the same
//! philosophy the task asks for on the submit side.

use chrono::Utc;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use uuid::Uuid;

use crate::dto::{BuildTxResponse, IntentParams};
use crate::error::ApiError;
use crate::state::AppState;

pub fn parse_pubkey(s: &str, field: &str) -> Result<Pubkey, ApiError> {
    Pubkey::from_str(s).map_err(|_| ApiError::BadRequest(format!("{field} is not a valid base58 pubkey")))
}

pub fn parse_amount_raw(s: &str, field: &str) -> Result<u64, ApiError> {
    s.parse::<u64>()
        .map_err(|_| ApiError::BadRequest(format!("{field} is not a valid non-negative integer")))
}

/// No `queries::tokens` read path is exposed by the storage layer (only `write::tokens` exists,
/// for the indexer's own metadata refresh) -- see this task's report for the gap. A real symbol
/// needs off-chain token metadata this keyless service has no other reason to fetch, so this
/// stands in with the mint's own address, shortened, until that gap is closed.
pub fn mint_symbol_placeholder(mint: &Pubkey) -> String {
    let s = mint.to_string();
    s.chars().take(4).collect()
}

/// The `wallet_address, idempotency_key` this intent must resolve to, plus everything needed to
/// reconstruct a `BuildTxResponse` from a stored row.
pub struct IntentInsert {
    pub wallet_address: String,
    pub position_id: Option<Uuid>,
    pub pool_address: String,
    pub action: i16,
    pub idempotency_key: String,
    pub request_json: serde_json::Value,
    pub token_x_decimals: Option<u8>,
    pub token_y_decimals: Option<u8>,
    pub response: BuildTxResponse,
    pub expires_at: chrono::DateTime<Utc>,
}

/// Attempts to record `insert` as a brand new transaction_intent. Returns the `BuildTxResponse`
/// that is now authoritative for this (wallet, idempotency_key) pair -- `insert.response` if
/// this call actually created the row, or the first call's cached response decoded back out of
/// `params` if a prior build already claimed this key. A `CONFIRMED` prior intent short-circuits
/// as a conflict instead: the miniapp contract's `BuildTxResponse` has no field for "here is the
/// existing signature", so this is expressed the same way every other refusal is, through the
/// shared `ApiErrorBody` shape the client already knows how to render (see the report on this
/// task for why this is the one place the contract's success shape could not be honoured as-is).
pub async fn create_intent_idempotently(
    state: &AppState,
    insert: IntentInsert,
) -> Result<BuildTxResponse, ApiError> {
    let candidate_id = Uuid::new_v4();
    let params = IntentParams {
        request: insert.request_json,
        token_x_decimals: insert.token_x_decimals,
        token_y_decimals: insert.token_y_decimals,
        response: insert.response.clone(),
    };
    let params_json = serde_json::to_value(&params)
        .map_err(|e| ApiError::Internal(eyre::eyre!("Serialising intent params: {e}")))?;

    let row = storage::write::create_transaction_intent(
        &state.db,
        &storage::write::NewTransactionIntent {
            id: candidate_id,
            wallet_address: insert.wallet_address,
            position_id: insert.position_id,
            pool_address: insert.pool_address,
            venue: storage::types::venue::DLMM,
            action: insert.action,
            idempotency_key: insert.idempotency_key,
            unsigned_tx_base64: insert.response.unsigned_transaction.clone(),
            params: Some(params_json),
            created_at: Utc::now(),
            expires_at: Some(insert.expires_at),
        },
    )
    .await
    .map_err(ApiError::Internal)?;

    if row.id == candidate_id {
        return Ok(insert.response);
    }

    // A prior build already owns this idempotency key. Its status decides how to respond.
    if row.status == storage::types::intent_status::CONFIRMED {
        let signature = row.signature.as_deref().unwrap_or("<unknown>");
        return Err(ApiError::conflict(
            "already_confirmed",
            format!(
                "This action already confirmed on chain (signature {signature}); use a new \
                 idempotency key to build a fresh transaction"
            ),
        ));
    }

    let cached: IntentParams = row
        .params
        .ok_or_else(|| ApiError::Internal(eyre::eyre!("Transaction intent {} has no params", row.id)))
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| ApiError::Internal(eyre::eyre!("Decoding cached intent params: {e}")))
        })?;

    Ok(cached.response)
}
