import { PublicKey, TransactionInstruction } from "@solana/web3.js";

import { DLMM_PROGRAM_ID, MEMO_PROGRAM_ID, SYSTEM_PROGRAM_ID, anchorDiscriminator } from "../../src/solana/constants";
import { associatedTokenAddress, binArraysCoveringRange, eventAuthority, optionalBinArrayBitmapExtension, reserve } from "../../src/solana/pda";

/**
 * Test-only instruction encoders, deliberately re-implemented from the same public IDL account
 * orders and argument layouts verify/txVerifier.ts checks against -- not imported from src/verify
 * or src/solana, so a bug shared between the encoder and the decoder would not silently cancel
 * out and produce a false "matches" in these tests. Where this and the production decoder must
 * agree (account order, argument layout) is exactly what a real backend implementation has to
 * get right too, so this doubles as a runnable description of the wire format the backend is
 * expected to produce.
 */

function le32(n: number): Uint8Array {
  const buf = new Uint8Array(4);
  new DataView(buf.buffer).setInt32(0, n, true);
  return buf;
}

function le64(n: bigint): Uint8Array {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigUint64(0, n, true);
  return buf;
}

function le16(n: number): Uint8Array {
  const buf = new Uint8Array(2);
  new DataView(buf.buffer).setUint16(0, n, true);
  return buf;
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((sum, p) => sum + p.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

const REMAINING_ACCOUNTS_INFO_NONE = new Uint8Array([2, 0, 0, 0, 0, 0, 1, 0]);

export interface AddLiquidityFixture {
  lbPair: PublicKey;
  position: PublicKey;
  owner: PublicKey;
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
}

export function buildAddLiquidityInstruction(fixture: AddLiquidityFixture): TransactionInstruction {
  const userX = associatedTokenAddress(fixture.owner, fixture.tokenXMint, fixture.tokenXProgram);
  const userY = associatedTokenAddress(fixture.owner, fixture.tokenYMint, fixture.tokenYProgram);
  const reserveX = reserve(fixture.lbPair, fixture.tokenXMint);
  const reserveY = reserve(fixture.lbPair, fixture.tokenYMint);
  const bitmapExt = optionalBinArrayBitmapExtension(fixture.lbPair, fixture.minBinId, fixture.maxBinId);
  const binArrays = binArraysCoveringRange(fixture.lbPair, fixture.minBinId, fixture.maxBinId);

  const keys = [
    { pubkey: fixture.position, isSigner: false, isWritable: true },
    { pubkey: fixture.lbPair, isSigner: false, isWritable: true },
    { pubkey: bitmapExt, isSigner: false, isWritable: true },
    { pubkey: userX, isSigner: false, isWritable: true },
    { pubkey: userY, isSigner: false, isWritable: true },
    { pubkey: reserveX, isSigner: false, isWritable: true },
    { pubkey: reserveY, isSigner: false, isWritable: true },
    { pubkey: fixture.tokenXMint, isSigner: false, isWritable: false },
    { pubkey: fixture.tokenYMint, isSigner: false, isWritable: false },
    { pubkey: fixture.owner, isSigner: true, isWritable: false },
    { pubkey: fixture.tokenXProgram, isSigner: false, isWritable: false },
    { pubkey: fixture.tokenYProgram, isSigner: false, isWritable: false },
    { pubkey: eventAuthority(), isSigner: false, isWritable: false },
    { pubkey: DLMM_PROGRAM_ID, isSigner: false, isWritable: false },
    ...binArrays.map((b) => ({ pubkey: b, isSigner: false, isWritable: true })),
  ];

  const data = concatBytes(
    anchorDiscriminator("global", "add_liquidity_by_strategy2"),
    le64(fixture.amountX),
    le64(fixture.amountY),
    le32(fixture.activeId),
    le32(fixture.maxActiveBinSlippage),
    le32(fixture.minBinId),
    le32(fixture.maxBinId),
    new Uint8Array([3]), // StrategyType::SpotBalanced
    new Uint8Array(64), // StrategyParameters.parameters, all zero (favor_token_x = false)
    REMAINING_ACCOUNTS_INFO_NONE,
  );

  return new TransactionInstruction({ keys, programId: DLMM_PROGRAM_ID, data: Buffer.from(data) });
}

export interface RemoveLiquidityFixture {
  lbPair: PublicKey;
  position: PublicKey;
  owner: PublicKey;
  tokenXMint: PublicKey;
  tokenYMint: PublicKey;
  tokenXProgram: PublicKey;
  tokenYProgram: PublicKey;
  fromBinId: number;
  toBinId: number;
  bpsToRemove: number;
}

export function buildRemoveLiquidityInstruction(fixture: RemoveLiquidityFixture): TransactionInstruction {
  const userX = associatedTokenAddress(fixture.owner, fixture.tokenXMint, fixture.tokenXProgram);
  const userY = associatedTokenAddress(fixture.owner, fixture.tokenYMint, fixture.tokenYProgram);
  const reserveX = reserve(fixture.lbPair, fixture.tokenXMint);
  const reserveY = reserve(fixture.lbPair, fixture.tokenYMint);
  const bitmapExt = optionalBinArrayBitmapExtension(fixture.lbPair, fixture.fromBinId, fixture.toBinId);
  const binArrays = binArraysCoveringRange(fixture.lbPair, fixture.fromBinId, fixture.toBinId);

  const keys = [
    { pubkey: fixture.position, isSigner: false, isWritable: true },
    { pubkey: fixture.lbPair, isSigner: false, isWritable: true },
    { pubkey: bitmapExt, isSigner: false, isWritable: true },
    { pubkey: userX, isSigner: false, isWritable: true },
    { pubkey: userY, isSigner: false, isWritable: true },
    { pubkey: reserveX, isSigner: false, isWritable: true },
    { pubkey: reserveY, isSigner: false, isWritable: true },
    { pubkey: fixture.tokenXMint, isSigner: false, isWritable: false },
    { pubkey: fixture.tokenYMint, isSigner: false, isWritable: false },
    { pubkey: fixture.owner, isSigner: true, isWritable: false },
    { pubkey: fixture.tokenXProgram, isSigner: false, isWritable: false },
    { pubkey: fixture.tokenYProgram, isSigner: false, isWritable: false },
    { pubkey: MEMO_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: eventAuthority(), isSigner: false, isWritable: false },
    { pubkey: DLMM_PROGRAM_ID, isSigner: false, isWritable: false },
    ...binArrays.map((b) => ({ pubkey: b, isSigner: false, isWritable: true })),
  ];

  const data = concatBytes(
    anchorDiscriminator("global", "remove_liquidity_by_range2"),
    le32(fixture.fromBinId),
    le32(fixture.toBinId),
    le16(fixture.bpsToRemove),
    REMAINING_ACCOUNTS_INFO_NONE,
  );

  return new TransactionInstruction({ keys, programId: DLMM_PROGRAM_ID, data: Buffer.from(data) });
}

export interface OpenPositionFixture {
  payer: PublicKey;
  position: PublicKey;
  lbPair: PublicKey;
  owner: PublicKey;
  lowerBinId: number;
  width: number;
}

export function buildOpenPositionInstruction(fixture: OpenPositionFixture): TransactionInstruction {
  const keys = [
    { pubkey: fixture.payer, isSigner: true, isWritable: true },
    { pubkey: fixture.position, isSigner: true, isWritable: true },
    { pubkey: fixture.lbPair, isSigner: false, isWritable: false },
    { pubkey: fixture.owner, isSigner: true, isWritable: false },
    { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: eventAuthority(), isSigner: false, isWritable: false },
    { pubkey: DLMM_PROGRAM_ID, isSigner: false, isWritable: false },
  ];
  const data = concatBytes(
    anchorDiscriminator("global", "initialize_position2"),
    le32(fixture.lowerBinId),
    le32(fixture.width),
  );
  return new TransactionInstruction({ keys, programId: DLMM_PROGRAM_ID, data: Buffer.from(data) });
}

export interface ClosePositionFixture {
  position: PublicKey;
  owner: PublicKey;
  rentReceiver: PublicKey;
}

export function buildClosePositionInstruction(fixture: ClosePositionFixture): TransactionInstruction {
  const keys = [
    { pubkey: fixture.position, isSigner: false, isWritable: true },
    { pubkey: fixture.owner, isSigner: true, isWritable: false },
    { pubkey: fixture.rentReceiver, isSigner: false, isWritable: true },
    { pubkey: eventAuthority(), isSigner: false, isWritable: false },
    { pubkey: DLMM_PROGRAM_ID, isSigner: false, isWritable: false },
  ];
  const data = anchorDiscriminator("global", "close_position2");
  return new TransactionInstruction({ keys, programId: DLMM_PROGRAM_ID, data: Buffer.from(data) });
}
