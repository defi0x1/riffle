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

import { TOKEN_PROGRAM_ID } from "../src/solana/constants";
import type { ExpectedAction } from "../src/verify/types";
import { verifyTransaction } from "../src/verify/txVerifier";
import {
  buildAddLiquidityInstruction,
  buildClosePositionInstruction,
  buildRemoveLiquidityInstruction,
} from "./support/dlmmIx";

const DUMMY_BLOCKHASH = new PublicKey(new Uint8Array(32)).toBase58();

function compileUnsigned(instructions: TransactionInstruction[], payer: PublicKey): Uint8Array {
  const message = new TransactionMessage({
    payerKey: payer,
    instructions,
    recentBlockhash: DUMMY_BLOCKHASH,
  }).compileToLegacyMessage();
  return new VersionedTransaction(message).serialize();
}

describe("verifyTransaction: add-liquidity", () => {
  const owner = Keypair.generate().publicKey;
  const lbPair = Keypair.generate().publicKey;
  const position = Keypair.generate().publicKey;
  const tokenXMint = Keypair.generate().publicKey;
  const tokenYMint = Keypair.generate().publicKey;

  const fixture = {
    lbPair,
    position,
    owner,
    tokenXMint,
    tokenYMint,
    tokenXProgram: TOKEN_PROGRAM_ID,
    tokenYProgram: TOKEN_PROGRAM_ID,
    amountX: 1_000_000n,
    amountY: 2_000_000n,
    activeId: 100,
    maxActiveBinSlippage: 5,
    minBinId: 80,
    maxBinId: 120,
  };

  const expected: ExpectedAction = {
    kind: "add-liquidity",
    lbPair,
    position,
    positionLowerBinId: 80,
    positionUpperBinId: 120,
    tokenXMint,
    tokenYMint,
    tokenXProgram: TOKEN_PROGRAM_ID,
    tokenYProgram: TOKEN_PROGRAM_ID,
    amountX: 1_000_000n,
    amountY: 2_000_000n,
    activeId: 100,
    maxActiveBinSlippage: 5,
    minBinId: 80,
    maxBinId: 120,
  };

  it("accepts a correctly built transaction matching the displayed summary", () => {
    const ix = buildAddLiquidityInstruction(fixture);
    const bytes = compileUnsigned([ix], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(true);
  });

  it("refuses to sign when the decoded amount does not match what was shown (tampered amount)", () => {
    // The backend claims (via `expected`, standing in for the displayed summary) that amountX is
    // 1_000_000, but the actual instruction it built deposits 9_000_000 -- exactly the "decoded
    // transaction does not match the displayed summary" case.
    const ix = buildAddLiquidityInstruction({ ...fixture, amountX: 9_000_000n });
    const bytes = compileUnsigned([ix], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/amount/i);
  });

  it("refuses to sign when the pool address does not match what was shown", () => {
    const wrongPool = Keypair.generate().publicKey;
    const ix = buildAddLiquidityInstruction({ ...fixture, lbPair: wrongPool });
    const bytes = compileUnsigned([ix], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
  });

  it("refuses to sign when the owner is not this wallet", () => {
    const attacker = Keypair.generate().publicKey;
    const ix = buildAddLiquidityInstruction({ ...fixture, owner: attacker });
    // Attacker pays and signs instead of the real wallet -- the real wallet's signature is never
    // required at all, which the verifier must also catch.
    const bytes = compileUnsigned([ix], attacker);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
  });

  it("refuses to sign when the token destination is not this wallet's own token account", () => {
    // Simulate a backend that redirects the deposit's destination to an attacker-controlled
    // token account by building the instruction for a different owner's ATAs, then relabelling
    // the owner-signer account back to the real wallet.
    const attacker = Keypair.generate().publicKey;
    const ix = buildAddLiquidityInstruction({ ...fixture, owner: attacker });
    ix.keys[9] = { pubkey: owner, isSigner: true, isWritable: false }; // owner-signer slot, see dlmmIx.ts account order
    const bytes = compileUnsigned([ix], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/token X destination|token Y destination/);
  });

  it("refuses to sign when an instruction targets a program not on the allow-list", () => {
    const ix = buildAddLiquidityInstruction(fixture);
    const rogueProgram = Keypair.generate().publicKey;
    const rogueIx = new TransactionInstruction({
      keys: [{ pubkey: owner, isSigner: true, isWritable: true }],
      programId: rogueProgram,
      data: Buffer.from([1, 2, 3]),
    });
    const bytes = compileUnsigned([ix, rogueIx], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/allow-list/);
  });

  it("refuses to sign when the backend built a different instruction than what was approved", () => {
    // The summary the user approved describes add-liquidity, but the actual transaction removes
    // liquidity instead.
    const removeIx = buildRemoveLiquidityInstruction({
      lbPair,
      position,
      owner,
      tokenXMint,
      tokenYMint,
      tokenXProgram: TOKEN_PROGRAM_ID,
      tokenYProgram: TOKEN_PROGRAM_ID,
      fromBinId: 80,
      toBinId: 120,
      bpsToRemove: 10_000,
    });
    const bytes = compileUnsigned([removeIx], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/removeLiquidityByRange2/);
  });

  it("refuses a transaction that requires an unexpected extra signer", () => {
    const ix = buildAddLiquidityInstruction(fixture);
    const extraSigner = Keypair.generate().publicKey;
    const decoyIx = new TransactionInstruction({
      keys: [
        { pubkey: owner, isSigner: true, isWritable: true },
        { pubkey: extraSigner, isSigner: true, isWritable: false },
      ],
      programId: SystemProgram.programId,
      data: Buffer.alloc(0),
    });
    const bytes = compileUnsigned([ix, decoyIx], owner);
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/unexpected signer/);
  });

  it("refuses a transaction that uses an address lookup table", () => {
    const ix = buildAddLiquidityInstruction(fixture);
    const legacyMessage = new TransactionMessage({
      payerKey: owner,
      instructions: [ix],
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
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toMatch(/address lookup table/);
  });
});

describe("verifyTransaction: close-position", () => {
  it("accepts a correctly built close-position transaction", () => {
    const owner = Keypair.generate().publicKey;
    const position = Keypair.generate().publicKey;
    const rentReceiver = owner;
    const ix = buildClosePositionInstruction({ position, owner, rentReceiver });
    const bytes = compileUnsigned([ix], owner);

    const expected: ExpectedAction = { kind: "close-position", position, rentReceiver };
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
    expect(result.ok).toBe(true);
  });

  it("refuses to sign when rent is redirected to an address other than what was shown", () => {
    const owner = Keypair.generate().publicKey;
    const position = Keypair.generate().publicKey;
    const attacker = Keypair.generate().publicKey;
    const ix = buildClosePositionInstruction({ position, owner, rentReceiver: attacker });
    const bytes = compileUnsigned([ix], owner);

    const expected: ExpectedAction = { kind: "close-position", position, rentReceiver: owner };
    const result = verifyTransaction(bytes, { walletPubkey: owner, expected });
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
