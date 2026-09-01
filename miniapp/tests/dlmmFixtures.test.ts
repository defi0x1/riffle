import { PublicKey } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { anchorDiscriminator } from "../src/solana/constants";
import { verifyTransaction } from "../src/verify/txVerifier";
import { FIXTURE_NAMES, type FixtureName, loadFixture } from "./support/fixtures";
import {
  duplicateDlmmInstruction,
  flipDlmmInstructionByte,
  substituteAccount,
  swapAccounts,
  swapDlmmDiscriminator,
} from "./support/mutate";

/**
 * Runs the real verifier (src/verify/txVerifier.ts) against transactions the Rust `dlmm_tx`
 * crate actually built -- see libraries/dlmm_tx/tests/fixtures.rs for how these fixtures are
 * generated and regenerated. Each fixture is proven two ways: it verifies successfully against
 * the semantics its own sidecar declares (the crate's real output matches what the UI would show
 * for it), and byte-realistic mutations derived from that same genuine transaction are all
 * rejected (the verifier is not just accepting anything that vaguely resembles a DLMM
 * transaction).
 *
 * What passing here does NOT prove: that either side's understanding of the wire format matches
 * what the on-chain DLMM program actually expects. A fixture only shows the verifier accepts
 * what this version of the Rust builder produces -- both sides could share the same
 * misunderstanding of an account order or argument layout and every test below would still pass.
 * The next step that would close that gap is simulating a built transaction against a real RPC
 * endpoint (or replaying it against a local validator with the real program deployed) and
 * checking it doesn't fail on chain; nothing in this repository does that today.
 */

const EXPECTED_INSTRUCTION_NAME: Record<FixtureName, string> = {
  open_position: "initializePosition2",
  add_liquidity: "addLiquidityByStrategy2",
  remove_liquidity: "removeLiquidityByRange2",
  claim_fee: "claimFee2",
  close_position: "closePosition2",
};

// Byte offset (from the start of the instruction data, i.e. counting the 8-byte discriminator)
// of one numeric argument that flows through to what the user was shown. close_position2 takes
// no arguments at all -- see the "no numeric argument to flip" case below for how its coverage
// of this failure mode works instead.
const NUMERIC_ARG_OFFSET: Partial<Record<FixtureName, number>> = {
  open_position: 8, // lowerBinId: i32
  add_liquidity: 8, // amountX: u64
  remove_liquidity: 16, // bpsToRemove: u16 (after fromBinId, toBinId)
  claim_fee: 8, // minBinId: i32
};

// A different real instruction's discriminator to swap each fixture's own into, so the mutated
// bytes always decode as *some* other genuine DLMM instruction rather than nothing at all.
const DISCRIMINATOR_SWAP_TARGET: Record<FixtureName, string> = {
  open_position: "close_position2",
  add_liquidity: "remove_liquidity_by_range2",
  remove_liquidity: "add_liquidity_by_strategy2",
  claim_fee: "close_position2",
  close_position: "claim_fee2",
};

const ROGUE_PUBKEY = new PublicKey(new Uint8Array(32).fill(0x99));

describe.each(FIXTURE_NAMES)("dlmm_tx fixture: %s", (name) => {
  const fixture = loadFixture(name);

  it("verifies successfully against the semantics its own sidecar declares", () => {
    const result = verifyTransaction(fixture.bytes, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.decoded.dlmmAction).toBe(EXPECTED_INSTRUCTION_NAME[name]);
      expect(result.decoded.instructionCount).toBeGreaterThanOrEqual(1);
    }
  });

  const numericOffset = NUMERIC_ARG_OFFSET[name];
  if (numericOffset !== undefined) {
    it("rejects a flipped numeric argument", () => {
      const mutated = flipDlmmInstructionByte(fixture.bytes, numericOffset);
      const result = verifyTransaction(mutated, {
        walletPubkey: fixture.walletPubkey,
        expected: fixture.expected,
      });
      expect(result.ok).toBe(false);
    });
  } else {
    it("has no numeric argument to flip (close_position2 takes none)", () => {
      expect(name).toBe("close_position");
    });
  }

  if (fixture.semantics.lbPair) {
    it("rejects a substituted pool address", () => {
      const mutated = substituteAccount(fixture.bytes, new PublicKey(fixture.semantics.lbPair as string), ROGUE_PUBKEY);
      const result = verifyTransaction(mutated, {
        walletPubkey: fixture.walletPubkey,
        expected: fixture.expected,
      });
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.reason).toMatch(/pool/i);
    });
  }

  it("rejects a substituted position account", () => {
    const mutated = substituteAccount(fixture.bytes, new PublicKey(fixture.semantics.position as string), ROGUE_PUBKEY);
    const result = verifyTransaction(mutated, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects a substituted owner", () => {
    const mutated = substituteAccount(fixture.bytes, fixture.walletPubkey, ROGUE_PUBKEY);
    const result = verifyTransaction(mutated, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects a swapped instruction discriminator", () => {
    const mutated = swapDlmmDiscriminator(
      fixture.bytes,
      anchorDiscriminator("global", DISCRIMINATOR_SWAP_TARGET[name]),
    );
    const result = verifyTransaction(mutated, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects an added extra DLMM instruction", () => {
    const mutated = duplicateDlmmInstruction(fixture.bytes);
    const result = verifyTransaction(mutated, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/exactly one DLMM instruction/);
  });

  it("rejects a changed required signer", () => {
    // Swap the wallet's account slot with some other already-present, non-wallet account's slot
    // -- the pool for the four operations that have one, or the position for close-position,
    // which has no pool account at all.
    const other = fixture.semantics.lbPair
      ? new PublicKey(fixture.semantics.lbPair)
      : new PublicKey(fixture.semantics.position as string);
    const mutated = swapAccounts(fixture.bytes, fixture.walletPubkey, other);
    const result = verifyTransaction(mutated, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(false);
  });
});
