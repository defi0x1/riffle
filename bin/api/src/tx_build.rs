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
    Pubkey::from_str(s)
        .map_err(|_| ApiError::BadRequest(format!("{field} is not a valid base58 pubkey")))
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
        .ok_or_else(|| {
            ApiError::Internal(eyre::eyre!("Transaction intent {} has no params", row.id))
        })
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| ApiError::Internal(eyre::eyre!("Decoding cached intent params: {e}")))
        })?;

    Ok(cached.response)
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use chrono::Utc;
    use rust_decimal::Decimal;

    use super::*;
    use crate::dto::{BuildTxResponse, ClosePositionSummary, SimulationDto, TxSummary};
    use crate::test_support::{test_pool, test_state};

    async fn ensure_wallet_and_pool(pool: &sqlx::PgPool, wallet: &str, pool_address: &str) {
        storage::write::register_wallet(
            pool,
            &storage::write::NewWallet {
                pubkey: wallet.to_string(),
                telegram_user_id: (uuid::Uuid::new_v4().as_u128() % (i64::MAX as u128)) as i64,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let now = Utc::now();
        storage::write::upsert_dlmm_pool(
            pool,
            &storage::write::NewPool {
                pool_address: pool_address.to_string(),
                venue: storage::types::venue::DLMM,
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
            },
            &storage::write::NewDlmmPoolParams {
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
            },
        )
        .await
        .unwrap();
    }

    fn sample_response(idempotency_key: &str, tag: &str) -> BuildTxResponse {
        BuildTxResponse {
            unsigned_transaction: format!("unsigned-{tag}"),
            expiry_blockhash: "11111111111111111111111111111111111111111".to_string(),
            expiry_last_valid_block_height: 100,
            idempotency_key: idempotency_key.to_string(),
            simulation: SimulationDto {
                success: true,
                error: None,
                logs_tail: vec![],
            },
            estimated_network_fee_lamports: "5000".to_string(),
            summary: TxSummary::ClosePosition(ClosePositionSummary {
                position_address: "position_placeholder".to_string(),
                rent_receiver: "rent_receiver_placeholder".to_string(),
            }),
        }
    }

    #[tokio::test]
    async fn test_same_idempotency_key_returns_the_first_response_not_a_second_build() {
        // Fresh identifiers per run: this crate has no delete/reset helper of its own (all SQL
        // stays in libraries/storage), so a repeated run against a persistent test database
        // must not collide with rows an earlier run left behind.
        let wallet = format!("wallet_intent_idem_{}", uuid::Uuid::new_v4());
        let pool_address = format!("pool_intent_idem_{}", uuid::Uuid::new_v4());
        let idempotency_key = format!("idem-key-tx-build-test-{}", uuid::Uuid::new_v4());

        let pool = test_pool().await;
        ensure_wallet_and_pool(&pool, &wallet, &pool_address).await;
        let state = test_state(pool);

        let first = create_intent_idempotently(
            &state,
            IntentInsert {
                wallet_address: wallet.clone(),
                position_id: None,
                pool_address: pool_address.clone(),
                action: storage::types::intent_action::CLOSE,
                idempotency_key: idempotency_key.clone(),
                request_json: serde_json::json!({}),
                token_x_decimals: None,
                token_y_decimals: None,
                response: sample_response(&idempotency_key, "first"),
                expires_at: Utc::now() + chrono::Duration::seconds(90),
            },
        )
        .await
        .unwrap();
        assert_eq!(first.unsigned_transaction, "unsigned-first");

        // A second call under the same (wallet, idempotency_key) -- as a retried "build me the
        // transaction" request would produce -- must resolve to the first build's bytes, never
        // mint a second, different one.
        let second = create_intent_idempotently(
            &state,
            IntentInsert {
                wallet_address: wallet.clone(),
                position_id: None,
                pool_address: pool_address.clone(),
                action: storage::types::intent_action::CLOSE,
                idempotency_key: idempotency_key.clone(),
                request_json: serde_json::json!({}),
                token_x_decimals: None,
                token_y_decimals: None,
                response: sample_response(&idempotency_key, "second"),
                expires_at: Utc::now() + chrono::Duration::seconds(90),
            },
        )
        .await
        .unwrap();
        assert_eq!(second.unsigned_transaction, "unsigned-first");

        // Reusing the storage layer's own read path rather than a raw COUNT query, matching
        // this crate's "no SQL of its own" constraint even in tests: exactly one intent exists
        // for this wallet, still pending (CREATED), so it appears exactly once here.
        let pending = storage::queries::pending_intents_for_wallet(&state.db, &wallet)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].idempotency_key, idempotency_key);
    }
}
