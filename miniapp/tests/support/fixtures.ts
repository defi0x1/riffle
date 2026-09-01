import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { PublicKey } from "@solana/web3.js";

import type { ExpectedAction } from "../../src/verify/types";

/**
 * Loads the golden cross-language fixtures `libraries/dlmm_tx/tests/fixtures.rs` builds and
 * commits: one unsigned transaction per supported operation, built from the Rust crate's real
 * instruction builders, plus a JSON sidecar describing what the transaction is supposed to mean.
 * This is what lets these tests run the real verifier (src/verify/txVerifier.ts) against bytes
 * that actually came out of the backend's own code, instead of a second, hand-written
 * TypeScript re-implementation of the wire format that could drift from it in the same way the
 * real builder did and never notice.
 *
 * Regenerate the fixtures themselves with:
 *   cargo test -p dlmm_tx --test fixtures -- --ignored regenerate_fixtures
 */

const FIXTURE_DIR = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
  "fixtures",
  "dlmm_tx",
);

export const FIXTURE_NAMES = [
  "open_position",
  "add_liquidity",
  "remove_liquidity",
  "claim_fee",
  "close_position",
] as const;

export type FixtureName = (typeof FIXTURE_NAMES)[number];

type FixtureOperation =
  | "open-position"
  | "add-liquidity"
  | "remove-liquidity"
  | "claim-fees"
  | "close-position";

/**
 * Shape of the JSON sidecar the Rust side writes next to each `.tx.hex` fixture. Every field is
 * optional here because not every operation uses every field -- `fromRawSemantics` below is the
 * single place that knows which fields a given `operation` requires.
 */
export interface RawFixtureSemantics {
  operation: FixtureOperation;
  walletPubkey: string;
  lbPair?: string;
  tokenXMint?: string;
  tokenYMint?: string;
  tokenXProgram?: string;
  tokenYProgram?: string;
  position?: string;
  positionLowerBinId?: number;
  positionUpperBinId?: number;
  lowerBinId?: number;
  width?: number;
  amountX?: string;
  amountY?: string;
  activeId?: number;
  maxActiveBinSlippage?: number;
  minBinId?: number;
  maxBinId?: number;
  fromBinId?: number;
  toBinId?: number;
  bpsToRemove?: number;
  rentReceiver?: string;
}

export interface LoadedFixture {
  name: FixtureName;
  bytes: Uint8Array;
  semantics: RawFixtureSemantics;
  walletPubkey: PublicKey;
  expected: ExpectedAction;
}

function hexToBytes(hex: string): Uint8Array {
  const trimmed = hex.trim();
  if (trimmed.length % 2 !== 0) {
    throw new Error("fixture hex has odd length");
  }
  const out = new Uint8Array(trimmed.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(trimmed.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function requiredString(value: string | undefined, field: string, operation: string): string {
  if (value === undefined) {
    throw new Error(`fixture ${operation}: missing required field ${field}`);
  }
  return value;
}

function requiredNumber(value: number | undefined, field: string, operation: string): number {
  if (value === undefined) {
    throw new Error(`fixture ${operation}: missing required field ${field}`);
  }
  return value;
}

function requiredPubkey(value: string | undefined, field: string, operation: string): PublicKey {
  return new PublicKey(requiredString(value, field, operation));
}

/** Mirrors src/verify/fromSummary.ts, but sourced from a fixture's committed sidecar rather than
 * a backend response -- deliberately not shared code with fromSummary.ts, since fixture loading
 * is test-only infrastructure and has no business depending on (or being depended on by) the
 * production conversion path it is here to exercise. */
function expectedActionFromRawSemantics(s: RawFixtureSemantics): ExpectedAction {
  const op = s.operation;
  switch (op) {
    case "open-position":
      return {
        kind: "open-position",
        lbPair: requiredPubkey(s.lbPair, "lbPair", op),
        tokenXMint: requiredPubkey(s.tokenXMint, "tokenXMint", op),
        tokenYMint: requiredPubkey(s.tokenYMint, "tokenYMint", op),
        position: requiredPubkey(s.position, "position", op),
        lowerBinId: requiredNumber(s.lowerBinId, "lowerBinId", op),
        width: requiredNumber(s.width, "width", op),
      };
    case "add-liquidity":
      return {
        kind: "add-liquidity",
        lbPair: requiredPubkey(s.lbPair, "lbPair", op),
        position: requiredPubkey(s.position, "position", op),
        positionLowerBinId: requiredNumber(s.positionLowerBinId, "positionLowerBinId", op),
        positionUpperBinId: requiredNumber(s.positionUpperBinId, "positionUpperBinId", op),
        tokenXMint: requiredPubkey(s.tokenXMint, "tokenXMint", op),
        tokenYMint: requiredPubkey(s.tokenYMint, "tokenYMint", op),
        tokenXProgram: requiredPubkey(s.tokenXProgram, "tokenXProgram", op),
        tokenYProgram: requiredPubkey(s.tokenYProgram, "tokenYProgram", op),
        amountX: BigInt(requiredString(s.amountX, "amountX", op)),
        amountY: BigInt(requiredString(s.amountY, "amountY", op)),
        activeId: requiredNumber(s.activeId, "activeId", op),
        maxActiveBinSlippage: requiredNumber(s.maxActiveBinSlippage, "maxActiveBinSlippage", op),
        minBinId: requiredNumber(s.minBinId, "minBinId", op),
        maxBinId: requiredNumber(s.maxBinId, "maxBinId", op),
      };
    case "remove-liquidity":
      return {
        kind: "remove-liquidity",
        lbPair: requiredPubkey(s.lbPair, "lbPair", op),
        position: requiredPubkey(s.position, "position", op),
        positionLowerBinId: requiredNumber(s.positionLowerBinId, "positionLowerBinId", op),
        positionUpperBinId: requiredNumber(s.positionUpperBinId, "positionUpperBinId", op),
        tokenXMint: requiredPubkey(s.tokenXMint, "tokenXMint", op),
        tokenYMint: requiredPubkey(s.tokenYMint, "tokenYMint", op),
        tokenXProgram: requiredPubkey(s.tokenXProgram, "tokenXProgram", op),
        tokenYProgram: requiredPubkey(s.tokenYProgram, "tokenYProgram", op),
        fromBinId: requiredNumber(s.fromBinId, "fromBinId", op),
        toBinId: requiredNumber(s.toBinId, "toBinId", op),
        bpsToRemove: requiredNumber(s.bpsToRemove, "bpsToRemove", op),
      };
    case "claim-fees":
      return {
        kind: "claim-fees",
        lbPair: requiredPubkey(s.lbPair, "lbPair", op),
        position: requiredPubkey(s.position, "position", op),
        positionLowerBinId: requiredNumber(s.positionLowerBinId, "positionLowerBinId", op),
        positionUpperBinId: requiredNumber(s.positionUpperBinId, "positionUpperBinId", op),
        tokenXMint: requiredPubkey(s.tokenXMint, "tokenXMint", op),
        tokenYMint: requiredPubkey(s.tokenYMint, "tokenYMint", op),
        tokenXProgram: requiredPubkey(s.tokenXProgram, "tokenXProgram", op),
        tokenYProgram: requiredPubkey(s.tokenYProgram, "tokenYProgram", op),
        minBinId: requiredNumber(s.minBinId, "minBinId", op),
        maxBinId: requiredNumber(s.maxBinId, "maxBinId", op),
      };
    case "close-position":
      return {
        kind: "close-position",
        position: requiredPubkey(s.position, "position", op),
        rentReceiver: requiredPubkey(s.rentReceiver, "rentReceiver", op),
      };
  }
}

export function loadFixture(name: FixtureName): LoadedFixture {
  const bytes = hexToBytes(readFileSync(path.join(FIXTURE_DIR, `${name}.tx.hex`), "utf8"));
  const semantics = JSON.parse(
    readFileSync(path.join(FIXTURE_DIR, `${name}.json`), "utf8"),
  ) as RawFixtureSemantics;
  const walletPubkey = requiredPubkey(semantics.walletPubkey, "walletPubkey", semantics.operation);
  return { name, bytes, semantics, walletPubkey, expected: expectedActionFromRawSemantics(semantics) };
}
