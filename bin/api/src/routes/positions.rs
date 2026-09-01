use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use rust_decimal::Decimal;

use crate::dto::{PositionStatus, PositionSummary, PositionsResponse, ProfitResponse};
use crate::error::ApiError;
use crate::state::AppState;
use crate::{rpc_ext, tx_build, wallet_resolve};

/// Open positions for the caller's own wallet. The miniapp contract also asks for "recently
/// closed" positions in the same response -- the storage layer exposes no query for that (only
/// `open_positions_for_wallet`, filtered to `closed_at IS NULL`, and `position_by_address` for a
/// single known address), so this list is open positions only. See the report on this task.
pub async fn positions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PositionsResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;

    let rows = storage::queries::open_positions_for_wallet(&state.db, &wallet.pubkey)
        .await
        .map_err(ApiError::Internal)?;

    let mut positions = Vec::with_capacity(rows.len());
    for row in rows {
        let fees = match tx_build::parse_pubkey(&row.position_address, "positionAddress") {
            Ok(position_pubkey) => match rpc_ext::fetch_live_position(&state.rpc, &position_pubkey).await {
                Ok(Some(live)) => rpc_ext::pending_fees_in_range(&live, row.lower_bin, row.upper_bin),
                Ok(None) => {
                    tracing::warn!(position = %row.position_address, "Position not found on chain while listing positions");
                    (0, 0)
                }
                Err(e) => {
                    tracing::warn!(position = %row.position_address, error = ?e, "Failed to read live position state");
                    (0, 0)
                }
            },
            Err(_) => (0, 0),
        };

        positions.push(PositionSummary {
            position_address: row.position_address,
            pool_address: row.pool_address,
            status: PositionStatus::Open,
            lower_bin_id: row.lower_bin,
            upper_bin_id: row.upper_bin,
            opened_at: row.opened_at.to_rfc3339(),
            closed_at: row.closed_at.map(|d| d.to_rfc3339()),
            fees_x_pending: fees.0.to_string(),
            fees_y_pending: fees.1.to_string(),
        });
    }

    Ok(Json(PositionsResponse { positions }))
}

/// Not part of the miniapp's documented contract -- see the report on this task. Profit is
/// derived strictly from `position_cash_flows` and `position_valuations`, the definition the
/// storage layer's own migration comments already establish (0031: "Profit is never stored as a
/// bare number anywhere in this schema; it is always computed from these rows plus a
/// position_valuations mark"), not a second definition computed here.
pub async fn profit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(position_address): Path<String>,
) -> Result<Json<ProfitResponse>, ApiError> {
    let user = super::authenticate(&headers, &state).await?;
    let wallet = wallet_resolve::resolve_wallet(&state, user.id).await?;

    let position = storage::queries::position_by_address(&state.db, &position_address)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("No position {position_address}")))?;
    if position.wallet_address != wallet.pubkey {
        return Err(ApiError::refused(
            "position_not_owned",
            "This position does not belong to your registered wallet",
        ));
    }

    let cash_flows = storage::queries::cash_flows_for_position(&state.db, position.id)
        .await
        .map_err(ApiError::Internal)?;
    let valuation = storage::queries::latest_position_valuation(&state.db, position.id)
        .await
        .map_err(ApiError::Internal)?;

    let mut deposited = Decimal::ZERO;
    let mut withdrawn = Decimal::ZERO;
    let mut fees_claimed = Decimal::ZERO;
    for cf in &cash_flows {
        let value = cf.value_usd.unwrap_or(Decimal::ZERO);
        if cf.kind == storage::types::cash_flow_kind::DEPOSIT {
            deposited += value;
        } else if cf.kind == storage::types::cash_flow_kind::WITHDRAWAL {
            withdrawn += value;
        } else if cf.kind == storage::types::cash_flow_kind::FEE_CLAIM {
            fees_claimed += value;
        }
    }

    let current_value = valuation.as_ref().and_then(|v| v.value_usd);
    let as_of = valuation.as_ref().map(|v| v.ts.to_rfc3339());
    let profit = current_value.map(|cv| cv + withdrawn + fees_claimed - deposited);

    Ok(Json(ProfitResponse {
        position_address,
        deposited_usd: Some(deposited.to_string()),
        withdrawn_usd: Some(withdrawn.to_string()),
        fees_claimed_usd: Some(fees_claimed.to_string()),
        current_value_usd: current_value.map(|v| v.to_string()),
        profit_usd: profit.map(|v| v.to_string()),
        as_of,
    }))
}
