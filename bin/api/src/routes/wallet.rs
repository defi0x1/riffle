use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::Utc;
use solana_account_decoder::UiAccountData;
use solana_rpc_client_api::request::TokenAccountsFilter;

use crate::dto::{BalancesResponse, RegisterWalletRequest, RegisterWalletResponse, TokenBalance};
use crate::error::ApiError;
use crate::state::AppState;
use crate::{tx_build, wallet_resolve};

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterWalletRequest>,
) -> Result<Json<RegisterWalletResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let pubkey = tx_build::parse_pubkey(&req.pubkey, "pubkey")?;
    let pubkey_str = pubkey.to_string();
    let now = Utc::now();

    let outcome = storage::write::register_wallet(
        &state.db,
        &storage::write::NewWallet {
            pubkey: pubkey_str.clone(),
            telegram_user_id: user.id,
            label: None,
            registered_at: now,
        },
    )
    .await
    .map_err(ApiError::Internal)?;

    match outcome {
        storage::write::RegisterWalletOutcome::OwnedByAnotherUser { .. } => Err(ApiError::conflict(
            "wallet_owned_by_other",
            "This wallet is already registered to a different Telegram account",
        )),
        storage::write::RegisterWalletOutcome::Registered
        | storage::write::RegisterWalletOutcome::AlreadyOwnedByCaller => {
            let wallets = storage::queries::active_wallets_for_user(&state.db, user.id)
                .await
                .map_err(ApiError::Internal)?;
            let registered_at = wallets
                .iter()
                .find(|w| w.pubkey == pubkey_str)
                .map(|w| w.registered_at)
                .unwrap_or(now);

            tracing::info!(telegram_user_id = user.id, "Wallet registered");
            Ok(Json(RegisterWalletResponse {
                registered_at: registered_at.to_rfc3339(),
            }))
        }
    }
}

pub async fn balances(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BalancesResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    // Read live from chain, per the miniapp contract -- balances are explicitly not served
    // from a cache here (see storage's wallet_balances table, which this handler deliberately
    // does not read: it exists for a future periodic-refresh worker, not for this endpoint).
    let sol_lamports = state
        .rpc
        .get_balance(&pubkey)
        .await
        .map_err(|e| ApiError::Internal(eyre::eyre!("Fetching SOL balance: {e}")))?;

    let mut tokens = Vec::new();
    for program_id in [dlmm_tx::TOKEN_PROGRAM_ID, dlmm_tx::TOKEN_2022_PROGRAM_ID] {
        let accounts = state
            .rpc
            .get_token_accounts_by_owner(&pubkey, TokenAccountsFilter::ProgramId(program_id))
            .await
            .map_err(|e| ApiError::Internal(eyre::eyre!("Fetching token accounts: {e}")))?;

        for keyed_account in accounts {
            let UiAccountData::Json(parsed) = keyed_account.account.data else {
                continue;
            };
            let info = &parsed.parsed["info"];
            let (Some(mint), Some(amount), Some(decimals)) = (
                info["mint"].as_str(),
                info["tokenAmount"]["amount"].as_str(),
                info["tokenAmount"]["decimals"].as_u64(),
            ) else {
                continue;
            };
            tokens.push(TokenBalance {
                mint: mint.to_string(),
                program_id: keyed_account.account.owner,
                amount_raw: amount.to_string(),
                decimals: decimals as u8,
            });
        }
    }

    Ok(Json(BalancesResponse {
        sol_lamports: sol_lamports.to_string(),
        tokens,
    }))
}
