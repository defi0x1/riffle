//! Cross-language fixtures: one unsigned transaction per supported operation, built from this
//! crate's real public API with fixed, deterministic inputs, committed alongside a JSON sidecar
//! describing what the transaction is supposed to mean. `miniapp/tests/` loads these exact bytes
//! and runs the real TypeScript verifier against the sidecar's declared semantics -- the goal is
//! to prove the verifier accepts what this crate actually produces, not what a second,
//! hand-written TypeScript re-implementation of the wire format imagines this crate produces.
//!
//! Regenerate after any change to the instruction builders with:
//!
//!   cargo test -p dlmm_tx --test fixtures -- --ignored regenerate_fixtures
//!
//! `test_fixtures_match_committed_bytes` rebuilds every fixture from the same inputs and fails
//! loudly if the freshly built bytes disagree with what's committed -- a builder change breaks
//! this test until someone runs the command above and commits the result, so drift between the
//! fixtures and the code that built them cannot pass silently.

use std::fs;
use std::path::{Path, PathBuf};

use dlmm_tx::{
    AddLiquidityByStrategyParams, ClaimFeeParams, ClosePositionParams, ComputeBudgetConfig,
    OpenPositionParams, RemoveLiquidityByRangeParams, StrategyType,
    build_add_liquidity_by_strategy, build_claim_fee, build_close_position, build_open_position,
    build_remove_liquidity_by_range,
};
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction;

fn pubkey(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

// One fixed identity per role, reused across every fixture so the five fixtures read as one
// consistent position lifecycle (open it, add to it, remove from it, claim its fees, close it)
// rather than five unrelated pools and wallets.
fn owner() -> Pubkey {
    pubkey(0xA1)
}
fn lb_pair() -> Pubkey {
    pubkey(0xB1)
}
fn token_x_mint() -> Pubkey {
    pubkey(0xC1)
}
fn token_y_mint() -> Pubkey {
    pubkey(0xC2)
}
fn position() -> Pubkey {
    pubkey(0xD1)
}

// Deliberately distinct from `owner()`: a rent receiver that happens to equal the owner would
// make the close-position "rent redirected" negative fixture unable to distinguish substituting
// the rent receiver from substituting the owner, since both roles would sit in the same account
// slot. A separate designated receiver (a treasury address, say) is realistic and keeps the two
// mutations independently testable.
fn rent_receiver() -> Pubkey {
    pubkey(0xE1)
}

// The position this lifecycle opens: width 70 (the maximum a single initialize_position2 call
// can cover) starting at bin -35, so it straddles the active bin at 0.
const POSITION_LOWER_BIN_ID: i32 = -35;
const POSITION_WIDTH: i32 = 70;
const POSITION_UPPER_BIN_ID: i32 = POSITION_LOWER_BIN_ID + POSITION_WIDTH - 1;

struct Fixture {
    name: &'static str,
    instructions: Vec<Instruction>,
    semantics: serde_json::Value,
}

fn build_fixtures() -> Vec<Fixture> {
    let budget = ComputeBudgetConfig {
        unit_limit: Some(300_000),
        unit_price_micro_lamports: Some(1_000),
    };

    let open_position = {
        let params = OpenPositionParams {
            lb_pair: lb_pair(),
            owner: owner(),
            payer: owner(),
            position: position(),
            lower_bin_id: POSITION_LOWER_BIN_ID,
            width: POSITION_WIDTH,
        };
        let instructions =
            build_open_position(&params, &budget).expect("fixture inputs must build cleanly");
        let semantics = serde_json::json!({
            "operation": "open-position",
            "walletPubkey": owner().to_string(),
            "lbPair": lb_pair().to_string(),
            "tokenXMint": token_x_mint().to_string(),
            "tokenYMint": token_y_mint().to_string(),
            "position": position().to_string(),
            "lowerBinId": POSITION_LOWER_BIN_ID,
            "width": POSITION_WIDTH,
        });
        Fixture {
            name: "open_position",
            instructions,
            semantics,
        }
    };

    let add_liquidity = {
        let params = AddLiquidityByStrategyParams {
            lb_pair: lb_pair(),
            position: position(),
            position_lower_bin_id: POSITION_LOWER_BIN_ID,
            position_upper_bin_id: POSITION_UPPER_BIN_ID,
            owner: owner(),
            token_x_mint: token_x_mint(),
            token_y_mint: token_y_mint(),
            token_x_program: dlmm_tx::TOKEN_PROGRAM_ID,
            token_y_program: dlmm_tx::TOKEN_PROGRAM_ID,
            amount_x: 1_500_000,
            amount_y: 2_500_000,
            active_id: 0,
            max_active_bin_slippage: 5,
            strategy_type: StrategyType::SpotBalanced,
            favor_token_x: false,
            min_bin_id: -10,
            max_bin_id: 10,
        };
        let instructions = build_add_liquidity_by_strategy(&params, &budget)
            .expect("fixture inputs must build cleanly");
        let semantics = serde_json::json!({
            "operation": "add-liquidity",
            "walletPubkey": owner().to_string(),
            "lbPair": lb_pair().to_string(),
            "position": position().to_string(),
            "positionLowerBinId": POSITION_LOWER_BIN_ID,
            "positionUpperBinId": POSITION_UPPER_BIN_ID,
            "tokenXMint": token_x_mint().to_string(),
            "tokenYMint": token_y_mint().to_string(),
            "tokenXProgram": dlmm_tx::TOKEN_PROGRAM_ID.to_string(),
            "tokenYProgram": dlmm_tx::TOKEN_PROGRAM_ID.to_string(),
            "amountX": "1500000",
            "amountY": "2500000",
            "activeId": 0,
            "maxActiveBinSlippage": 5,
            "minBinId": -10,
            "maxBinId": 10,
        });
        Fixture {
            name: "add_liquidity",
            instructions,
            semantics,
        }
    };

    let remove_liquidity = {
        let params = RemoveLiquidityByRangeParams {
            lb_pair: lb_pair(),
            position: position(),
            position_lower_bin_id: POSITION_LOWER_BIN_ID,
            position_upper_bin_id: POSITION_UPPER_BIN_ID,
            owner: owner(),
            token_x_mint: token_x_mint(),
            token_y_mint: token_y_mint(),
            token_x_program: dlmm_tx::TOKEN_PROGRAM_ID,
            token_y_program: dlmm_tx::TOKEN_PROGRAM_ID,
            from_bin_id: -10,
            to_bin_id: 10,
            bps_to_remove: 5_000,
        };
        let instructions = build_remove_liquidity_by_range(&params, &budget)
            .expect("fixture inputs must build cleanly");
        let semantics = serde_json::json!({
            "operation": "remove-liquidity",
            "walletPubkey": owner().to_string(),
            "lbPair": lb_pair().to_string(),
            "position": position().to_string(),
            "positionLowerBinId": POSITION_LOWER_BIN_ID,
            "positionUpperBinId": POSITION_UPPER_BIN_ID,
            "tokenXMint": token_x_mint().to_string(),
            "tokenYMint": token_y_mint().to_string(),
            "tokenXProgram": dlmm_tx::TOKEN_PROGRAM_ID.to_string(),
            "tokenYProgram": dlmm_tx::TOKEN_PROGRAM_ID.to_string(),
            "fromBinId": -10,
            "toBinId": 10,
            "bpsToRemove": 5_000,
        });
        Fixture {
            name: "remove_liquidity",
            instructions,
            semantics,
        }
    };

    let claim_fee = {
        let params = ClaimFeeParams {
            lb_pair: lb_pair(),
            position: position(),
            position_lower_bin_id: POSITION_LOWER_BIN_ID,
            position_upper_bin_id: POSITION_UPPER_BIN_ID,
            owner: owner(),
            token_x_mint: token_x_mint(),
            token_y_mint: token_y_mint(),
            token_x_program: dlmm_tx::TOKEN_PROGRAM_ID,
            token_y_program: dlmm_tx::TOKEN_PROGRAM_ID,
            min_bin_id: POSITION_LOWER_BIN_ID,
            max_bin_id: POSITION_UPPER_BIN_ID,
        };
        let instructions =
            build_claim_fee(&params, &budget).expect("fixture inputs must build cleanly");
        let semantics = serde_json::json!({
            "operation": "claim-fees",
            "walletPubkey": owner().to_string(),
            "lbPair": lb_pair().to_string(),
            "position": position().to_string(),
            "positionLowerBinId": POSITION_LOWER_BIN_ID,
            "positionUpperBinId": POSITION_UPPER_BIN_ID,
            "tokenXMint": token_x_mint().to_string(),
            "tokenYMint": token_y_mint().to_string(),
            "tokenXProgram": dlmm_tx::TOKEN_PROGRAM_ID.to_string(),
            "tokenYProgram": dlmm_tx::TOKEN_PROGRAM_ID.to_string(),
            "minBinId": POSITION_LOWER_BIN_ID,
            "maxBinId": POSITION_UPPER_BIN_ID,
        });
        Fixture {
            name: "claim_fee",
            instructions,
            semantics,
        }
    };

    let close_position = {
        let params = ClosePositionParams {
            position: position(),
            owner: owner(),
            rent_receiver: rent_receiver(),
        };
        let instructions = build_close_position(&params, &ComputeBudgetConfig::none());
        let semantics = serde_json::json!({
            "operation": "close-position",
            "walletPubkey": owner().to_string(),
            "position": position().to_string(),
            "rentReceiver": rent_receiver().to_string(),
        });
        Fixture {
            name: "close_position",
            instructions,
            semantics,
        }
    };

    vec![
        open_position,
        add_liquidity,
        remove_liquidity,
        claim_fee,
        close_position,
    ]
}

/// Compiles instructions into a fully unsigned, fixed-blockhash transaction and serialises it
/// with the same wire format `VersionedTransaction.deserialize()` on the TypeScript side expects
/// for a legacy (non-versioned) message: a compact-array of signature placeholders followed by
/// the compiled message bytes. A zero blockhash keeps the fixture reproducible -- nothing here is
/// ever sent to a cluster, so a real recent blockhash would only make the fixture non-deterministic
/// for no benefit.
fn compile_unsigned(payer: &Pubkey, instructions: &[Instruction]) -> Vec<u8> {
    let message = Message::new_with_blockhash(instructions, Some(payer), &Hash::default());
    let tx = Transaction::new_unsigned(message);
    bincode::serialize(&tx).expect("transaction bincode serialisation cannot fail")
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(hex: &str) -> Vec<u8> {
    let hex = hex.trim();
    assert!(hex.len().is_multiple_of(2), "hex fixture has odd length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("invalid hex fixture"))
        .collect()
}

fn fixture_dir() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/dlmm_tx"
    ))
    .to_path_buf()
}

fn tx_path(name: &str) -> PathBuf {
    fixture_dir().join(format!("{name}.tx.hex"))
}

fn json_path(name: &str) -> PathBuf {
    fixture_dir().join(format!("{name}.json"))
}

#[test]
fn test_fixtures_match_committed_bytes() {
    for fixture in build_fixtures() {
        let bytes = compile_unsigned(&owner(), &fixture.instructions);
        let hex = to_hex(&bytes);

        let committed_hex = fs::read_to_string(tx_path(fixture.name)).unwrap_or_else(|_| {
            panic!(
                "missing committed fixture for {}; regenerate with `cargo test -p dlmm_tx --test \
                 fixtures -- --ignored regenerate_fixtures`",
                fixture.name
            )
        });
        assert_eq!(
            hex,
            committed_hex.trim(),
            "{}: freshly built transaction bytes disagree with the committed fixture -- the \
             instruction builder changed since this fixture was generated; regenerate with \
             `cargo test -p dlmm_tx --test fixtures -- --ignored regenerate_fixtures` and review \
             the diff before committing",
            fixture.name
        );

        let committed_json = fs::read_to_string(json_path(fixture.name)).unwrap_or_else(|_| {
            panic!(
                "missing committed sidecar for {}; regenerate with `cargo test -p dlmm_tx --test \
                 fixtures -- --ignored regenerate_fixtures`",
                fixture.name
            )
        });
        let committed_value: serde_json::Value =
            serde_json::from_str(&committed_json).expect("parsing committed sidecar");
        assert_eq!(
            fixture.semantics, committed_value,
            "{}: freshly computed semantics disagree with the committed sidecar; regenerate with \
             `cargo test -p dlmm_tx --test fixtures -- --ignored regenerate_fixtures`",
            fixture.name
        );
    }
}

/// Not run by default (see the module doc comment for why): writes the committed fixtures. Run
/// deliberately after an instruction-builder change, then review the diff before committing --
/// this test intentionally has no assertions of its own.
#[test]
#[ignore]
fn regenerate_fixtures() {
    fs::create_dir_all(fixture_dir()).expect("creating fixture directory");
    for fixture in build_fixtures() {
        let bytes = compile_unsigned(&owner(), &fixture.instructions);
        fs::write(tx_path(fixture.name), to_hex(&bytes)).expect("writing tx fixture");
        let pretty = serde_json::to_string_pretty(&fixture.semantics).expect("serialising sidecar");
        fs::write(json_path(fixture.name), pretty + "\n").expect("writing sidecar fixture");
    }
}

/// Sanity check on the encoding helpers themselves, independent of any fixture file.
#[test]
fn test_hex_round_trips() {
    let bytes = vec![0u8, 1, 255, 16, 128];
    assert_eq!(from_hex(&to_hex(&bytes)), bytes);
}
