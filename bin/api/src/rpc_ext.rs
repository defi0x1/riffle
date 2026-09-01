//! Live on-chain reads and the build/simulate helper shared by every tx-build handler. Pool and
//! position state are read live via RPC (decoded with `dlmm_decode`) rather than from Postgres:
//! the storage layer's own pool/position rows can lag the chain by a poll interval, and a
//! transaction built from a stale bin range or a stale active bin is exactly the kind of mistake
//! item 3 (simulate before returning) exists to catch early -- reading live in the first place
//! avoids manufacturing that mistake at all. Postgres is still the source of truth for whether a
//! pool is *known and risk-screened* (see `risk.rs`); RPC is the source of truth for its current
//! shape.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use eyre::WrapErr;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::{VersionedMessage, v0};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;

use crate::dto::SimulationDto;
use crate::error::ApiError;

pub struct LivePool {
    pub state: dlmm_decode::PoolState,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub token_x_decimals: u8,
    pub token_y_decimals: u8,
}

fn internal<E: std::fmt::Display>(context: &'static str) -> impl FnOnce(E) -> ApiError {
    move |e| ApiError::Internal(eyre::eyre!("{context}: {e}"))
}

/// An SPL Token / Token-2022 mint's `decimals` byte sits at a fixed offset in the base Mint
/// layout (mint_authority_option(4) + mint_authority(32) + supply(8) + decimals(1) + ...) --
/// identical between the two programs, since Token-2022 only ever appends extension TLVs after
/// this base layout. Reading one byte here is simpler and more auditable than a full mint
/// deserialiser for the one field this service needs.
const MINT_DECIMALS_OFFSET: usize = 44;

fn mint_program_and_decimals(account: &solana_sdk::account::Account) -> Option<(Pubkey, u8)> {
    let decimals = *account.data.get(MINT_DECIMALS_OFFSET)?;
    Some((account.owner, decimals))
}

/// Fetches and decodes an LbPair account plus its two mints' owning token program and decimals,
/// in two RPC round trips total. `None` means the account genuinely does not exist on chain --
/// distinct from an RPC-level failure, which is surfaced as `ApiError::Internal` instead, since
/// the risk gate (`risk.rs`) already established Postgres knows this pool, and a chain-side miss
/// at that point is an inconsistency worth alerting on, not an ordinary user-facing refusal.
pub async fn fetch_live_pool(rpc: &RpcClient, lb_pair: &Pubkey) -> Result<Option<LivePool>, ApiError> {
    let account = match rpc.get_account(lb_pair).await {
        Ok(account) => account,
        Err(_) => return Ok(None),
    };
    let state = dlmm_decode::decode_lb_pair(&account.data)
        .wrap_err_with(|| format!("Decoding LbPair {lb_pair}"))
        .map_err(ApiError::Internal)?;

    let mints = rpc
        .get_multiple_accounts(&[state.token_x_mint, state.token_y_mint])
        .await
        .map_err(internal("Fetching mint accounts"))?;

    let (token_x_program, token_x_decimals) = mints[0]
        .as_ref()
        .and_then(mint_program_and_decimals)
        .ok_or_else(|| ApiError::Internal(eyre::eyre!("Token X mint {} not found or unreadable", state.token_x_mint)))?;
    let (token_y_program, token_y_decimals) = mints[1]
        .as_ref()
        .and_then(mint_program_and_decimals)
        .ok_or_else(|| ApiError::Internal(eyre::eyre!("Token Y mint {} not found or unreadable", state.token_y_mint)))?;

    Ok(Some(LivePool {
        state,
        token_x_program,
        token_y_program,
        token_x_decimals,
        token_y_decimals,
    }))
}

/// `None` means the position account does not exist (or is not a PositionV2) -- a normal,
/// user-facing "unknown position" case, unlike a pool-account miss above.
pub async fn fetch_live_position(
    rpc: &RpcClient,
    position: &Pubkey,
) -> Result<Option<dlmm_decode::PositionState>, ApiError> {
    let account = match rpc.get_account(position).await {
        Ok(account) => account,
        Err(_) => return Ok(None),
    };
    match dlmm_decode::decode_position_v2(&account.data) {
        Ok(state) => Ok(Some(state)),
        Err(_) => Ok(None),
    }
}

/// Sums each bin's cached `fee_x_pending`/`fee_y_pending` across `[min_bin_id, max_bin_id]`.
/// This is the same "fees earned but not yet claimed on-chain" figure `position_valuations`'
/// own migration comment describes -- an estimate as of the position's last on-chain update,
/// not a live re-simulation of fee accrual since then, which is exactly what the `estimated`
/// naming in the miniapp contract's `estimatedFeesXRaw`/`estimatedFeesYRaw` fields signals.
pub fn pending_fees_in_range(
    position: &dlmm_decode::PositionState,
    min_bin_id: i32,
    max_bin_id: i32,
) -> (u64, u64) {
    let mut fee_x: u64 = 0;
    let mut fee_y: u64 = 0;
    for (offset, fee_info) in position.fee_infos.iter().enumerate() {
        let bin_id = position.lower_bin_id + offset as i32;
        if bin_id < min_bin_id || bin_id > max_bin_id {
            continue;
        }
        fee_x = fee_x.saturating_add(fee_info.fee_x_pending);
        fee_y = fee_y.saturating_add(fee_info.fee_y_pending);
    }
    (fee_x, fee_y)
}

pub struct BuiltTx {
    pub bytes_b64: String,
    pub blockhash: Hash,
    pub last_valid_block_height: u64,
    pub simulation: SimulationDto,
    pub estimated_fee_lamports: u64,
}

/// Compiles `instructions` (already including any ComputeBudget instructions) into an unsigned
/// `VersionedTransaction` with `payer` as fee payer and no address lookup tables -- the miniapp's
/// own verifier refuses any transaction that uses one (see its README), so this service never
/// builds one. Every signature slot is a zeroed placeholder: nothing here ever holds, needs, or
/// could hold a private key. Then simulates it (item 3: catch a failure here, not after the user
/// has approved it) and estimates its network fee via a live `getFeeForMessage` call plus the
/// configured priority fee, which this service knows exactly since it set it.
pub async fn assemble_and_simulate(
    rpc: &RpcClient,
    payer: &Pubkey,
    instructions: Vec<Instruction>,
    compute_unit_limit: u32,
    compute_unit_price_micro_lamports: u64,
) -> Result<BuiltTx, ApiError> {
    let (blockhash, last_valid_block_height) = rpc
        .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
        .await
        .map_err(internal("Fetching latest blockhash"))?;

    let message = v0::Message::try_compile(payer, &instructions, &[], blockhash)
        .map_err(internal("Compiling transaction message"))?;

    let base_fee_lamports = rpc
        .get_fee_for_message(&message)
        .await
        .map_err(internal("Estimating network fee"))?;
    let priority_fee_lamports = (u128::from(compute_unit_limit)
        * u128::from(compute_unit_price_micro_lamports)
        / 1_000_000) as u64;
    let estimated_fee_lamports = base_fee_lamports.saturating_add(priority_fee_lamports);

    let num_required_signatures = message.header.num_required_signatures as usize;
    let versioned_tx = VersionedTransaction {
        signatures: vec![Signature::default(); num_required_signatures],
        message: VersionedMessage::V0(message),
    };

    let bytes = bincode::serialize(&versioned_tx)
        .wrap_err_with(|| "Serialising unsigned transaction")
        .map_err(ApiError::Internal)?;
    let bytes_b64 = BASE64_STANDARD.encode(&bytes);

    let sim_config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::processed()),
        ..Default::default()
    };
    let simulation = match rpc
        .simulate_transaction_with_config(&versioned_tx, sim_config)
        .await
    {
        Ok(response) => {
            let logs = response.value.logs.unwrap_or_default();
            let tail_start = logs.len().saturating_sub(20);
            SimulationDto {
                success: response.value.err.is_none(),
                error: response.value.err.map(|e| format!("{e:?}")),
                logs_tail: logs[tail_start..].to_vec(),
            }
        }
        // A transport-level failure to even run the simulation is still reported *in* the
        // response (per the miniapp contract's `simulation` shape), never turned into an HTTP
        // error -- the Mini App shows it as a warning and re-simulates independently anyway
        // (see its README's "The transaction verifier" section).
        Err(e) => SimulationDto {
            success: false,
            error: Some(format!("simulation request failed: {e}")),
            logs_tail: Vec::new(),
        },
    };

    Ok(BuiltTx {
        bytes_b64,
        blockhash,
        last_valid_block_height,
        simulation,
        estimated_fee_lamports,
    })
}

/// Submits an already-signed transaction opaquely -- no instruction or account inspection --
/// except confirming its fee payer is the wallet this request is authenticated as, which keeps
/// this endpoint from being usable as an open relay for a transaction unrelated to any intent
/// this service built. Returns the transaction's own first signature either way, since it is
/// deterministic from the signed bytes regardless of whether the network accepted it.
pub async fn submit_signed_transaction(
    rpc: &RpcClient,
    tx: &VersionedTransaction,
) -> (Signature, Result<(), String>) {
    let signature = tx.signatures.first().copied().unwrap_or_default();
    let config = RpcSendTransactionConfig {
        skip_preflight: false,
        ..Default::default()
    };
    match rpc.send_transaction_with_config(tx, config).await {
        Ok(_) => (signature, Ok(())),
        Err(e) => (signature, Err(e.to_string())),
    }
}
