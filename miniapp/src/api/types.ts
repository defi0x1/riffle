/**
 * The HTTP contract this app expects from the backend. Nothing on the backend exists yet --
 * these types are the interface the backend is expected to be built against, not a description
 * of something already running. See README.md for the full endpoint list, the idempotency and
 * expiry rules, and why every one of these shapes has no field capable of carrying key material.
 */

export interface RegisterWalletRequest {
  pubkey: string; // base58
}

export interface RegisterWalletResponse {
  registeredAt: string; // ISO 8601
}

export interface TokenBalance {
  mint: string;
  programId: string;
  amountRaw: string; // base-10 string, not a float -- avoids precision loss on large token amounts
  decimals: number;
}

export interface BalancesResponse {
  solLamports: string;
  tokens: TokenBalance[];
}

export type PositionStatus = "open" | "closed";

export interface PositionSummary {
  positionAddress: string;
  poolAddress: string;
  status: PositionStatus;
  lowerBinId: number;
  upperBinId: number;
  openedAt: string;
  closedAt: string | null;
  feesXPending: string;
  feesYPending: string;
}

export interface PositionsResponse {
  positions: PositionSummary[];
}

/**
 * The load-bearing part of every build-tx response: `summary` is what the UI renders and what
 * gets fed into verify/txVerifier.ts as the expected shape, and `unsignedTransaction` is what
 * gets independently decoded and checked against it. Both are populated by the same backend
 * request, deliberately -- the point of the verifier is that they might disagree.
 */
export interface BuildTxResponseBase {
  unsignedTransaction: string; // base64, wire-format Transaction or VersionedTransaction bytes
  expiryBlockhash: string;
  expiryLastValidBlockHeight: number;
  idempotencyKey: string;
  /** The backend's own simulateTransaction result -- a UX courtesy shown before prompting for a
   * passphrase, never trusted as the sole gate (see README). This app simulates again,
   * independently, immediately before signing. */
  simulation: {
    success: boolean;
    error: string | null;
    logsTail: string[];
  };
  estimatedNetworkFeeLamports: string;
}

// Every field below that verify/txVerifier.ts needs to build its ExpectedAction is present here
// deliberately, including ones a summary UI would not otherwise show (token program ids, the
// position's own bin range) -- the verifier is built to derive its expectations entirely from
// what the backend claims in `summary`, the same object rendered on screen, so there is exactly
// one source of "what the user was told" for both the display and the check.
export interface OpenPositionSummary {
  action: "open-position";
  poolAddress: string;
  tokenXMint: string;
  tokenYMint: string;
  tokenXSymbol: string;
  tokenYSymbol: string;
  lowerBinId: number;
  width: number;
  ephemeralPositionPubkey: string;
}

export interface AddLiquiditySummary {
  action: "add-liquidity";
  poolAddress: string;
  positionAddress: string;
  positionLowerBinId: number;
  positionUpperBinId: number;
  tokenXMint: string;
  tokenYMint: string;
  tokenXProgram: string;
  tokenYProgram: string;
  tokenXSymbol: string;
  tokenYSymbol: string;
  amountXRaw: string;
  amountYRaw: string;
  amountXUsd: number | null;
  amountYUsd: number | null;
  activeId: number;
  maxActiveBinSlippageBps: number;
  minBinId: number;
  maxBinId: number;
}

export interface RemoveLiquiditySummary {
  action: "remove-liquidity";
  poolAddress: string;
  positionAddress: string;
  positionLowerBinId: number;
  positionUpperBinId: number;
  tokenXMint: string;
  tokenYMint: string;
  tokenXProgram: string;
  tokenYProgram: string;
  fromBinId: number;
  toBinId: number;
  bpsToRemove: number;
}

export interface ClaimFeesSummary {
  action: "claim-fees";
  poolAddress: string;
  positionAddress: string;
  positionLowerBinId: number;
  positionUpperBinId: number;
  tokenXMint: string;
  tokenYMint: string;
  tokenXProgram: string;
  tokenYProgram: string;
  minBinId: number;
  maxBinId: number;
  estimatedFeesXRaw: string;
  estimatedFeesYRaw: string;
}

export interface ClosePositionSummary {
  action: "close-position";
  positionAddress: string;
  rentReceiver: string;
}

export type TxSummary =
  | OpenPositionSummary
  | AddLiquiditySummary
  | RemoveLiquiditySummary
  | ClaimFeesSummary
  | ClosePositionSummary;

export interface BuildTxResponse extends BuildTxResponseBase {
  summary: TxSummary;
}

export interface OpenPositionRequest {
  poolAddress: string;
  lowerBinId: number;
  width: number;
  ephemeralPositionPubkey: string; // generated client-side, see README's ephemeral-key note
  idempotencyKey: string;
}

export interface AddLiquidityRequest {
  poolAddress: string;
  positionAddress: string;
  amountXRaw: string;
  amountYRaw: string;
  maxActiveBinSlippageBps: number;
  minBinId: number;
  maxBinId: number;
  strategy: "spot-balanced"; // the only shape the simple flow exposes at launch
  idempotencyKey: string;
}

export interface RemoveLiquidityRequest {
  poolAddress: string;
  positionAddress: string;
  fromBinId: number;
  toBinId: number;
  bpsToRemove: number;
  idempotencyKey: string;
}

export interface ClaimFeesRequest {
  poolAddress: string;
  positionAddress: string;
  minBinId: number;
  maxBinId: number;
  idempotencyKey: string;
}

export interface ClosePositionRequest {
  positionAddress: string;
  idempotencyKey: string;
}

export interface SubmitTxRequest {
  signedTransaction: string; // base64
  idempotencyKey: string;
}

export type TxStatus = "submitted" | "confirmed" | "failed" | "expired";

export interface SubmitTxResponse {
  signature: string;
  status: TxStatus;
}

export interface TxStatusResponse {
  signature: string;
  status: TxStatus;
  error: string | null;
}

export interface ApiErrorBody {
  error: string;
  code: string;
}
