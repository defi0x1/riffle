import { PublicKey, VersionedTransaction } from "@solana/web3.js";

import {
  ALLOWED_PROGRAM_IDS,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  COMPUTE_BUDGET_PROGRAM_ID,
  DLMM_PROGRAM_ID,
  MEMO_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  identifyDlmmInstruction,
} from "../solana/constants";
import {
  associatedTokenAddress,
  binArraysCoveringRange,
  eventAuthority,
  optionalBinArrayBitmapExtension,
  reserve,
} from "../solana/pda";
import {
  decodeAddLiquidityArgs,
  decodeClaimFeeArgs,
  decodeClosePositionArgs,
  decodeOpenPositionArgs,
  decodeRemoveLiquidityArgs,
} from "./decode";
import type { DecodedSummary, ExpectedAction, VerificationContext, VerificationResult } from "./types";

/**
 * This is the control the rest of the custody model leans on: the backend builds every
 * transaction, so nothing stops it (or a hostile RPC in the middle, or a compromised dependency
 * in the build pipeline) from asking this app to sign something other than what the UI showed.
 * Everything here re-derives what the transaction *should* contain from data the UI already
 * displayed and confirms the actual bytes match, byte for byte on every account and argument
 * that touches funds, ownership, or program identity. Any mismatch is a hard failure -- no
 * partial credit, no "looks close enough."
 *
 * What this deliberately does NOT catch, stated plainly rather than left implied:
 *  - A pool that is legitimately what it claims to be but is itself malicious or mispriced
 *    (e.g. a rug-pull token, a manipulated active bin) -- this checks the transaction matches
 *    the summary the user was shown, not that the summary itself describes a good trade.
 *  - A transaction using an address lookup table is rejected outright rather than resolved, so
 *    it never gets this far -- see the ALT check below for why.
 *  - Anything about a already-signed, already-submitted transaction being replaced, delayed, or
 *    censored after this app hands it off -- that is a submission-layer concern (see the API
 *    contract's idempotency notes in the README), not something a pre-sign byte check can see.
 *  - SOL-wrapping instructions (System transfer + Token SyncNative) are not recognised at all;
 *    a build response that includes them fails closed as an unrecognised instruction rather than
 *    being accepted. The HTTP contract this app expects (README.md) requires the backend to
 *    assume any wrapped-SOL ATA the flow needs already exists, specifically so this verifier
 *    never has to make a judgment call about a wrapping instruction's amount being "close
 *    enough" to what was shown.
 *  - A logic bug in this file itself. This is code like any other; treat "the verifier passed"
 *    as strong evidence, not proof.
 */

function pkEq(a: PublicKey, b: PublicKey): boolean {
  return a.equals(b);
}

function amountWithinTolerance(actual: bigint, expected: bigint, toleranceBps: number): boolean {
  if (toleranceBps <= 0) return actual === expected;
  const delta = actual > expected ? actual - expected : expected - actual;
  const allowed = (expected * BigInt(toleranceBps)) / 10_000n;
  return delta <= allowed;
}

interface ResolvedInstruction {
  programId: PublicKey;
  accounts: PublicKey[];
  data: Uint8Array;
}

function resolveInstructions(tx: VersionedTransaction): ResolvedInstruction[] {
  const message = tx.message;
  const keys = message.staticAccountKeys;
  return message.compiledInstructions.map((ix) => {
    const programId = keys[ix.programIdIndex];
    if (!programId) {
      throw new Error(`instruction references out-of-range program id index ${ix.programIdIndex}`);
    }
    const accounts = ix.accountKeyIndexes.map((i) => {
      const key = keys[i];
      if (!key) {
        throw new Error(`instruction references out-of-range account index ${i}`);
      }
      return key;
    });
    return { programId, accounts, data: ix.data };
  });
}

function checkComputeBudgetInstruction(
  ix: ResolvedInstruction,
  maxUnitPriceMicroLamports: number | undefined,
): string | null {
  const tag = ix.data[0];
  if (tag === 2) {
    // SetComputeUnitLimit(u32) -- a limit is never a fund-movement risk, nothing further to check.
    return null;
  }
  if (tag === 3) {
    if (ix.data.length < 9) return "malformed SetComputeUnitPrice instruction";
    const view = new DataView(ix.data.buffer, ix.data.byteOffset + 1, 8);
    const price = view.getBigUint64(0, true);
    if (maxUnitPriceMicroLamports !== undefined && price > BigInt(maxUnitPriceMicroLamports)) {
      return `compute unit price ${price} exceeds the configured ceiling ${maxUnitPriceMicroLamports}`;
    }
    return null;
  }
  return `unexpected ComputeBudget instruction tag ${tag}`;
}

function checkAtaCreateInstruction(
  ix: ResolvedInstruction,
  wallet: PublicKey,
  allowedMints: PublicKey[],
): string | null {
  const tag = ix.data.length === 0 ? 0 : ix.data[0];
  if (ix.data.length > 1 || (ix.data.length === 1 && tag !== 1)) {
    return "unexpected associated-token-account instruction (only create/create-idempotent allowed)";
  }
  const [payer, ata, owner, mint, systemProgram, tokenProgram] = ix.accounts;
  if (!payer || !ata || !owner || !mint || !systemProgram || !tokenProgram) {
    return "associated-token-account instruction has too few accounts";
  }
  if (!pkEq(payer, wallet)) return "ATA creation funded by an account other than this wallet";
  if (!pkEq(owner, wallet)) return "ATA creation for an owner other than this wallet";
  if (!allowedMints.some((m) => pkEq(m, mint))) return "ATA creation for an unexpected mint";
  if (!pkEq(ata, associatedTokenAddress(owner, mint, tokenProgram))) {
    return "ATA creation targets an address that does not match owner+mint+token-program";
  }
  if (!pkEq(systemProgram, SYSTEM_PROGRAM_ID)) return "ATA creation references the wrong system program";
  return null;
}

function verifyOpenPosition(
  ix: ResolvedInstruction,
  expected: Extract<ExpectedAction, { kind: "open-position" }>,
  wallet: PublicKey,
): string | null {
  const args = decodeOpenPositionArgs(ix.data);
  if (args.lowerBinId !== expected.lowerBinId) return "lower bin id does not match what was shown";
  if (args.width !== expected.width) return "position width does not match what was shown";

  const [payer, position, lbPair, owner, systemProgram, evtAuthority, dlmmSelf] = ix.accounts;
  if (ix.accounts.length !== 7 || !payer || !position || !lbPair || !owner || !systemProgram || !evtAuthority || !dlmmSelf) {
    return "unexpected account list for open-position";
  }
  if (!pkEq(payer, wallet)) return "payer is not this wallet";
  if (!pkEq(position, expected.position)) return "position account does not match the one generated for this request";
  if (!pkEq(lbPair, expected.lbPair)) return "pool address does not match what was shown";
  if (!pkEq(owner, wallet)) return "owner is not this wallet";
  if (!pkEq(systemProgram, SYSTEM_PROGRAM_ID)) return "wrong system program account";
  if (!pkEq(evtAuthority, eventAuthority())) return "wrong event-authority account";
  if (!pkEq(dlmmSelf, DLMM_PROGRAM_ID)) return "wrong self-referenced program account";
  return null;
}

function verifyAddLiquidity(
  ix: ResolvedInstruction,
  expected: Extract<ExpectedAction, { kind: "add-liquidity" }>,
  wallet: PublicKey,
): string | null {
  const args = decodeAddLiquidityArgs(ix.data);
  const toleranceBps = expected.amountToleranceBps ?? 0;
  if (!amountWithinTolerance(args.amountX, expected.amountX, toleranceBps)) return "token X amount does not match what was shown";
  if (!amountWithinTolerance(args.amountY, expected.amountY, toleranceBps)) return "token Y amount does not match what was shown";
  if (args.activeId !== expected.activeId) return "active bin does not match what was shown";
  if (args.maxActiveBinSlippage !== expected.maxActiveBinSlippage) return "slippage bound does not match what was shown";
  if (args.minBinId !== expected.minBinId || args.maxBinId !== expected.maxBinId) return "deposit bin range does not match what was shown";
  if (args.minBinId < expected.positionLowerBinId || args.maxBinId > expected.positionUpperBinId) {
    return "deposit range extends outside this position's own bin range";
  }

  const [position, lbPair, bitmapExt, userX, userY, reserveX, reserveY, mintX, mintY, owner, tokenProgX, tokenProgY, evtAuthority, dlmmSelf, ...binArrays] = ix.accounts;
  if (!position || !lbPair || !bitmapExt || !userX || !userY || !reserveX || !reserveY || !mintX || !mintY || !owner || !tokenProgX || !tokenProgY || !evtAuthority || !dlmmSelf) {
    return "unexpected account list for add-liquidity";
  }
  if (!pkEq(position, expected.position)) return "position account does not match the one this wallet controls";
  if (!pkEq(lbPair, expected.lbPair)) return "pool address does not match what was shown";
  if (!pkEq(bitmapExt, optionalBinArrayBitmapExtension(lbPair, args.minBinId, args.maxBinId))) return "wrong bin-array-bitmap-extension account";
  if (!pkEq(mintX, expected.tokenXMint) || !pkEq(mintY, expected.tokenYMint)) return "token mint does not match what was shown";
  if (!pkEq(tokenProgX, expected.tokenXProgram) || !pkEq(tokenProgY, expected.tokenYProgram)) return "wrong token program for one side";
  if (!pkEq(userX, associatedTokenAddress(wallet, mintX, tokenProgX))) return "token X destination is not this wallet's own token account";
  if (!pkEq(userY, associatedTokenAddress(wallet, mintY, tokenProgY))) return "token Y destination is not this wallet's own token account";
  if (!pkEq(reserveX, reserve(lbPair, mintX)) || !pkEq(reserveY, reserve(lbPair, mintY))) return "wrong pool reserve account";
  if (!pkEq(owner, wallet)) return "owner is not this wallet";
  if (!pkEq(evtAuthority, eventAuthority())) return "wrong event-authority account";
  if (!pkEq(dlmmSelf, DLMM_PROGRAM_ID)) return "wrong self-referenced program account";

  const expectedBinArrays = binArraysCoveringRange(lbPair, args.minBinId, args.maxBinId);
  if (binArrays.length !== expectedBinArrays.length || !binArrays.every((b, i) => pkEq(b, expectedBinArrays[i] as PublicKey))) {
    return "bin array accounts do not match the deposit range";
  }
  return null;
}

function verifyRemoveLiquidity(
  ix: ResolvedInstruction,
  expected: Extract<ExpectedAction, { kind: "remove-liquidity" }>,
  wallet: PublicKey,
): string | null {
  const args = decodeRemoveLiquidityArgs(ix.data);
  if (args.fromBinId !== expected.fromBinId || args.toBinId !== expected.toBinId) return "withdrawal bin range does not match what was shown";
  if (args.bpsToRemove !== expected.bpsToRemove) return "withdrawal fraction does not match what was shown";
  if (args.fromBinId < expected.positionLowerBinId || args.toBinId > expected.positionUpperBinId) {
    return "withdrawal range extends outside this position's own bin range";
  }

  const [position, lbPair, bitmapExt, userX, userY, reserveX, reserveY, mintX, mintY, owner, tokenProgX, tokenProgY, memoProgram, evtAuthority, dlmmSelf, ...binArrays] = ix.accounts;
  if (!position || !lbPair || !bitmapExt || !userX || !userY || !reserveX || !reserveY || !mintX || !mintY || !owner || !tokenProgX || !tokenProgY || !memoProgram || !evtAuthority || !dlmmSelf) {
    return "unexpected account list for remove-liquidity";
  }
  if (!pkEq(position, expected.position)) return "position account does not match the one this wallet controls";
  if (!pkEq(lbPair, expected.lbPair)) return "pool address does not match what was shown";
  if (!pkEq(bitmapExt, optionalBinArrayBitmapExtension(lbPair, args.fromBinId, args.toBinId))) return "wrong bin-array-bitmap-extension account";
  if (!pkEq(mintX, expected.tokenXMint) || !pkEq(mintY, expected.tokenYMint)) return "token mint does not match what was shown";
  if (!pkEq(tokenProgX, expected.tokenXProgram) || !pkEq(tokenProgY, expected.tokenYProgram)) return "wrong token program for one side";
  if (!pkEq(userX, associatedTokenAddress(wallet, mintX, tokenProgX))) return "token X destination is not this wallet's own token account";
  if (!pkEq(userY, associatedTokenAddress(wallet, mintY, tokenProgY))) return "token Y destination is not this wallet's own token account";
  if (!pkEq(reserveX, reserve(lbPair, mintX)) || !pkEq(reserveY, reserve(lbPair, mintY))) return "wrong pool reserve account";
  if (!pkEq(owner, wallet)) return "owner is not this wallet";
  if (!pkEq(memoProgram, MEMO_PROGRAM_ID)) return "wrong memo program account";
  if (!pkEq(evtAuthority, eventAuthority())) return "wrong event-authority account";
  if (!pkEq(dlmmSelf, DLMM_PROGRAM_ID)) return "wrong self-referenced program account";

  const expectedBinArrays = binArraysCoveringRange(lbPair, args.fromBinId, args.toBinId);
  if (binArrays.length !== expectedBinArrays.length || !binArrays.every((b, i) => pkEq(b, expectedBinArrays[i] as PublicKey))) {
    return "bin array accounts do not match the withdrawal range";
  }
  return null;
}

function verifyClaimFees(
  ix: ResolvedInstruction,
  expected: Extract<ExpectedAction, { kind: "claim-fees" }>,
  wallet: PublicKey,
): string | null {
  const args = decodeClaimFeeArgs(ix.data);
  if (args.minBinId !== expected.minBinId || args.maxBinId !== expected.maxBinId) return "claim range does not match what was shown";
  if (args.minBinId < expected.positionLowerBinId || args.maxBinId > expected.positionUpperBinId) {
    return "claim range extends outside this position's own bin range";
  }

  const [lbPair, position, owner, reserveX, reserveY, userX, userY, mintX, mintY, tokenProgX, tokenProgY, memoProgram, evtAuthority, dlmmSelf, ...binArrays] = ix.accounts;
  if (!lbPair || !position || !owner || !reserveX || !reserveY || !userX || !userY || !mintX || !mintY || !tokenProgX || !tokenProgY || !memoProgram || !evtAuthority || !dlmmSelf) {
    return "unexpected account list for claim-fees";
  }
  if (!pkEq(lbPair, expected.lbPair)) return "pool address does not match what was shown";
  if (!pkEq(position, expected.position)) return "position account does not match the one this wallet controls";
  if (!pkEq(owner, wallet)) return "owner is not this wallet";
  if (!pkEq(mintX, expected.tokenXMint) || !pkEq(mintY, expected.tokenYMint)) return "token mint does not match what was shown";
  if (!pkEq(tokenProgX, expected.tokenXProgram) || !pkEq(tokenProgY, expected.tokenYProgram)) return "wrong token program for one side";
  if (!pkEq(userX, associatedTokenAddress(wallet, mintX, tokenProgX))) return "token X destination is not this wallet's own token account";
  if (!pkEq(userY, associatedTokenAddress(wallet, mintY, tokenProgY))) return "token Y destination is not this wallet's own token account";
  if (!pkEq(reserveX, reserve(lbPair, mintX)) || !pkEq(reserveY, reserve(lbPair, mintY))) return "wrong pool reserve account";
  if (!pkEq(memoProgram, MEMO_PROGRAM_ID)) return "wrong memo program account";
  if (!pkEq(evtAuthority, eventAuthority())) return "wrong event-authority account";
  if (!pkEq(dlmmSelf, DLMM_PROGRAM_ID)) return "wrong self-referenced program account";

  const expectedBinArrays = binArraysCoveringRange(lbPair, args.minBinId, args.maxBinId);
  if (binArrays.length !== expectedBinArrays.length || !binArrays.every((b, i) => pkEq(b, expectedBinArrays[i] as PublicKey))) {
    return "bin array accounts do not match the claim range";
  }
  return null;
}

function verifyClosePosition(
  ix: ResolvedInstruction,
  expected: Extract<ExpectedAction, { kind: "close-position" }>,
  wallet: PublicKey,
): string | null {
  decodeClosePositionArgs(ix.data);
  const [position, owner, rentReceiver, evtAuthority, dlmmSelf] = ix.accounts;
  if (ix.accounts.length !== 5 || !position || !owner || !rentReceiver || !evtAuthority || !dlmmSelf) {
    return "unexpected account list for close-position";
  }
  if (!pkEq(position, expected.position)) return "position account does not match what was shown";
  if (!pkEq(owner, wallet)) return "owner is not this wallet";
  if (!pkEq(rentReceiver, expected.rentReceiver)) return "rent receiver does not match what was shown";
  if (!pkEq(evtAuthority, eventAuthority())) return "wrong event-authority account";
  if (!pkEq(dlmmSelf, DLMM_PROGRAM_ID)) return "wrong self-referenced program account";
  return null;
}

const EXPECTED_INSTRUCTION_BY_KIND: Record<ExpectedAction["kind"], string> = {
  "open-position": "initializePosition2",
  "add-liquidity": "addLiquidityByStrategy2",
  "remove-liquidity": "removeLiquidityByRange2",
  "claim-fees": "claimFee2",
  "close-position": "closePosition2",
};

function allowedMintsFor(expected: ExpectedAction): PublicKey[] {
  if (expected.kind === "open-position") return [expected.tokenXMint, expected.tokenYMint];
  if (expected.kind === "close-position") return [];
  return [expected.tokenXMint, expected.tokenYMint];
}

export function verifyTransaction(
  unsignedTxBytes: Uint8Array,
  context: VerificationContext,
): VerificationResult {
  let tx: VersionedTransaction;
  try {
    tx = VersionedTransaction.deserialize(unsignedTxBytes);
  } catch (err) {
    return { ok: false, reason: `could not decode transaction bytes: ${(err as Error).message}` };
  }

  const message = tx.message;

  // An address lookup table resolves extra accounts at execution time from an on-chain table
  // this app has not read -- verifying against it would mean trusting the same untrusted source
  // (the backend, or its RPC) that supplied the lookup table address in the first place. None of
  // this app's own flows need one (account counts stay well under the legacy-message limit), so
  // any transaction that uses one is refused outright rather than partially checked.
  if (message.addressTableLookups.length > 0) {
    return { ok: false, reason: "transaction uses an address lookup table, which this app does not resolve or trust" };
  }

  let instructions: ResolvedInstruction[];
  try {
    instructions = resolveInstructions(tx);
  } catch (err) {
    return { ok: false, reason: (err as Error).message };
  }

  if (instructions.length === 0) {
    return { ok: false, reason: "transaction has no instructions" };
  }

  for (const ix of instructions) {
    if (!ALLOWED_PROGRAM_IDS.has(ix.programId.toBase58())) {
      return { ok: false, reason: `instruction targets a program not on the allow-list: ${ix.programId.toBase58()}` };
    }
  }

  // Every required signer must be an account this wallet controls or expects to co-sign with
  // (the ephemeral position key, open-position only). A required signer this app did not expect
  // is refused even if everything else about the transaction looks fine -- an unexpected signer
  // requirement is itself a sign something is wrong, not a detail to wave through.
  const keys = message.staticAccountKeys;
  const numSigners = message.header.numRequiredSignatures;
  const requiredSigners = keys.slice(0, numSigners);
  const expectedSigners = new Set<string>([context.walletPubkey.toBase58()]);
  if (context.expected.kind === "open-position") {
    expectedSigners.add(context.expected.position.toBase58());
  }
  for (const signer of requiredSigners) {
    if (!signer) return { ok: false, reason: "malformed signer list" };
    if (!expectedSigners.has(signer.toBase58())) {
      return { ok: false, reason: `transaction requires an unexpected signer: ${signer.toBase58()}` };
    }
  }
  if (!requiredSigners.some((s) => s?.equals(context.walletPubkey))) {
    return { ok: false, reason: "transaction does not require this wallet's signature at all" };
  }

  const dlmmInstructions = instructions.filter((ix) => ix.programId.equals(DLMM_PROGRAM_ID));
  if (dlmmInstructions.length !== 1) {
    return { ok: false, reason: `expected exactly one DLMM instruction, found ${dlmmInstructions.length}` };
  }
  const dlmmIx = dlmmInstructions[0] as ResolvedInstruction;
  const identified = identifyDlmmInstruction(dlmmIx.data);
  const expectedName = EXPECTED_INSTRUCTION_BY_KIND[context.expected.kind];
  if (identified !== expectedName) {
    return {
      ok: false,
      reason: `backend built a ${identified ?? "non-DLMM"} instruction, but the user approved a ${context.expected.kind} summary`,
    };
  }

  for (const ix of instructions) {
    if (ix.programId.equals(COMPUTE_BUDGET_PROGRAM_ID)) {
      const err = checkComputeBudgetInstruction(ix, context.maxComputeUnitPriceMicroLamports);
      if (err) return { ok: false, reason: err };
    }
    if (ix.programId.equals(ASSOCIATED_TOKEN_PROGRAM_ID)) {
      const err = checkAtaCreateInstruction(ix, context.walletPubkey, allowedMintsFor(context.expected));
      if (err) return { ok: false, reason: err };
    }
  }

  let dlmmError: string | null;
  switch (context.expected.kind) {
    case "open-position":
      dlmmError = verifyOpenPosition(dlmmIx, context.expected, context.walletPubkey);
      break;
    case "add-liquidity":
      dlmmError = verifyAddLiquidity(dlmmIx, context.expected, context.walletPubkey);
      break;
    case "remove-liquidity":
      dlmmError = verifyRemoveLiquidity(dlmmIx, context.expected, context.walletPubkey);
      break;
    case "claim-fees":
      dlmmError = verifyClaimFees(dlmmIx, context.expected, context.walletPubkey);
      break;
    case "close-position":
      dlmmError = verifyClosePosition(dlmmIx, context.expected, context.walletPubkey);
      break;
  }
  if (dlmmError) {
    return { ok: false, reason: dlmmError };
  }

  const decoded: DecodedSummary = {
    instructionCount: instructions.length,
    dlmmAction: expectedName,
    programIds: instructions.map((ix) => ix.programId.toBase58()),
  };
  return { ok: true, decoded };
}
