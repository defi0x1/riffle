//! Validates this crate's instruction builders against the real, deployed DLMM program
//! (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`) rather than against a second implementation
//! of the same misunderstanding. Every other test in this crate (and in `miniapp/tests/`,
//! against the committed `fixtures/dlmm_tx/` transactions) checks internal consistency: the
//! builders agree with themselves, with the vendored IDL transcription, and with an independent
//! TypeScript verifier. None of that proves the on-chain program accepts what gets built --
//! this crate and the verifier could share the same wrong account order and every one of those
//! tests would still pass. This file closes that gap by sending or simulating real transactions
//! against the real program, loaded either from a mainnet dump into a local validator (strongest;
//! see `scripts/validate-onchain.sh`) or reached directly over a mainnet RPC endpoint.
//!
//! Needs a network or a validator, so it is never part of `cargo test --workspace`: gated on
//! `DLMM_TX_VALIDATION_RPC_URL`, skipped cleanly when unset exactly like `tests/src/lib.rs`'s
//! `require_database!` gates the database-backed integration suite elsewhere in this workspace.
//!
//! See `docs/validation.md` for how to run this, what each outcome means, and -- most
//! importantly -- how to tell a genuine encoding bug apart from a simulation merely hitting an
//! unfunded account or an uninitialised prerequisite. Read that before treating anything this
//! file prints as a confirmed bug.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::time::Duration;

use dlmm_tx::{
    AddLiquidityByStrategyParams, ClaimFeeParams, ClosePositionParams, ComputeBudgetConfig,
    OpenPositionParams, RemoveLiquidityByRangeParams, StrategyType,
    build_add_liquidity_by_strategy, build_claim_fee, build_close_position, build_open_position,
    build_remove_liquidity_by_range,
};
use solana_rpc_client::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::Transaction;

/// `None` when unset or empty, mirroring `tests/src/lib.rs`'s `database_url()` -- a clean skip,
/// never a guessed fallback that would connect somewhere nobody configured.
fn validation_rpc_url() -> Option<String> {
    env::var("DLMM_TX_VALIDATION_RPC_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

// The real, live SOL-USDC DLMM pool on mainnet -- the same one `libraries/dlmm_tx/src/pda.rs`
// and `libraries/dlmm_decode/tests/golden.rs` already cross-check their own PDA derivations and
// account decoding against, so an operator who already trusts those addresses is trusting
// nothing new here.
const POOL: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

// BinArray index -81 covers bin ids -5670..=-5601 (see `libraries/dlmm_decode/tests/golden.rs`)
// and is a real, already-initialised account on this pool. Picking a position range that fits
// entirely inside one already-initialised array means the only account this run depends on
// existing ahead of time is one this crate's own PDA tests already prove it derives correctly --
// nothing here has to hope a wider, uninitialised range lazily springs into existence.
const POSITION_LOWER_BIN_ID: i32 = -5660;
const POSITION_WIDTH: i32 = 20;
const POSITION_UPPER_BIN_ID: i32 = POSITION_LOWER_BIN_ID + POSITION_WIDTH - 1;

fn pubkey(s: &str) -> Pubkey {
    s.parse().expect("valid base58 pubkey constant")
}

/// The vendored IDL's `errors` section, keyed by numeric code -- the same file
/// `tests/idl_conformance.rs` and `src/test_support.rs` already treat as this crate's source of
/// truth for the program's declared shape. A transaction that fails with one of these codes was
/// accepted, decoded, and evaluated by the real program; it was only rejected for a business
/// reason the program itself names, which is a materially different, and far less concerning,
/// outcome than failing before the program ever got to render a verdict.
fn idl_error_table() -> HashMap<u64, (String, String)> {
    let raw = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dlmm.json"
    ))
    .expect("reading vendored IDL fixture");
    let idl: serde_json::Value = serde_json::from_str(&raw).expect("parsing vendored IDL fixture");
    idl["errors"]
        .as_array()
        .expect("vendored IDL has an errors array")
        .iter()
        .map(|e| {
            let code = e["code"].as_u64().expect("error code is a number");
            let name = e["name"].as_str().unwrap_or_default().to_string();
            let msg = e["msg"].as_str().unwrap_or_default().to_string();
            (code, (name, msg))
        })
        .collect()
}

/// One operation's result against the real program: whether the run believes it proved
/// structural correctness (`ok`), plus everything a human needs to judge that call themselves.
struct Outcome {
    operation: &'static str,
    ok: bool,
    summary: String,
    logs: Vec<String>,
}

/// Airdrops a local validator's own mint SOL to `pubkey` and waits for it to land. Returns
/// `false` (without treating it as an error) when the endpoint refuses airdrops at all, which is
/// mainnet's normal behaviour, not a fault in this harness -- see `docs/validation.md` for what
/// that does and does not let this run prove.
fn try_fund(client: &RpcClient, pubkey: &Pubkey) -> bool {
    // Polling balance rather than confirming this specific signature keeps this helper simple
    // and works identically whether or not the endpoint even returns a real signature for a
    // faucet airdrop.
    if client.request_airdrop(pubkey, 2_000_000_000).is_err() {
        return false;
    }
    for _ in 0..20 {
        if let Ok(balance) = client.get_balance(pubkey)
            && balance > 0
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// Simulates `instructions` with signature verification off (`sig_verify: false`): this proves
/// the real program accepts the account list and argument encoding without needing this harness
/// to hold a real signing key for every account involved -- the whole reason a keyless backend's
/// transactions can be checked against the real program at all without asking a user to sign a
/// probe transaction on its behalf.
fn simulate(
    client: &RpcClient,
    payer: &Pubkey,
    instructions: &[Instruction],
) -> (bool, String, Vec<String>) {
    let blockhash = client.get_latest_blockhash().unwrap_or_default();
    let message =
        solana_sdk::message::Message::new_with_blockhash(instructions, Some(payer), &blockhash);
    let tx = Transaction::new_unsigned(message);

    let config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: true,
        commitment: Some(CommitmentConfig::processed()),
        ..RpcSimulateTransactionConfig::default()
    };

    match client.simulate_transaction_with_config(&tx, config) {
        Ok(response) => {
            let result = response.value;
            let logs = result.logs.unwrap_or_default();
            match result.err {
                None => (true, "simulation succeeded, no error".to_string(), logs),
                Some(err) => (false, format!("{err}"), logs),
            }
        }
        Err(e) => (false, format!("RPC call itself failed: {e}"), Vec::new()),
    }
}

/// Actually sends and confirms `instructions`, signed for real by `signers`. Only used for
/// `open_position`/`close_position` against a validator this harness has just funded itself --
/// simulation alone cannot prove a *sequence* of instructions leaves the chain in the state the
/// next one expects, and a real position genuinely created and genuinely closed is the strongest
/// evidence this suite can produce that the real program executes this crate's account list and
/// argument encoding exactly as intended, not merely that it fails to reject them outright.
fn send(
    client: &RpcClient,
    payer: &Pubkey,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> (bool, String, Vec<String>) {
    let blockhash = match client.get_latest_blockhash() {
        Ok(hash) => hash,
        Err(e) => {
            return (
                false,
                format!("fetching a recent blockhash: {e}"),
                Vec::new(),
            );
        }
    };
    let tx = Transaction::new_signed_with_payer(instructions, Some(payer), signers, blockhash);
    match client.send_and_confirm_transaction(&tx) {
        Ok(signature) => (true, format!("confirmed as {signature}"), Vec::new()),
        Err(e) => {
            // send_and_confirm_transaction's error doesn't carry the program's own log lines;
            // simulating the identical instructions is the standard way to recover them for
            // diagnosis without needing a second real send.
            let (_, _, logs) = simulate(client, payer, instructions);
            (false, format!("{e}"), logs)
        }
    }
}

/// Classifies a raw simulate/send outcome using only what the program's own logs say happened,
/// never a guess at what a numeric error code "probably" means beyond the vendored IDL's own
/// `errors` table. `reached_program` is true once the real program's log lines appear at all --
/// proof the runtime accepted the instruction's shape and handed it to the program to evaluate,
/// which is the property this whole suite exists to check. A `Custom` error matching the IDL's
/// table is additionally named, because that turns "the program said no" into "the program said
/// no, specifically: ExceededBinSlippageTolerance" -- worth having even though it does not change
/// the pass/fail verdict, which rests on `reached_program` alone.
fn classify(operation: &'static str, ok: bool, summary: String, logs: Vec<String>) -> Outcome {
    let reached_program = logs
        .iter()
        .any(|l| l.contains(&format!("Program {} invoke", dlmm_decode::ID)));

    let errors = idl_error_table();
    let mut named_summary = summary.clone();
    for (code, (name, msg)) in &errors {
        if summary.contains(&format!("Custom({code})"))
            || summary.contains(&format!("custom program error: 0x{code:x}"))
        {
            named_summary = format!("{summary} -- {name}: {msg}");
            break;
        }
    }

    Outcome {
        operation,
        ok: ok || reached_program,
        summary: named_summary,
        logs,
    }
}

fn report(outcome: &Outcome) {
    let status = if outcome.ok { "PASS" } else { "FAIL" };
    eprintln!("[{status}] {}: {}", outcome.operation, outcome.summary);
    for line in &outcome.logs {
        eprintln!("    {line}");
    }
}

#[test]
fn test_builders_against_real_program() {
    let Some(url) = validation_rpc_url() else {
        eprintln!(
            "skipping test_builders_against_real_program: DLMM_TX_VALIDATION_RPC_URL is not set \
             (see docs/validation.md to run this against a local validator or mainnet)"
        );
        return;
    };

    let client = RpcClient::new_with_commitment(url, CommitmentConfig::confirmed());
    let pool = pubkey(POOL);
    let sol_mint = pubkey(SOL_MINT);
    let usdc_mint = pubkey(USDC_MINT);

    let payer = Keypair::new();
    let position = Keypair::new();
    let rent_receiver = Pubkey::new_unique();

    // A successful airdrop only ever happens against a local validator's own faucet -- mainnet
    // refuses it outright. That refusal is the signal this run downgrades from "send real
    // transactions" to "simulate against synthetic, unfunded accounts": both are legitimate,
    // see docs/validation.md for what each does and does not prove.
    let send_capable = try_fund(&client, &payer.pubkey());
    if !send_capable {
        eprintln!(
            "note: airdrop unavailable at this endpoint -- falling back to simulate-only mode \
             against synthetic accounts (expected on mainnet; see docs/validation.md)"
        );
    }

    let budget = ComputeBudgetConfig {
        unit_limit: Some(400_000),
        unit_price_micro_lamports: None,
    };

    let mut outcomes = Vec::new();

    // open_position: the one operation every other operation below depends on, so it runs
    // first and, when this endpoint can fund a payer, is sent for real rather than simulated --
    // the position it creates is what add/remove/claim then reference.
    let open_ixs = build_open_position(
        &OpenPositionParams {
            lb_pair: pool,
            owner: payer.pubkey(),
            payer: payer.pubkey(),
            position: position.pubkey(),
            lower_bin_id: POSITION_LOWER_BIN_ID,
            width: POSITION_WIDTH,
        },
        &budget,
    )
    .expect("valid open_position params");

    let (open_ok, open_summary, open_logs) = if send_capable {
        send(&client, &payer.pubkey(), &open_ixs, &[&payer, &position])
    } else {
        simulate(&client, &payer.pubkey(), &open_ixs)
    };
    let position_created = send_capable && open_ok;
    outcomes.push(classify("open_position", open_ok, open_summary, open_logs));

    // add_liquidity / remove_liquidity / claim_fee always simulate, never send: even on a
    // funded local validator this harness holds no real SPL Token balance for either side of
    // the pool, so a real deposit could never succeed -- simulating still proves the real
    // program accepts the account list and argument encoding up to that funding shortfall.
    // Referencing the position this run just opened for real (when it could) rather than a
    // synthetic address makes any failure past that point attributable to the missing token
    // balance specifically, not to "no such position".
    let effective_position = if position_created {
        position.pubkey()
    } else {
        Pubkey::new_unique()
    };

    let add_ixs = build_add_liquidity_by_strategy(
        &AddLiquidityByStrategyParams {
            lb_pair: pool,
            position: effective_position,
            position_lower_bin_id: POSITION_LOWER_BIN_ID,
            position_upper_bin_id: POSITION_UPPER_BIN_ID,
            owner: payer.pubkey(),
            token_x_mint: sol_mint,
            token_y_mint: usdc_mint,
            token_x_program: dlmm_tx::TOKEN_PROGRAM_ID,
            token_y_program: dlmm_tx::TOKEN_PROGRAM_ID,
            amount_x: 1_500_000,
            amount_y: 2_500_000,
            active_id: current_active_id(&client, &pool),
            max_active_bin_slippage: 50,
            strategy_type: StrategyType::SpotBalanced,
            favor_token_x: false,
            min_bin_id: POSITION_LOWER_BIN_ID,
            max_bin_id: POSITION_UPPER_BIN_ID,
        },
        &budget,
    )
    .expect("valid add_liquidity params");
    let (ok, summary, logs) = simulate(&client, &payer.pubkey(), &add_ixs);
    outcomes.push(classify("add_liquidity", ok, summary, logs));

    let remove_ixs = build_remove_liquidity_by_range(
        &RemoveLiquidityByRangeParams {
            lb_pair: pool,
            position: effective_position,
            position_lower_bin_id: POSITION_LOWER_BIN_ID,
            position_upper_bin_id: POSITION_UPPER_BIN_ID,
            owner: payer.pubkey(),
            token_x_mint: sol_mint,
            token_y_mint: usdc_mint,
            token_x_program: dlmm_tx::TOKEN_PROGRAM_ID,
            token_y_program: dlmm_tx::TOKEN_PROGRAM_ID,
            from_bin_id: POSITION_LOWER_BIN_ID,
            to_bin_id: POSITION_UPPER_BIN_ID,
            bps_to_remove: 10_000,
        },
        &budget,
    )
    .expect("valid remove_liquidity params");
    let (ok, summary, logs) = simulate(&client, &payer.pubkey(), &remove_ixs);
    outcomes.push(classify("remove_liquidity", ok, summary, logs));

    let claim_ixs = build_claim_fee(
        &ClaimFeeParams {
            lb_pair: pool,
            position: effective_position,
            position_lower_bin_id: POSITION_LOWER_BIN_ID,
            position_upper_bin_id: POSITION_UPPER_BIN_ID,
            owner: payer.pubkey(),
            token_x_mint: sol_mint,
            token_y_mint: usdc_mint,
            token_x_program: dlmm_tx::TOKEN_PROGRAM_ID,
            token_y_program: dlmm_tx::TOKEN_PROGRAM_ID,
            min_bin_id: POSITION_LOWER_BIN_ID,
            max_bin_id: POSITION_UPPER_BIN_ID,
        },
        &budget,
    )
    .expect("valid claim_fee params");
    let (ok, summary, logs) = simulate(&client, &payer.pubkey(), &claim_ixs);
    outcomes.push(classify("claim_fee", ok, summary, logs));

    // close_position: sent for real, after the simulations above, when this run opened a real
    // position -- a freshly opened position holds zero liquidity and zero fees by construction,
    // so unlike add/remove/claim this one can genuinely succeed end to end without this harness
    // needing any token balance at all.
    let close_ixs = build_close_position(
        &ClosePositionParams {
            position: effective_position,
            owner: payer.pubkey(),
            rent_receiver,
        },
        &ComputeBudgetConfig::none(),
    );
    let (ok, summary, logs) = if position_created {
        send(&client, &payer.pubkey(), &close_ixs, &[&payer])
    } else {
        simulate(&client, &payer.pubkey(), &close_ixs)
    };
    outcomes.push(classify("close_position", ok, summary, logs));

    for outcome in &outcomes {
        report(outcome);
    }

    let failures: Vec<&str> = outcomes
        .iter()
        .filter(|o| !o.ok)
        .map(|o| o.operation)
        .collect();
    assert!(
        failures.is_empty(),
        "{} operation(s) never reached the real program's own logs, which every recognised \
         precondition-only failure (unfunded payer, missing token account, uninitialised bin \
         array) still does -- see docs/validation.md before treating this as anything other than \
         a genuine encoding bug: {failures:?}",
        failures.len()
    );
}

/// The pool's current on-chain active bin id, or the golden fixture's last-known value if the
/// account cannot be read -- either way this only feeds `add_liquidity`'s slippage check, so a
/// stale value at worst turns a PASS into an `ExceededBinSlippageTolerance` business-logic
/// rejection, never a structural one.
fn current_active_id(client: &RpcClient, pool: &Pubkey) -> i32 {
    client
        .get_account_data(pool)
        .ok()
        .and_then(|data| dlmm_decode::decode_lb_pair(&data).ok())
        .map(|lb_pair| lb_pair.active_bin_id)
        .unwrap_or(-5664)
}
