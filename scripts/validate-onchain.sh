#!/bin/sh
# One-command entry point for libraries/dlmm_tx/tests/onchain_validation.rs -- the test that
# proves dlmm_tx's instruction builders are accepted by the real, deployed DLMM program, not
# merely self-consistent with the vendored IDL and the miniapp's TypeScript verifier. See
# docs/validation.md for what this does and does not prove, and how to read its output.
#
# Default mode: dumps the real program from mainnet, boots a throwaway local validator with it
# loaded plus a handful of real mainnet accounts cloned in, points the gated test at it, and
# tears the validator down again afterwards regardless of outcome. This is the strongest form
# of the check this task asks for -- it can fully fund a payer and send real transactions, not
# only simulate them.
#
# `--rpc-url <URL>` mode: skips the local validator entirely and runs the same test straight
# against a caller-supplied RPC endpoint (a public mainnet endpoint, say). No local validator
# needed, but simulate-only: this harness holds no real mainnet SOL, so nothing gets sent, only
# simulated -- and public endpoints rate-limit aggressively, so this mode makes only the five
# calls the test itself issues, no retries.
set -eu

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

PROGRAM_ID='LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo'
DUMP_SOURCE_URL='https://api.mainnet-beta.solana.com'
WORK_DIR="$REPO_ROOT/target/onchain-validate"
LOCAL_RPC_URL='http://127.0.0.1:8899'

# Real, already-initialised mainnet accounts the test's chosen operations touch -- the same
# SOL-USDC pool libraries/dlmm_tx/src/pda.rs and libraries/dlmm_decode/tests/golden.rs already
# cross-check their own derivations against. See onchain_validation.rs's own module doc for why
# this specific set is enough: every account the test's five instructions reference is either
# one of these, a fresh keypair the test generates itself, or a PDA this crate derives from them.
CLONE_ACCOUNTS='
5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6
EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o
CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz
EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
HQH5fsUpWdDtV5m4EaJo6TNcbLq5HxFzYzGXBptgJDD3
'

VALIDATOR_PID=''

cleanup() {
    if [ -n "$VALIDATOR_PID" ] && kill -0 "$VALIDATOR_PID" 2>/dev/null; then
        echo "validate-onchain: stopping local validator (pid $VALIDATOR_PID)"
        kill "$VALIDATOR_PID" 2>/dev/null || true
        wait "$VALIDATOR_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

RPC_URL=''
if [ "${1:-}" = '--rpc-url' ]; then
    RPC_URL="${2:?--rpc-url requires a URL argument}"
fi

if [ -n "$RPC_URL" ]; then
    echo "validate-onchain: simulate-only mode against $RPC_URL (no local validator)"
    DLMM_TX_VALIDATION_RPC_URL="$RPC_URL" \
        SQLX_OFFLINE=true \
        cargo test -p dlmm_tx --test onchain_validation -- --nocapture
    exit $?
fi

if ! command -v solana >/dev/null 2>&1 || ! command -v solana-test-validator >/dev/null 2>&1; then
    echo 'validate-onchain: solana and solana-test-validator must both be on PATH.' >&2
    echo 'validate-onchain: install the Solana CLI: https://docs.anza.xyz/cli/install' >&2
    echo 'validate-onchain: or run against a public RPC endpoint instead:' >&2
    echo "validate-onchain:   $0 --rpc-url https://api.mainnet-beta.solana.com" >&2
    exit 1
fi

mkdir -p "$WORK_DIR"
PROGRAM_SO="$WORK_DIR/dlmm_program.so"
LEDGER_DIR="$WORK_DIR/test-ledger"
VALIDATOR_LOG="$WORK_DIR/validator.log"

echo "validate-onchain: dumping the real program from $DUMP_SOURCE_URL"
solana program dump "$PROGRAM_ID" "$PROGRAM_SO" --url "$DUMP_SOURCE_URL"

# --bpf-program loads the dumped bytecode at the program's real mainnet address with upgrades
# disabled -- this is the actual deployed program, not a rebuild from source (the source isn't
# public; see libraries/dlmm_tx/src/lib.rs's module doc for why).
set -- --reset --quiet \
    --url "$DUMP_SOURCE_URL" \
    --ledger "$LEDGER_DIR" \
    --bpf-program "$PROGRAM_ID" "$PROGRAM_SO"
for account in $CLONE_ACCOUNTS; do
    set -- "$@" --clone "$account"
done

echo 'validate-onchain: starting local validator'
solana-test-validator "$@" >"$VALIDATOR_LOG" 2>&1 &
VALIDATOR_PID=$!

echo "validate-onchain: waiting for $LOCAL_RPC_URL to come up"
up=0
i=0
while [ "$i" -lt 30 ]; do
    if curl -s -m 2 -X POST -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
        "$LOCAL_RPC_URL" >/dev/null 2>&1; then
        up=1
        break
    fi
    if ! kill -0 "$VALIDATOR_PID" 2>/dev/null; then
        echo 'validate-onchain: validator process exited early; see log below' >&2
        cat "$VALIDATOR_LOG" >&2
        exit 1
    fi
    i=$((i + 1))
    sleep 1
done
if [ "$up" -ne 1 ]; then
    echo "validate-onchain: validator did not become healthy in time; see $VALIDATOR_LOG" >&2
    exit 1
fi
echo 'validate-onchain: validator is up'

DLMM_TX_VALIDATION_RPC_URL="$LOCAL_RPC_URL" \
    SQLX_OFFLINE=true \
    cargo test -p dlmm_tx --test onchain_validation -- --nocapture
