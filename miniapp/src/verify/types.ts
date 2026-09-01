import type { PublicKey } from "@solana/web3.js";

/**
 * Everything the verifier checks a transaction against, derived from what the UI already showed
 * the user before this runs -- never from the backend's own claims about itself. See
 * txVerifier.ts's module comment for the full checklist this type supports.
 *
 * No variant carries its own "owner" field: the owner/payer/sender this transaction must be
 * signed by is always VerificationContext.walletPubkey, uniformly across every action kind --
 * carrying a second, per-action owner field here would just be a place for a caller to (or an
 * accidental copy-paste bug to) pass something other than the wallet actually signing, which is
 * exactly the kind of divergence this type exists to prevent.
 */
export type ExpectedAction =
  | {
      kind: "open-position";
      lbPair: PublicKey;
      tokenXMint: PublicKey;
      tokenYMint: PublicKey;
      position: PublicKey; // ephemeral pubkey generated client-side for this request
      lowerBinId: number;
      width: number;
    }
  | {
      kind: "add-liquidity";
      lbPair: PublicKey;
      position: PublicKey;
      positionLowerBinId: number;
      positionUpperBinId: number;
      tokenXMint: PublicKey;
      tokenYMint: PublicKey;
      tokenXProgram: PublicKey;
      tokenYProgram: PublicKey;
      amountX: bigint;
      amountY: bigint;
      activeId: number;
      maxActiveBinSlippage: number;
      minBinId: number;
      maxBinId: number;
      amountToleranceBps?: number;
    }
  | {
      kind: "remove-liquidity";
      lbPair: PublicKey;
      position: PublicKey;
      positionLowerBinId: number;
      positionUpperBinId: number;
      tokenXMint: PublicKey;
      tokenYMint: PublicKey;
      tokenXProgram: PublicKey;
      tokenYProgram: PublicKey;
      fromBinId: number;
      toBinId: number;
      bpsToRemove: number;
    }
  | {
      kind: "claim-fees";
      lbPair: PublicKey;
      position: PublicKey;
      positionLowerBinId: number;
      positionUpperBinId: number;
      tokenXMint: PublicKey;
      tokenYMint: PublicKey;
      tokenXProgram: PublicKey;
      tokenYProgram: PublicKey;
      minBinId: number;
      maxBinId: number;
    }
  | {
      kind: "close-position";
      position: PublicKey;
      rentReceiver: PublicKey;
    };

export interface VerificationContext {
  /** The wallet about to sign. Every account this transaction asks the wallet to sign as
   * owner/payer/sender must equal this, or verification fails. */
  walletPubkey: PublicKey;
  expected: ExpectedAction;
  /** Maximum allowed compute-unit price, in micro-lamports -- an advisory ceiling on network
   * fee, not a security control (see README's HTTP contract notes on why a cap here cannot be
   * enforced against a determined actor); still worth catching an absurd value before signing. */
  maxComputeUnitPriceMicroLamports?: number;
}

export interface DecodedSummary {
  instructionCount: number;
  dlmmAction: string;
  programIds: string[];
}

export type VerificationResult =
  | { ok: true; decoded: DecodedSummary }
  | { ok: false; reason: string };
