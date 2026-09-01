/**
 * Fixed-layout decoders for the five DLMM instructions this app ever builds a transaction
 * around, plus the account lists each one takes -- hand-rolled against a byte cursor rather than
 * a general borsh library. The layouts are small and fixed (no dynamic-length fields beyond one
 * constant, checked pattern -- see the RemainingAccountsInfo check below), so a ~60-line reader
 * is more auditable than pulling in a generic deserialiser for five struct shapes.
 */

export class ByteReader {
  private offset = 0;
  constructor(private readonly bytes: Uint8Array) {}

  get remaining(): number {
    return this.bytes.length - this.offset;
  }

  readU8(): number {
    this.assertRemaining(1);
    const value = this.bytes[this.offset] as number;
    this.offset += 1;
    return value;
  }

  readI32LE(): number {
    this.assertRemaining(4);
    const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 4);
    const value = view.getInt32(0, true);
    this.offset += 4;
    return value;
  }

  readU16LE(): number {
    this.assertRemaining(2);
    const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 2);
    const value = view.getUint16(0, true);
    this.offset += 2;
    return value;
  }

  readU64LE(): bigint {
    this.assertRemaining(8);
    const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 8);
    const value = view.getBigUint64(0, true);
    this.offset += 8;
    return value;
  }

  readBytes(n: number): Uint8Array {
    this.assertRemaining(n);
    const value = this.bytes.slice(this.offset, this.offset + n);
    this.offset += n;
    return value;
  }

  private assertRemaining(n: number): void {
    if (this.remaining < n) {
      throw new Error(`instruction data too short: need ${n} more bytes, have ${this.remaining}`);
    }
  }
}

/**
 * This app never builds transfer-hook remaining accounts (matching the backend instruction
 * builder's own documented scope, and the deliberate decision to exclude Token-2022
 * transfer-hook mints from the real-liquidity pool set) -- so the only RemainingAccountsInfo
 * this verifier ever accepts is the fixed "none" encoding: a 2-element slice vector with both
 * transfer-hook slots present but zero-length. Anything else, including a nonzero hook length,
 * is rejected rather than interpreted, since interpreting it correctly would mean trusting
 * accounts this verifier has no independent way to check.
 */
const REMAINING_ACCOUNTS_INFO_NONE = new Uint8Array([2, 0, 0, 0, 0, 0, 1, 0]);

export function expectRemainingAccountsInfoNone(reader: ByteReader): void {
  const tail = reader.readBytes(REMAINING_ACCOUNTS_INFO_NONE.length);
  for (let i = 0; i < tail.length; i++) {
    if (tail[i] !== REMAINING_ACCOUNTS_INFO_NONE[i]) {
      throw new Error("unexpected transfer-hook remaining-accounts encoding");
    }
  }
  if (reader.remaining !== 0) {
    throw new Error(`unexpected trailing bytes after instruction args: ${reader.remaining}`);
  }
}

export interface OpenPositionArgs {
  lowerBinId: number;
  width: number;
}

export function decodeOpenPositionArgs(data: Uint8Array): OpenPositionArgs {
  const reader = new ByteReader(data.slice(8));
  const lowerBinId = reader.readI32LE();
  const width = reader.readI32LE();
  if (reader.remaining !== 0) {
    throw new Error(`unexpected trailing bytes in initialize_position2 args: ${reader.remaining}`);
  }
  return { lowerBinId, width };
}

export interface AddLiquidityArgs {
  amountX: bigint;
  amountY: bigint;
  activeId: number;
  maxActiveBinSlippage: number;
  minBinId: number;
  maxBinId: number;
  strategyType: number;
  favorTokenX: boolean;
}

export function decodeAddLiquidityArgs(data: Uint8Array): AddLiquidityArgs {
  const reader = new ByteReader(data.slice(8));
  const amountX = reader.readU64LE();
  const amountY = reader.readU64LE();
  const activeId = reader.readI32LE();
  const maxActiveBinSlippage = reader.readI32LE();
  const minBinId = reader.readI32LE();
  const maxBinId = reader.readI32LE();
  const strategyType = reader.readU8();
  const parameters = reader.readBytes(64);
  expectRemainingAccountsInfoNone(reader);
  return {
    amountX,
    amountY,
    activeId,
    maxActiveBinSlippage,
    minBinId,
    maxBinId,
    strategyType,
    favorTokenX: parameters[0] === 1,
  };
}

export interface RemoveLiquidityArgs {
  fromBinId: number;
  toBinId: number;
  bpsToRemove: number;
}

export function decodeRemoveLiquidityArgs(data: Uint8Array): RemoveLiquidityArgs {
  const reader = new ByteReader(data.slice(8));
  const fromBinId = reader.readI32LE();
  const toBinId = reader.readI32LE();
  const bpsToRemove = reader.readU16LE();
  expectRemainingAccountsInfoNone(reader);
  return { fromBinId, toBinId, bpsToRemove };
}

export interface ClaimFeeArgs {
  minBinId: number;
  maxBinId: number;
}

export function decodeClaimFeeArgs(data: Uint8Array): ClaimFeeArgs {
  const reader = new ByteReader(data.slice(8));
  const minBinId = reader.readI32LE();
  const maxBinId = reader.readI32LE();
  expectRemainingAccountsInfoNone(reader);
  return { minBinId, maxBinId };
}

export function decodeClosePositionArgs(data: Uint8Array): Record<string, never> {
  if (data.length !== 8) {
    throw new Error(`close_position2 takes no arguments, got ${data.length - 8} extra bytes`);
  }
  return {};
}
