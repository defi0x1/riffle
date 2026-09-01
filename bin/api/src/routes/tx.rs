//! The five build-tx handlers: open, add, remove, claim, close. Each one authenticates,
//! resolves the caller's own wallet, validates and risk-gates the request, reads live chain
//! state, builds unsigned instructions via `dlmm_tx`, compiles and simulates the transaction,
//! and records a transaction_intent before returning -- matching items 2 and 3 of this task.
//!
//! Only `open-position` and `add-liquidity` pass through the pool risk gate
//! (`risk::pool_risk_gate`): those are the two actions that put new capital at risk in a pool.
//! `remove-liquidity`, `claim-fees` and `close-position` are exit/harvest actions -- refusing
//! them because a pool was later blacklisted or demoted would trap a user's existing funds
//! rather than protect them, so they are deliberately exempt.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::Utc;

use crate::dto::{
    AddLiquidityRequest, AddLiquiditySummary, BuildTxResponse, ClaimFeesRequest, ClaimFeesSummary,
    ClosePositionRequest, ClosePositionSummary, OpenPositionRequest, OpenPositionSummary,
    RemoveLiquidityRequest, RemoveLiquiditySummary, TxSummary,
};
use crate::error::ApiError;
use crate::state::AppState;
use crate::{risk, rpc_ext, tx_build, wallet_resolve};

fn compute_budget_config(state: &AppState) -> dlmm_tx::ComputeBudgetConfig {
    dlmm_tx::ComputeBudgetConfig {
        unit_limit: Some(state.config.compute_unit_limit),
        unit_price_micro_lamports: if state.config.compute_unit_price_micro_lamports > 0 {
            Some(state.config.compute_unit_price_micro_lamports)
        } else {
            None
        },
    }
}

fn expires_at(state: &AppState) -> chrono::DateTime<Utc> {
    let delta =
        chrono::Duration::from_std(state.config.intent_expiry).unwrap_or(chrono::Duration::seconds(90));
    Utc::now() + delta
}

fn to_json_value<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(|e| ApiError::Internal(eyre::eyre!("Serialising request: {e}")))
}

pub async fn open_position(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OpenPositionRequest>,
) -> Result<Json<BuildTxResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let wallet_pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    risk::pool_risk_gate(&state, &req.pool_address).await?;
    let lb_pair = tx_build::parse_pubkey(&req.pool_address, "poolAddress")?;
    let position_pubkey = tx_build::parse_pubkey(&req.ephemeral_position_pubkey, "ephemeralPositionPubkey")?;

    let live_pool = rpc_ext::fetch_live_pool(&state.rpc, &lb_pair)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "pool_unavailable",
                "This pool's on-chain account could not be read",
            )
        })?;

    let instructions = dlmm_tx::build_open_position(
        &dlmm_tx::OpenPositionParams {
            lb_pair,
            owner: wallet_pubkey,
            payer: wallet_pubkey,
            position: position_pubkey,
            lower_bin_id: req.lower_bin_id,
            width: req.width,
        },
        &compute_budget_config(&state),
    )
    .map_err(|e| ApiError::refused("invalid_bin_range", e.to_string()))?;

    let built = rpc_ext::assemble_and_simulate(
        &state.rpc,
        &wallet_pubkey,
        instructions,
        state.config.compute_unit_limit,
        state.config.compute_unit_price_micro_lamports,
    )
    .await?;

    let summary = TxSummary::OpenPosition(OpenPositionSummary {
        pool_address: req.pool_address.clone(),
        token_x_mint: live_pool.state.token_x_mint.to_string(),
        token_y_mint: live_pool.state.token_y_mint.to_string(),
        token_x_symbol: tx_build::mint_symbol_placeholder(&live_pool.state.token_x_mint),
        token_y_symbol: tx_build::mint_symbol_placeholder(&live_pool.state.token_y_mint),
        lower_bin_id: req.lower_bin_id,
        width: req.width,
        ephemeral_position_pubkey: req.ephemeral_position_pubkey.clone(),
    });

    let response = BuildTxResponse {
        unsigned_transaction: built.bytes_b64,
        expiry_blockhash: built.blockhash.to_string(),
        expiry_last_valid_block_height: built.last_valid_block_height,
        idempotency_key: req.idempotency_key.clone(),
        simulation: built.simulation,
        estimated_network_fee_lamports: built.estimated_fee_lamports.to_string(),
        summary,
    };

    let final_response = tx_build::create_intent_idempotently(
        &state,
        tx_build::IntentInsert {
            wallet_address: wallet.pubkey.clone(),
            position_id: None,
            pool_address: req.pool_address.clone(),
            action: storage::types::intent_action::OPEN,
            idempotency_key: req.idempotency_key.clone(),
            request_json: to_json_value(&req)?,
            token_x_decimals: Some(live_pool.token_x_decimals),
            token_y_decimals: Some(live_pool.token_y_decimals),
            response,
            expires_at: expires_at(&state),
        },
    )
    .await?;

    tracing::info!(telegram_user_id = user.id, pool = %req.pool_address, action = "open", "Built transaction");
    Ok(Json(final_response))
}

pub async fn add_liquidity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AddLiquidityRequest>,
) -> Result<Json<BuildTxResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let wallet_pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    if req.strategy != "spot-balanced" {
        return Err(ApiError::refused(
            "unsupported_strategy",
            "Only the spot-balanced strategy is supported at launch",
        ));
    }

    risk::pool_risk_gate(&state, &req.pool_address).await?;

    let amount_x = tx_build::parse_amount_raw(&req.amount_x_raw, "amountXRaw")?;
    let amount_y = tx_build::parse_amount_raw(&req.amount_y_raw, "amountYRaw")?;
    risk::check_amount_cap(state.config.max_amount_raw, amount_x, amount_y)?;

    let position_row = storage::queries::position_by_address(&state.db, &req.position_address)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("No position {}", req.position_address)))?;
    if position_row.wallet_address != wallet.pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position does not belong to your registered wallet",
        ));
    }
    if position_row.pool_address != req.pool_address {
        return Err(ApiError::BadRequest(
            "positionAddress does not belong to poolAddress".to_string(),
        ));
    }
    if position_row.closed_at.is_some() {
        return Err(ApiError::refused(
            "position_closed",
            "This position is already closed",
        ));
    }

    let lb_pair = tx_build::parse_pubkey(&req.pool_address, "poolAddress")?;
    let position_pubkey = tx_build::parse_pubkey(&req.position_address, "positionAddress")?;

    let live_pool = rpc_ext::fetch_live_pool(&state.rpc, &lb_pair)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "pool_unavailable",
                "This pool's on-chain account could not be read",
            )
        })?;
    let live_position = rpc_ext::fetch_live_position(&state.rpc, &position_pubkey)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "position_unavailable",
                "This position's on-chain account could not be read",
            )
        })?;
    if live_position.owner != wallet_pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position's on-chain owner does not match your registered wallet",
        ));
    }

    let instructions = dlmm_tx::build_add_liquidity_by_strategy(
        &dlmm_tx::AddLiquidityByStrategyParams {
            lb_pair,
            position: position_pubkey,
            position_lower_bin_id: live_position.lower_bin_id,
            position_upper_bin_id: live_position.upper_bin_id,
            owner: wallet_pubkey,
            token_x_mint: live_pool.state.token_x_mint,
            token_y_mint: live_pool.state.token_y_mint,
            token_x_program: live_pool.token_x_program,
            token_y_program: live_pool.token_y_program,
            amount_x,
            amount_y,
            active_id: live_pool.state.active_bin_id,
            // Passed straight through, not converted from basis points -- see the report on
            // this task for why the miniapp's own verifier (`fromSummary.ts`) treats this field
            // as a raw bin-count value despite its "Bps" name, and why matching that wire
            // behaviour exactly (not a "corrected" unit conversion) is what actually round-trips
            // through the client's own signing check.
            max_active_bin_slippage: req.max_active_bin_slippage_bps,
            strategy_type: dlmm_tx::StrategyType::SpotBalanced,
            favor_token_x: false,
            min_bin_id: req.min_bin_id,
            max_bin_id: req.max_bin_id,
        },
        &compute_budget_config(&state),
    )
    .map_err(|e| ApiError::refused("invalid_liquidity_params", e.to_string()))?;

    let built = rpc_ext::assemble_and_simulate(
        &state.rpc,
        &wallet_pubkey,
        instructions,
        state.config.compute_unit_limit,
        state.config.compute_unit_price_micro_lamports,
    )
    .await?;

    let summary = TxSummary::AddLiquidity(AddLiquiditySummary {
        pool_address: req.pool_address.clone(),
        position_address: req.position_address.clone(),
        position_lower_bin_id: live_position.lower_bin_id,
        position_upper_bin_id: live_position.upper_bin_id,
        token_x_mint: live_pool.state.token_x_mint.to_string(),
        token_y_mint: live_pool.state.token_y_mint.to_string(),
        token_x_program: live_pool.token_x_program.to_string(),
        token_y_program: live_pool.token_y_program.to_string(),
        token_x_symbol: tx_build::mint_symbol_placeholder(&live_pool.state.token_x_mint),
        token_y_symbol: tx_build::mint_symbol_placeholder(&live_pool.state.token_y_mint),
        amount_x_raw: amount_x.to_string(),
        amount_y_raw: amount_y.to_string(),
        // No live price oracle is wired into this service -- the miniapp contract already
        // allows null here for exactly that reason.
        amount_x_usd: None,
        amount_y_usd: None,
        active_id: live_pool.state.active_bin_id,
        max_active_bin_slippage_bps: req.max_active_bin_slippage_bps,
        min_bin_id: req.min_bin_id,
        max_bin_id: req.max_bin_id,
    });

    let response = BuildTxResponse {
        unsigned_transaction: built.bytes_b64,
        expiry_blockhash: built.blockhash.to_string(),
        expiry_last_valid_block_height: built.last_valid_block_height,
        idempotency_key: req.idempotency_key.clone(),
        simulation: built.simulation,
        estimated_network_fee_lamports: built.estimated_fee_lamports.to_string(),
        summary,
    };

    let final_response = tx_build::create_intent_idempotently(
        &state,
        tx_build::IntentInsert {
            wallet_address: wallet.pubkey.clone(),
            position_id: Some(position_row.id),
            pool_address: req.pool_address.clone(),
            action: storage::types::intent_action::ADD,
            idempotency_key: req.idempotency_key.clone(),
            request_json: to_json_value(&req)?,
            token_x_decimals: Some(live_pool.token_x_decimals),
            token_y_decimals: Some(live_pool.token_y_decimals),
            response,
            expires_at: expires_at(&state),
        },
    )
    .await?;

    tracing::info!(telegram_user_id = user.id, pool = %req.pool_address, action = "add", "Built transaction");
    Ok(Json(final_response))
}

pub async fn remove_liquidity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RemoveLiquidityRequest>,
) -> Result<Json<BuildTxResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let wallet_pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    let position_row = storage::queries::position_by_address(&state.db, &req.position_address)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("No position {}", req.position_address)))?;
    if position_row.wallet_address != wallet.pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position does not belong to your registered wallet",
        ));
    }
    if position_row.pool_address != req.pool_address {
        return Err(ApiError::BadRequest(
            "positionAddress does not belong to poolAddress".to_string(),
        ));
    }
    if position_row.closed_at.is_some() {
        return Err(ApiError::refused(
            "position_closed",
            "This position is already closed",
        ));
    }

    let lb_pair = tx_build::parse_pubkey(&req.pool_address, "poolAddress")?;
    let position_pubkey = tx_build::parse_pubkey(&req.position_address, "positionAddress")?;

    let live_pool = rpc_ext::fetch_live_pool(&state.rpc, &lb_pair)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "pool_unavailable",
                "This pool's on-chain account could not be read",
            )
        })?;
    let live_position = rpc_ext::fetch_live_position(&state.rpc, &position_pubkey)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "position_unavailable",
                "This position's on-chain account could not be read",
            )
        })?;
    if live_position.owner != wallet_pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position's on-chain owner does not match your registered wallet",
        ));
    }

    let instructions = dlmm_tx::build_remove_liquidity_by_range(
        &dlmm_tx::RemoveLiquidityByRangeParams {
            lb_pair,
            position: position_pubkey,
            position_lower_bin_id: live_position.lower_bin_id,
            position_upper_bin_id: live_position.upper_bin_id,
            owner: wallet_pubkey,
            token_x_mint: live_pool.state.token_x_mint,
            token_y_mint: live_pool.state.token_y_mint,
            token_x_program: live_pool.token_x_program,
            token_y_program: live_pool.token_y_program,
            from_bin_id: req.from_bin_id,
            to_bin_id: req.to_bin_id,
            bps_to_remove: req.bps_to_remove,
        },
        &compute_budget_config(&state),
    )
    .map_err(|e| ApiError::refused("invalid_liquidity_params", e.to_string()))?;

    let built = rpc_ext::assemble_and_simulate(
        &state.rpc,
        &wallet_pubkey,
        instructions,
        state.config.compute_unit_limit,
        state.config.compute_unit_price_micro_lamports,
    )
    .await?;

    let summary = TxSummary::RemoveLiquidity(RemoveLiquiditySummary {
        pool_address: req.pool_address.clone(),
        position_address: req.position_address.clone(),
        position_lower_bin_id: live_position.lower_bin_id,
        position_upper_bin_id: live_position.upper_bin_id,
        token_x_mint: live_pool.state.token_x_mint.to_string(),
        token_y_mint: live_pool.state.token_y_mint.to_string(),
        token_x_program: live_pool.token_x_program.to_string(),
        token_y_program: live_pool.token_y_program.to_string(),
        from_bin_id: req.from_bin_id,
        to_bin_id: req.to_bin_id,
        bps_to_remove: req.bps_to_remove,
    });

    let response = BuildTxResponse {
        unsigned_transaction: built.bytes_b64,
        expiry_blockhash: built.blockhash.to_string(),
        expiry_last_valid_block_height: built.last_valid_block_height,
        idempotency_key: req.idempotency_key.clone(),
        simulation: built.simulation,
        estimated_network_fee_lamports: built.estimated_fee_lamports.to_string(),
        summary,
    };

    let final_response = tx_build::create_intent_idempotently(
        &state,
        tx_build::IntentInsert {
            wallet_address: wallet.pubkey.clone(),
            position_id: Some(position_row.id),
            pool_address: req.pool_address.clone(),
            action: storage::types::intent_action::REMOVE,
            idempotency_key: req.idempotency_key.clone(),
            request_json: to_json_value(&req)?,
            token_x_decimals: Some(live_pool.token_x_decimals),
            token_y_decimals: Some(live_pool.token_y_decimals),
            response,
            expires_at: expires_at(&state),
        },
    )
    .await?;

    tracing::info!(telegram_user_id = user.id, pool = %req.pool_address, action = "remove", "Built transaction");
    Ok(Json(final_response))
}

pub async fn claim_fees(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ClaimFeesRequest>,
) -> Result<Json<BuildTxResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let wallet_pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    let position_row = storage::queries::position_by_address(&state.db, &req.position_address)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("No position {}", req.position_address)))?;
    if position_row.wallet_address != wallet.pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position does not belong to your registered wallet",
        ));
    }
    if position_row.pool_address != req.pool_address {
        return Err(ApiError::BadRequest(
            "positionAddress does not belong to poolAddress".to_string(),
        ));
    }
    if position_row.closed_at.is_some() {
        return Err(ApiError::refused(
            "position_closed",
            "This position is already closed",
        ));
    }

    let lb_pair = tx_build::parse_pubkey(&req.pool_address, "poolAddress")?;
    let position_pubkey = tx_build::parse_pubkey(&req.position_address, "positionAddress")?;

    let live_pool = rpc_ext::fetch_live_pool(&state.rpc, &lb_pair)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "pool_unavailable",
                "This pool's on-chain account could not be read",
            )
        })?;
    let live_position = rpc_ext::fetch_live_position(&state.rpc, &position_pubkey)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "position_unavailable",
                "This position's on-chain account could not be read",
            )
        })?;
    if live_position.owner != wallet_pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position's on-chain owner does not match your registered wallet",
        ));
    }

    let (estimated_fees_x, estimated_fees_y) =
        rpc_ext::pending_fees_in_range(&live_position, req.min_bin_id, req.max_bin_id);

    let instructions = dlmm_tx::build_claim_fee(
        &dlmm_tx::ClaimFeeParams {
            lb_pair,
            position: position_pubkey,
            position_lower_bin_id: live_position.lower_bin_id,
            position_upper_bin_id: live_position.upper_bin_id,
            owner: wallet_pubkey,
            token_x_mint: live_pool.state.token_x_mint,
            token_y_mint: live_pool.state.token_y_mint,
            token_x_program: live_pool.token_x_program,
            token_y_program: live_pool.token_y_program,
            min_bin_id: req.min_bin_id,
            max_bin_id: req.max_bin_id,
        },
        &compute_budget_config(&state),
    )
    .map_err(|e| ApiError::refused("invalid_liquidity_params", e.to_string()))?;

    let built = rpc_ext::assemble_and_simulate(
        &state.rpc,
        &wallet_pubkey,
        instructions,
        state.config.compute_unit_limit,
        state.config.compute_unit_price_micro_lamports,
    )
    .await?;

    let summary = TxSummary::ClaimFees(ClaimFeesSummary {
        pool_address: req.pool_address.clone(),
        position_address: req.position_address.clone(),
        position_lower_bin_id: live_position.lower_bin_id,
        position_upper_bin_id: live_position.upper_bin_id,
        token_x_mint: live_pool.state.token_x_mint.to_string(),
        token_y_mint: live_pool.state.token_y_mint.to_string(),
        token_x_program: live_pool.token_x_program.to_string(),
        token_y_program: live_pool.token_y_program.to_string(),
        min_bin_id: req.min_bin_id,
        max_bin_id: req.max_bin_id,
        estimated_fees_x_raw: estimated_fees_x.to_string(),
        estimated_fees_y_raw: estimated_fees_y.to_string(),
    });

    let response = BuildTxResponse {
        unsigned_transaction: built.bytes_b64,
        expiry_blockhash: built.blockhash.to_string(),
        expiry_last_valid_block_height: built.last_valid_block_height,
        idempotency_key: req.idempotency_key.clone(),
        simulation: built.simulation,
        estimated_network_fee_lamports: built.estimated_fee_lamports.to_string(),
        summary,
    };

    let final_response = tx_build::create_intent_idempotently(
        &state,
        tx_build::IntentInsert {
            wallet_address: wallet.pubkey.clone(),
            position_id: Some(position_row.id),
            pool_address: req.pool_address.clone(),
            action: storage::types::intent_action::CLAIM,
            idempotency_key: req.idempotency_key.clone(),
            request_json: to_json_value(&req)?,
            token_x_decimals: Some(live_pool.token_x_decimals),
            token_y_decimals: Some(live_pool.token_y_decimals),
            response,
            expires_at: expires_at(&state),
        },
    )
    .await?;

    tracing::info!(telegram_user_id = user.id, pool = %req.pool_address, action = "claim", "Built transaction");
    Ok(Json(final_response))
}

pub async fn close_position(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ClosePositionRequest>,
) -> Result<Json<BuildTxResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;
    let wallet_pubkey = tx_build::parse_pubkey(&wallet.pubkey, "wallet")?;

    let position_row = storage::queries::position_by_address(&state.db, &req.position_address)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("No position {}", req.position_address)))?;
    if position_row.wallet_address != wallet.pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position does not belong to your registered wallet",
        ));
    }
    if position_row.closed_at.is_some() {
        return Err(ApiError::refused(
            "position_closed",
            "This position is already closed",
        ));
    }

    let position_pubkey = tx_build::parse_pubkey(&req.position_address, "positionAddress")?;
    let live_position = rpc_ext::fetch_live_position(&state.rpc, &position_pubkey)
        .await?
        .ok_or_else(|| {
            ApiError::refused(
                "position_unavailable",
                "This position's on-chain account could not be read",
            )
        })?;
    if live_position.owner != wallet_pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position's on-chain owner does not match your registered wallet",
        ));
    }

    // close_position2 does not itself check for zero liquidity/fees (see dlmm_tx's own doc
    // comment on build_close_position) -- if this position still holds either, the simulation
    // below catches it and reports it in `simulation`, the same as any other on-chain refusal.
    let instructions = dlmm_tx::build_close_position(
        &dlmm_tx::ClosePositionParams {
            position: position_pubkey,
            owner: wallet_pubkey,
            rent_receiver: wallet_pubkey,
        },
        &compute_budget_config(&state),
    );

    let built = rpc_ext::assemble_and_simulate(
        &state.rpc,
        &wallet_pubkey,
        instructions,
        state.config.compute_unit_limit,
        state.config.compute_unit_price_micro_lamports,
    )
    .await?;

    let summary = TxSummary::ClosePosition(ClosePositionSummary {
        position_address: req.position_address.clone(),
        rent_receiver: wallet.pubkey.clone(),
    });

    let response = BuildTxResponse {
        unsigned_transaction: built.bytes_b64,
        expiry_blockhash: built.blockhash.to_string(),
        expiry_last_valid_block_height: built.last_valid_block_height,
        idempotency_key: req.idempotency_key.clone(),
        simulation: built.simulation,
        estimated_network_fee_lamports: built.estimated_fee_lamports.to_string(),
        summary,
    };

    let final_response = tx_build::create_intent_idempotently(
        &state,
        tx_build::IntentInsert {
            wallet_address: wallet.pubkey.clone(),
            position_id: Some(position_row.id),
            pool_address: position_row.pool_address.clone(),
            action: storage::types::intent_action::CLOSE,
            idempotency_key: req.idempotency_key.clone(),
            request_json: to_json_value(&req)?,
            token_x_decimals: None,
            token_y_decimals: None,
            response,
            expires_at: expires_at(&state),
        },
    )
    .await?;

    tracing::info!(telegram_user_id = user.id, position = %req.position_address, action = "close", "Built transaction");
    Ok(Json(final_response))
}
