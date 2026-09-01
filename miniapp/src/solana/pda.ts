import { PublicKey } from "@solana/web3.js";

import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  BIN_ARRAY_BITMAP_SEED,
  BIN_ARRAY_SEED,
  DEFAULT_BITMAP_BIN_ARRAY_RANGE,
  DLMM_PROGRAM_ID,
  EVENT_AUTHORITY_SEED,
  MAX_BIN_PER_ARRAY,
} from "./constants";

/**
 * PDA derivations mirroring the backend's own instruction-building crate, seed for seed --
 * re-derived independently here rather than trusted from the backend's response, so the
 * verifier can check "is this the account I would have derived myself" instead of "does this
 * account look plausible."
 */

function le32(n: number): Uint8Array {
  const buf = new Uint8Array(4);
  new DataView(buf.buffer).setInt32(0, n, true);
  return buf;
}

function le64(n: bigint): Uint8Array {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigInt64(0, n, true);
  return buf;
}

/** Which BinArray index a bin id falls in: floor(bin_id / MAX_BIN_PER_ARRAY), matching Rust's
 * div_euclid (plain JS division truncates toward zero for negative numbers, so this floors
 * explicitly rather than reusing Math.trunc semantics). */
export function binIdToBinArrayIndex(binId: number): number {
  const size = MAX_BIN_PER_ARRAY;
  return Math.floor(binId / size);
}

export function eventAuthority(): PublicKey {
  return PublicKey.findProgramAddressSync([EVENT_AUTHORITY_SEED], DLMM_PROGRAM_ID)[0];
}

export function binArray(lbPair: PublicKey, index: number): PublicKey {
  return PublicKey.findProgramAddressSync(
    [BIN_ARRAY_SEED, lbPair.toBytes(), le64(BigInt(index))],
    DLMM_PROGRAM_ID,
  )[0];
}

export function binArraysCoveringRange(
  lbPair: PublicKey,
  lowerBinId: number,
  upperBinId: number,
): PublicKey[] {
  const lowerIndex = binIdToBinArrayIndex(lowerBinId);
  const upperIndex = binIdToBinArrayIndex(upperBinId);
  const arrays: PublicKey[] = [];
  for (let index = lowerIndex; index <= upperIndex; index++) {
    arrays.push(binArray(lbPair, index));
  }
  return arrays;
}

export function binArrayBitmapExtensionRequired(lowerBinId: number, upperBinId: number): boolean {
  const lowerIndex = binIdToBinArrayIndex(lowerBinId);
  const upperIndex = binIdToBinArrayIndex(upperBinId);
  return (
    lowerIndex < -DEFAULT_BITMAP_BIN_ARRAY_RANGE || upperIndex > DEFAULT_BITMAP_BIN_ARRAY_RANGE - 1
  );
}

export function binArrayBitmapExtension(lbPair: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [BIN_ARRAY_BITMAP_SEED, lbPair.toBytes()],
    DLMM_PROGRAM_ID,
  )[0];
}

/** Anchor's convention for a missing optional account is the program's own id as a sentinel. */
export function optionalBinArrayBitmapExtension(
  lbPair: PublicKey,
  lowerBinId: number,
  upperBinId: number,
): PublicKey {
  return binArrayBitmapExtensionRequired(lowerBinId, upperBinId)
    ? binArrayBitmapExtension(lbPair)
    : DLMM_PROGRAM_ID;
}

/** A pool's token reserve vault: PDA of [lb_pair, mint], no seed prefix. */
export function reserve(lbPair: PublicKey, mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([lbPair.toBytes(), mint.toBytes()], DLMM_PROGRAM_ID)[0];
}

/** Standard SPL associated-token-account derivation, valid for both the Token and Token-2022
 * programs depending on which `tokenProgram` is passed. */
export function associatedTokenAddress(
  owner: PublicKey,
  mint: PublicKey,
  tokenProgram: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [owner.toBytes(), tokenProgram.toBytes(), mint.toBytes()],
    ASSOCIATED_TOKEN_PROGRAM_ID,
  )[0];
}

// le32 is exported for instruction-argument decoding elsewhere (verify/decode.ts reads the same
// little-endian i32 layout borsh produces for lower_bin_id/upper_bin_id/etc).
export { le32 };
