import {
  Keypair,
  MessageV0,
  PublicKey,
  SystemProgram,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import type { ExpectedAction } from "../src/verify/types";
import { verifyTransaction } from "../src/verify/txVerifier";
import { loadFixture } from "./support/fixtures";
import { extractDlmmInstruction, substituteAccount } from "./support/mutate";

/**
 * Verifier behaviours that apply across every action kind -- allow-listed programs, unexpected
 * signers, address lookup tables, malformed bytes -- rather than to one specific DLMM
 * instruction's own fields. Those per-instruction checks (tampered amounts, substituted pools,
 * swapped discriminators, and so on, for each of the five operations) live in
 * tests/dlmmFixtures.test.ts instead, run against the real Rust-built fixture for every
 * operation. Here, one genuine fixture per describe block is enough to exercise a behaviour that
 * doesn't depend on which DLMM instruction is inside it.
 */

const DUMMY_BLOCKHASH = new PublicKey(new Uint8Array(32)).toBase58();

function compileUnsigned(instructions: TransactionInstruction[], payer: PublicKey): Uint8Array {
  const message = new TransactionMessage({
    payerKey: payer,
    instructions,
    recentBlockhash: DUMMY_BLOCKHASH,
  }).compileToLegacyMessage();
  return new VersionedTransaction(message).serialize();
}

describe("verifyTransaction: generic behaviours (genuine add-liquidity fixture)", () => {
  const fixture = loadFixture("add_liquidity");
  const wallet = fixture.walletPubkey;
  const expected: ExpectedAction = fixture.expected;

  it("accepts the genuine fixture unmodified", () => {
    const result = verifyTransaction(fixture.bytes, { walletPubkey: wallet, expected });
    expect(result.ok).toBe(true);
  });

  it("refuses to sign when the token destination is not this wallet's own token account", () => {
    // The genuine fixture's own token-X destination account, redirected to an unrelated
    // address -- models a backend that sends the deposit to an attacker-controlled token
    // account while every other field (owner, pool, amounts) still looks exactly as shown.
    const dlmmIx = extractDlmmInstruction(fixture.bytes);
    const userX = dlmmIx.keys[3];
    if (!userX) throw new Error("fixture's add-liquidity instruction is missing its token X account");
    const rogueAta = Keypair.generate().publicKey;
    const mutated = substituteAccount(fixture.bytes, userX.pubkey, rogueAta);
    const result = verifyTransaction(mutated, { walletPubkey: wallet, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/token X destination|token Y destination/);
  });

  it("refuses to sign when an instruction targets a program not on the allow-list", () => {
    const dlmmIx = extractDlmmInstruction(fixture.bytes);
    const rogueProgram = Keypair.generate().publicKey;
    const rogueIx = new TransactionInstruction({
      keys: [{ pubkey: wallet, isSigner: true, isWritable: true }],
      programId: rogueProgram,
      data: Buffer.from([1, 2, 3]),
    });
    const bytes = compileUnsigned([dlmmIx, rogueIx], wallet);
    const result = verifyTransaction(bytes, { walletPubkey: wallet, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/allow-list/);
  });

  it("refuses a transaction that requires an unexpected extra signer", () => {
    const dlmmIx = extractDlmmInstruction(fixture.bytes);
    const extraSigner = Keypair.generate().publicKey;
    const decoyIx = new TransactionInstruction({
      keys: [
        { pubkey: wallet, isSigner: true, isWritable: true },
        { pubkey: extraSigner, isSigner: true, isWritable: false },
      ],
      programId: SystemProgram.programId,
      data: Buffer.alloc(0),
    });
    const bytes = compileUnsigned([dlmmIx, decoyIx], wallet);
    const result = verifyTransaction(bytes, { walletPubkey: wallet, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/unexpected signer/);
  });

  it("refuses a transaction that uses an address lookup table", () => {
    const dlmmIx = extractDlmmInstruction(fixture.bytes);
    const legacyMessage = new TransactionMessage({
      payerKey: wallet,
      instructions: [dlmmIx],
      recentBlockhash: DUMMY_BLOCKHASH,
    }).compileToLegacyMessage();

    const messageV0 = new MessageV0({
      header: legacyMessage.header,
      staticAccountKeys: legacyMessage.staticAccountKeys,
      recentBlockhash: DUMMY_BLOCKHASH,
      compiledInstructions: legacyMessage.compiledInstructions,
      addressTableLookups: [
        { accountKey: Keypair.generate().publicKey, writableIndexes: [0], readonlyIndexes: [] },
      ],
    });
    const bytes = new VersionedTransaction(messageV0).serialize();
    const result = verifyTransaction(bytes, { walletPubkey: wallet, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/address lookup table/);
  });
});

describe("verifyTransaction: close-position (genuine close-position fixture)", () => {
  const fixture = loadFixture("close_position");

  it("accepts a correctly built close-position transaction", () => {
    const result = verifyTransaction(fixture.bytes, {
      walletPubkey: fixture.walletPubkey,
      expected: fixture.expected,
    });
    expect(result.ok).toBe(true);
  });

  it("refuses to sign when rent is redirected to an address other than what was shown", () => {
    const expected = fixture.expected as Extract<ExpectedAction, { kind: "close-position" }>;
    const attacker = Keypair.generate().publicKey;
    const mutated = substituteAccount(fixture.bytes, expected.rentReceiver, attacker);
    const result = verifyTransaction(mutated, { walletPubkey: fixture.walletPubkey, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/rent receiver/);
  });
});

describe("verifyTransaction: malformed input", () => {
  it("fails cleanly on bytes that are not a valid transaction", () => {
    const owner = Keypair.generate().publicKey;
    const expected: ExpectedAction = {
      kind: "close-position",
      position: Keypair.generate().publicKey,
      rentReceiver: owner,
    };
    const result = verifyTransaction(new Uint8Array([1, 2, 3, 4]), { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
  });
});
