//! Request/response shapes, mirroring `miniapp/src/api/types.ts` field for field. Every name
//! here is deliberately the snake_case spelling of the exact camelCase field the Mini App reads
//! or sends -- `#[serde(rename_all = "camelCase")]` does the rest. Do not add a field the
//! contract does not have; do not rename one it does.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterWalletRequest {
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterWalletResponse {
    pub registered_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBalance {
    pub mint: String,
    pub program_id: String,
    pub amount_raw: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancesResponse {
    pub sol_lamports: String,
    pub tokens: Vec<TokenBalance>,
}

// `Closed` is part of the contract's `PositionStatus` union (`miniapp/src/api/types.ts`) but
// unused by this pass: `routes::positions` only ever serves open positions, since the storage
// layer exposes no "recently closed positions for wallet" query -- see the report on this task.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum PositionStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionSummary {
    pub position_address: String,
    pub pool_address: String,
    pub status: PositionStatus,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub opened_at: String,
    pub closed_at: Option<String>,
    pub fees_x_pending: String,
    pub fees_y_pending: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionsResponse {
    pub positions: Vec<PositionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationDto {
    pub success: bool,
    pub error: Option<String>,
    pub logs_tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPositionSummary {
    pub pool_address: String,
    pub token_x_mint: String,
    pub token_y_mint: String,
    pub token_x_symbol: String,
    pub token_y_symbol: String,
    pub lower_bin_id: i32,
    pub width: i32,
    pub ephemeral_position_pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLiquiditySummary {
    pub pool_address: String,
    pub position_address: String,
    pub position_lower_bin_id: i32,
    pub position_upper_bin_id: i32,
    pub token_x_mint: String,
    pub token_y_mint: String,
    pub token_x_program: String,
    pub token_y_program: String,
    pub token_x_symbol: String,
    pub token_y_symbol: String,
    pub amount_x_raw: String,
    pub amount_y_raw: String,
    pub amount_x_usd: Option<f64>,
    pub amount_y_usd: Option<f64>,
    pub active_id: i32,
    pub max_active_bin_slippage_bps: i32,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveLiquiditySummary {
    pub pool_address: String,
    pub position_address: String,
    pub position_lower_bin_id: i32,
    pub position_upper_bin_id: i32,
    pub token_x_mint: String,
    pub token_y_mint: String,
    pub token_x_program: String,
    pub token_y_program: String,
    pub from_bin_id: i32,
    pub to_bin_id: i32,
    pub bps_to_remove: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimFeesSummary {
    pub pool_address: String,
    pub position_address: String,
    pub position_lower_bin_id: i32,
    pub position_upper_bin_id: i32,
    pub token_x_mint: String,
    pub token_y_mint: String,
    pub token_x_program: String,
    pub token_y_program: String,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub estimated_fees_x_raw: String,
    pub estimated_fees_y_raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosePositionSummary {
    pub position_address: String,
    pub rent_receiver: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum TxSummary {
    #[serde(rename = "open-position")]
    OpenPosition(OpenPositionSummary),
    #[serde(rename = "add-liquidity")]
    AddLiquidity(AddLiquiditySummary),
    #[serde(rename = "remove-liquidity")]
    RemoveLiquidity(RemoveLiquiditySummary),
    #[serde(rename = "claim-fees")]
    ClaimFees(ClaimFeesSummary),
    #[serde(rename = "close-position")]
    ClosePosition(ClosePositionSummary),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildTxResponse {
    pub unsigned_transaction: String,
    pub expiry_blockhash: String,
    pub expiry_last_valid_block_height: u64,
    pub idempotency_key: String,
    pub simulation: SimulationDto,
    pub estimated_network_fee_lamports: String,
    pub summary: TxSummary,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPositionRequest {
    pub pool_address: String,
    pub lower_bin_id: i32,
    pub width: i32,
    pub ephemeral_position_pubkey: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLiquidityRequest {
    pub pool_address: String,
    pub position_address: String,
    pub amount_x_raw: String,
    pub amount_y_raw: String,
    pub max_active_bin_slippage_bps: i32,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub strategy: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveLiquidityRequest {
    pub pool_address: String,
    pub position_address: String,
    pub from_bin_id: i32,
    pub to_bin_id: i32,
    pub bps_to_remove: u16,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimFeesRequest {
    pub pool_address: String,
    pub position_address: String,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosePositionRequest {
    pub position_address: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitTxRequest {
    pub signed_transaction: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TxStatus {
    Submitted,
    Confirmed,
    Failed,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitTxResponse {
    pub signature: String,
    pub status: TxStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxStatusQuery {
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxStatusResponse {
    pub signature: String,
    pub status: TxStatus,
    pub error: Option<String>,
}

/// Everything a build-tx handler needs to reconstruct its `BuildTxResponse` on an idempotent
/// replay without touching RPC again. Stored in `transaction_intents.params`; `request` is the
/// original request body (kept for the same reason the column's own migration comment gives:
/// "so an interrupted flow can be resumed without asking the user to re-enter what they asked
/// for"), the decimals are cached from the one live mint read the first build already did, and
/// `response` is the exact `BuildTxResponse` returned the first time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentParams {
    pub request: serde_json::Value,
    #[serde(default)]
    pub token_x_decimals: Option<u8>,
    #[serde(default)]
    pub token_y_decimals: Option<u8>,
    pub response: BuildTxResponse,
}

/// Not part of the miniapp's documented contract (see the report on this task for why) --
/// derived strictly from `position_cash_flows` and `position_valuations`, the definition the
/// storage layer's own migration comments already establish, never a second one computed here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfitResponse {
    pub position_address: String,
    pub deposited_usd: Option<String>,
    pub withdrawn_usd: Option<String>,
    pub fees_claimed_usd: Option<String>,
    pub current_value_usd: Option<String>,
    pub profit_usd: Option<String>,
    pub as_of: Option<String>,
}
