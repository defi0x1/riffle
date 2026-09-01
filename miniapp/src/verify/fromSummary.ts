import { PublicKey } from "@solana/web3.js";

import type { TxSummary } from "../api/types";
import type { ExpectedAction } from "./types";

/**
 * Turns the backend's own claimed summary (the same object the review screen renders) into the
 * expectation verify/txVerifier.ts checks the raw transaction bytes against. Kept as one small,
 * reviewable function rather than inlined at each call site, since a bug here would quietly
 * widen what the verifier accepts.
 */
export function expectedActionFromSummary(summary: TxSummary): ExpectedAction {
  switch (summary.action) {
    case "open-position":
      return {
        kind: "open-position",
        lbPair: new PublicKey(summary.poolAddress),
        tokenXMint: new PublicKey(summary.tokenXMint),
        tokenYMint: new PublicKey(summary.tokenYMint),
        position: new PublicKey(summary.ephemeralPositionPubkey),
        lowerBinId: summary.lowerBinId,
        width: summary.width,
      };
    case "add-liquidity":
      return {
        kind: "add-liquidity",
        lbPair: new PublicKey(summary.poolAddress),
        position: new PublicKey(summary.positionAddress),
        positionLowerBinId: summary.positionLowerBinId,
        positionUpperBinId: summary.positionUpperBinId,
        tokenXMint: new PublicKey(summary.tokenXMint),
        tokenYMint: new PublicKey(summary.tokenYMint),
        tokenXProgram: new PublicKey(summary.tokenXProgram),
        tokenYProgram: new PublicKey(summary.tokenYProgram),
        amountX: BigInt(summary.amountXRaw),
        amountY: BigInt(summary.amountYRaw),
        activeId: summary.activeId,
        maxActiveBinSlippage: summary.maxActiveBinSlippageBps,
        minBinId: summary.minBinId,
        maxBinId: summary.maxBinId,
      };
    case "remove-liquidity":
      return {
        kind: "remove-liquidity",
        lbPair: new PublicKey(summary.poolAddress),
        position: new PublicKey(summary.positionAddress),
        positionLowerBinId: summary.positionLowerBinId,
        positionUpperBinId: summary.positionUpperBinId,
        tokenXMint: new PublicKey(summary.tokenXMint),
        tokenYMint: new PublicKey(summary.tokenYMint),
        tokenXProgram: new PublicKey(summary.tokenXProgram),
        tokenYProgram: new PublicKey(summary.tokenYProgram),
        fromBinId: summary.fromBinId,
        toBinId: summary.toBinId,
        bpsToRemove: summary.bpsToRemove,
      };
    case "claim-fees":
      return {
        kind: "claim-fees",
        lbPair: new PublicKey(summary.poolAddress),
        position: new PublicKey(summary.positionAddress),
        positionLowerBinId: summary.positionLowerBinId,
        positionUpperBinId: summary.positionUpperBinId,
        tokenXMint: new PublicKey(summary.tokenXMint),
        tokenYMint: new PublicKey(summary.tokenYMint),
        tokenXProgram: new PublicKey(summary.tokenXProgram),
        tokenYProgram: new PublicKey(summary.tokenYProgram),
        minBinId: summary.minBinId,
        maxBinId: summary.maxBinId,
      };
    case "close-position":
      return {
        kind: "close-position",
        position: new PublicKey(summary.positionAddress),
        rentReceiver: new PublicKey(summary.rentReceiver),
      };
  }
}
