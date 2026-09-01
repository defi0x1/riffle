import { PublicKey } from "@solana/web3.js";
import { sha256 } from "@noble/hashes/sha256";

/**
 * Every program id and PDA seed below is transcribed from the same public Meteora DLMM IDL the
 * backend's instruction builder is transcribed from, plus the well-known SPL program ids. This
 * file has no dependency on backend code -- it is deliberately reconstructed independently, on
 * the client, because the whole point of the verifier (verify/txVerifier.ts) is to check the
 * backend's output against a fixed reference the backend cannot influence.
 */

export const DLMM_PROGRAM_ID = new PublicKey("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
export const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const TOKEN_2022_PROGRAM_ID = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
export const ASSOCIATED_TOKEN_PROGRAM_ID = new PublicKey(
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
export const SYSTEM_PROGRAM_ID = new PublicKey("11111111111111111111111111111111");
export const MEMO_PROGRAM_ID = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
export const COMPUTE_BUDGET_PROGRAM_ID = new PublicKey(
  "ComputeBudget111111111111111111111111111111",
);

/**
 * The complete set of program ids any instruction in a transaction this app is asked to sign is
 * allowed to invoke. Nothing else -- an instruction naming any other program id fails
 * verification outright, no matter how plausible its accounts or data look.
 */
export const ALLOWED_PROGRAM_IDS: ReadonlySet<string> = new Set(
  [
    DLMM_PROGRAM_ID,
    TOKEN_PROGRAM_ID,
    TOKEN_2022_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
    SYSTEM_PROGRAM_ID,
    MEMO_PROGRAM_ID,
    COMPUTE_BUDGET_PROGRAM_ID,
  ].map((pk) => pk.toBase58()),
);

export const BIN_ARRAY_SEED = new TextEncoder().encode("bin_array");
export const BIN_ARRAY_BITMAP_SEED = new TextEncoder().encode("bitmap");
export const EVENT_AUTHORITY_SEED = new TextEncoder().encode("__event_authority");

export const MAX_BIN_PER_ARRAY = 70;
export const DEFAULT_BITMAP_BIN_ARRAY_RANGE = 512;

/**
 * Anchor's account/instruction discriminator scheme: the first 8 bytes of
 * sha256("<namespace>:<name>"). Computed here rather than hard-coded as byte literals so this
 * stays self-verifying against its own description -- a transcription mistake in a literal
 * byte array is exactly the kind of error this function's approach avoids.
 */
export function anchorDiscriminator(namespace: string, name: string): Uint8Array {
  const digest = sha256(new TextEncoder().encode(`${namespace}:${name}`));
  return digest.slice(0, 8);
}

export const INSTRUCTION_DISCRIMINATORS = {
  initializePosition2: anchorDiscriminator("global", "initialize_position2"),
  addLiquidityByStrategy2: anchorDiscriminator("global", "add_liquidity_by_strategy2"),
  removeLiquidityByRange2: anchorDiscriminator("global", "remove_liquidity_by_range2"),
  claimFee2: anchorDiscriminator("global", "claim_fee2"),
  closePosition2: anchorDiscriminator("global", "close_position2"),
} as const;

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

export type DlmmInstructionName = keyof typeof INSTRUCTION_DISCRIMINATORS;

export function identifyDlmmInstruction(data: Uint8Array): DlmmInstructionName | null {
  if (data.length < 8) return null;
  const prefix = data.slice(0, 8);
  for (const [name, discriminator] of Object.entries(INSTRUCTION_DISCRIMINATORS)) {
    if (bytesEqual(prefix, discriminator)) {
      return name as DlmmInstructionName;
    }
  }
  return null;
}
