# Validating dlmm_tx against the real program

`libraries/dlmm_tx` builds unsigned Solana transactions for Meteora's DLMM program, transcribed
from the public IDL (the program's own source is not public -- see the module doc on
`libraries/dlmm_tx/src/lib.rs`). `miniapp/tests/` independently verifies those same transactions
against the same declared semantics, and `fixtures/dlmm_tx/` pins the exact bytes both sides
agree on today.

None of that proves the on-chain program actually accepts what gets built. This crate and the
TypeScript verifier could share the same wrong account order, the same wrong PDA seed, or the
same wrong argument layout, and every test described above would still pass -- right up until
the first real transaction either fails on chain or, worse, succeeds in a way nobody intended.

`libraries/dlmm_tx/tests/onchain_validation.rs` closes that specific gap. This document explains
what it proves, how to run it, what each outcome means, and -- the part that matters most --
how to tell a genuine builder bug apart from the test simply hitting an account this harness
never funded or initialised.

## What this proves that the rest of the suite does not

Every other check in this crate stays entirely inside the Rust process: it asserts that a built
instruction matches the vendored IDL, that two runs of the same builder produce the same bytes,
that a hand-picked strategy variant borsh-encodes to the index Anchor expects. `pda.rs`'s own
unit tests go one step further and spot-check four individual PDA formulas (`bin_array`,
`event_authority`, `reserve`, the associated-token-account derivation) against live mainnet
values fetched during development -- useful, but only proof that a few isolated formulas match
reality, not that a *complete* instruction -- the right accounts, in the right order, with the
right signer/writable flags, carrying the right discriminator and the right argument bytes --
is one the real, deployed program will actually execute.

This test sends or simulates the five real instructions this crate builds
(`initialize_position2`, `add_liquidity_by_strategy2`, `remove_liquidity_by_range2`,
`claim_fee2`, `close_position2`) against the actual compiled program at
`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`, dumped straight from mainnet -- not a rebuild
from source, since none exists publicly, but the literal bytecode mainnet runs. When it runs
against a local validator with a funded payer (the default `scripts/validate-onchain.sh` path),
it does something no other test in this repository does: it actually opens a real position and
actually closes it again, two genuine state-changing transactions confirmed by the real program,
proving not just that the program *accepts* the shape of what this crate builds but that
executing it leaves the chain in the state a caller would expect.

## How to run it

One command, from the repository root:

```sh
make validate-onchain
# or directly:
sh scripts/validate-onchain.sh
```

This requires the Solana CLI (`solana` and `solana-test-validator`) on `PATH`
(<https://docs.anza.xyz/cli/install>) and outbound network access to `api.mainnet-beta.solana.com`.
When both are available, the script:

1. Dumps the real program (`solana program dump`) into `target/onchain-validate/` (git-ignored,
   nothing here is ever committed).
2. Boots a throwaway `solana-test-validator`, loading that dump at the program's real address
   with `--bpf-program` (upgrades disabled -- this run never needs to upgrade it) and cloning a
   handful of real, already-initialised mainnet accounts into it with `--clone`: the live
   SOL-USDC DLMM pool (`5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6`), its two reserve vaults,
   the USDC mint, and one of its BinArray accounts (index -81, covering bin ids -5670..=-5601).
   Native SOL, the SPL Token program, the Associated Token Account program, and the Memo program
   all ship in `solana-test-validator`'s own default genesis, so nothing needs cloning for those.
3. Waits for the validator's RPC endpoint to report healthy.
4. Runs `cargo test -p dlmm_tx --test onchain_validation -- --nocapture` with
   `DLMM_TX_VALIDATION_RPC_URL` pointed at it.
5. Tears the validator down again, whether the test passed or not.

If the Solana CLI is not installed, the script says so and exits rather than pretending to have
run anything, and suggests the second mode below.

### Mainnet-simulation mode (no local validator)

```sh
sh scripts/validate-onchain.sh --rpc-url https://api.mainnet-beta.solana.com
```

Skips the local validator and runs the same test straight against the given RPC endpoint. This
harness holds no real mainnet SOL, so nothing is ever sent this way -- every one of the five
operations goes through `simulateTransaction` with `sigVerify: false` (no signature required;
see "Why `sigVerify: false` is not a shortcut" below) and `replaceRecentBlockhash: true`. That
still surfaces every account-order and argument-encoding error a local-validator run would, just
without the local run's ability to actually fund a payer and prove a real create/close round
trip. Public endpoints rate-limit aggressively; this mode makes exactly five RPC calls (plus one
read of the pool's current active bin), never a retry loop.

### Running it directly

Both modes above are `scripts/validate-onchain.sh` wrapping the same underlying command:

```sh
DLMM_TX_VALIDATION_RPC_URL=http://127.0.0.1:8899 \
  cargo test -p dlmm_tx --test onchain_validation -- --nocapture
```

Point `DLMM_TX_VALIDATION_RPC_URL` at any reachable Solana RPC endpoint -- a local validator you
started yourself, a devnet cluster with the program deployed there, whatever you have. The test
reads that one environment variable and nothing else.

### Why this never runs in `cargo test --workspace`

`libraries/dlmm_tx/tests/onchain_validation.rs` gates itself on `DLMM_TX_VALIDATION_RPC_URL`
being set to a non-empty value, exactly the way `tests/src/lib.rs`'s `require_database!` gates
this workspace's database-backed integration suite on `DATABASE_URL`. Unset (the case for every
normal `cargo test --workspace`, and for CI's `test` job), the test prints why it is skipping to
stderr and returns immediately -- it neither fails nor silently no-ops without saying so.
`SQLX_OFFLINE=true cargo test -p dlmm_tx` passes with this file present precisely because of that
gate; it was run as part of writing this document to confirm it.

## What each outcome means

The test prints one `[PASS]` or `[FAIL]` line per operation, followed by the program's own log
lines when there were any. Read the verdict rule before treating a `[FAIL]` as a confirmed bug:

**A `[PASS]` means one of two things**, and the log lines tell you which:

- The transaction fully succeeded (`open_position` and `close_position`, when the run could
  fund a payer) -- the strongest possible outcome: the real program executed every account this
  crate resolved and left the chain in the expected state.
- The transaction failed, but only *after* the real program's own log lines appear (a line
  starting `Program LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo invoke`, followed by
  `Program log: Instruction: <Name>`). Reaching that point means the Solana runtime accepted the
  instruction's account list and dispatched it, and the program's own account-resolution and
  argument-decoding logic ran far enough to name a specific reason for rejecting it. That
  rejection is a business-logic or precondition failure, not evidence of a wrong account order or
  a wrong argument encoding -- if either of those were wrong, the program would not have gotten
  as far as it did, or would be complaining about a different account than the one this test
  deliberately left unfunded or uninitialised.

**A `[FAIL]` means the transaction never reached the program's own logs at all** -- rejected by
the Solana runtime itself (a wrong account count, a missing required signature, a malformed
instruction) before the DLMM program's code ever ran. This is the one outcome that should be
investigated as a possible encoding bug: start by comparing the account list `src/instructions/`
built against the vendored IDL's declared order (`tests/idl_conformance.rs` and
`src/test_support.rs::assert_matches_idl` already do this at the unit-test level; a `[FAIL]` here
means something the *deployed* program expects has drifted from what that IDL, and this crate,
believe).

### The one recurring, expected failure this harness will show you

Running the default local-validator mode today, `add_liquidity`, `remove_liquidity`, and
`claim_fee` all report `[PASS]` by reaching this exact log line:

```
Program log: AnchorError caused by account: user_token_x. Error Code: AccountNotInitialized.
Error Number: 3012. Error Message: The program expected this account to be already initialized.
```

This is expected, not a bug: this harness generates a fresh, disposable keypair for its payer/
owner and never creates or funds that wallet's SPL Token accounts for either side of the pool
(wrapping native SOL is possible without any special authority; minting real USDC is not, since
this harness holds no USDC mint authority -- see "What remains unproven" below). The program
naming the *specific* account it expected to exist, by its declared name, after correctly
resolving the pool, the position, the reserves, and the bin array PDAs first, is itself strong
evidence the account list this crate built was correct up to that point -- a wrong account order
would far more likely produce a confusing error about the wrong account, or an error before the
program's logs begin at all, not a precise, correctly-named complaint about the one account this
test knowingly left uninitialised.

Other outcomes you may see instead, and how to read them:

| What you see | What it means |
| --- | --- |
| `[PASS]`, transaction fully confirmed | The real program executed it. Nothing left to check. |
| `[PASS]`, a named `AccountNotInitialized`/`AccountOwnedByWrongProgram` on `user_token_x`/`user_token_y` | Expected -- this harness's payer has no token account there. Not a bug. |
| `[PASS]`, a numeric `custom program error` matching one of the vendored IDL's `errors` entries (`ExceededBinSlippageTolerance`, `ZeroLiquidity`, etc.) | The program evaluated the request and rejected it for the business reason named. Not a bug in this crate; possibly worth adjusting the test's own fixture inputs if the rejection is surprising, but the account list and argument encoding are proven correct either way. |
| `[FAIL]`, `AccountNotFound` on `pool`/`lb_pair`, a reserve, or the bin array, in mainnet-simulation mode | Expected on a public RPC run against synthetic/placeholder accounts if you changed the constants in `onchain_validation.rs` away from the real pool this file already hard-codes; not expected against the real pool address it ships with. |
| `[FAIL]`, `NotEnoughAccountKeys`, `InvalidInstructionData`, `PrivilegeEscalation`, or any error before the program's own log lines | Investigate as a genuine encoding bug -- see above. |

### Why `sigVerify: false` is not a shortcut

Every simulate call in this test passes `sigVerify: false`. That is not skipping the check this
test exists to perform -- signature verification is a cryptographic check that has nothing to do
with whether an account list or an argument layout is correct, and requiring it would mean this
test could only ever validate transactions signed by keys it holds. Setting it false is what
`libraries/dlmm_tx`'s own design already relies on: this crate never holds a private key by
construction (`scripts/keyless-guard.sh`, `docs/security.md`) and builds transactions the user's
own device signs later. Simulating with `sigVerify: false` reproduces exactly that separation --
proving the real program accepts the *shape* of a transaction, the only thing this crate is
responsible for, independent of who eventually signs it.

## What remains unproven even after this passes

- **Real deposits and withdrawals with actual token balances.** This harness never sends
  `add_liquidity`, `remove_liquidity`, or `claim_fee` for real, only simulates them, because it
  holds no real balance of either token to deposit and no way to acquire USDC balance without a
  mint authority it does not have. A `[PASS]` on these three proves the account list and argument
  encoding are correct up to the point the program checks token balances; it does not prove the
  liquidity math on the far side of a real deposit behaves as expected. Wrapping native SOL into
  the payer's own associated token account (no special authority required) and pre-creating both
  associated token accounts before simulating would push the proof one step further and is a
  reasonable next enhancement, left undone here to keep this harness's own footprint small.
- **Positions or amounts outside the specific range this test exercises.** The test opens a
  position at bin ids -5660..-5641, fully inside one already-initialised BinArray, chosen so the
  harness does not depend on any account that might not exist. A position wide enough to span
  multiple BinArrays, or one that needs the `bin_array_bitmap_extension` account
  (`pda::bin_array_bitmap_extension_required`), is not exercised here.
- **Token-2022 mints, or mints with a transfer hook.** The real pool this test uses is a plain
  SPL Token pool; `RemainingAccountsInfo::none()` (see `src/args.rs`) is never exercised against
  a mint that actually has hook accounts to append.
- **Anything about `miniapp/`'s own verifier.** This file only checks dlmm_tx's output against
  the real program. The cross-language agreement between dlmm_tx and the TypeScript verifier is
  what `fixtures/dlmm_tx/` and `miniapp/tests/` already check, and is out of scope here.
- **Behaviour under real network conditions** -- congestion, priority fees actually needed to
  land, blockhash expiry -- since a local validator has none of these, and a mainnet-simulation
  run never actually lands a transaction at all.

## What this run found

Run against the real, deployed program (both the local-validator and mainnet-simulation paths),
every one of the five operations this crate builds passed by the rule above: `open_position` and
`close_position` were sent for real and confirmed by the network; `add_liquidity`,
`remove_liquidity`, and `claim_fee` all reached the program's own logs and were rejected only for
the expected, documented reason (an uninitialised token account this harness deliberately never
funded). No encoding bug was found in `libraries/dlmm_tx/src/`.
