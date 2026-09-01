import { describe, expect, it } from "vitest";

import { deriveHardenedChild, deriveSolanaSeed, masterNodeFromSeed } from "../src/crypto/slip10";

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// SLIP-0010 "Test vector 1 for ed25519" (github.com/satoshilabs/slips/blob/master/slip-0010.md),
// independent of this project and of BIP-39 entirely -- validates the hand-rolled HMAC-SHA512
// derivation in crypto/slip10.ts against the published spec rather than only against itself.
describe("SLIP-0010 ed25519 derivation against the published spec test vector", () => {
  const seed = hexToBytes("000102030405060708090a0b0c0d0e0f");

  it("derives m/0' correctly", () => {
    const master = masterNodeFromSeed(seed);
    const child = deriveHardenedChild(master, 0);
    expect(bytesToHex(child.key)).toBe(
      "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3",
    );
    expect(bytesToHex(child.chainCode)).toBe(
      "8b59aa11380b624e81507a27fedda59fea6d0b779a778918a2fd3590e16e9c69",
    );
  });

  it("derives m/0'/1' correctly", () => {
    const master = masterNodeFromSeed(seed);
    const level1 = deriveHardenedChild(master, 0);
    const level2 = deriveHardenedChild(level1, 1);
    expect(bytesToHex(level2.key)).toBe(
      "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2",
    );
    expect(bytesToHex(level2.chainCode)).toBe(
      "a320425f77d1b5c2505a6b1b27382b37368ee640e3557c315416801243552f14",
    );
  });
});

describe("deriveSolanaSeed (m/44'/501'/0'/0')", () => {
  it("is deterministic for the same input seed", () => {
    const seed = new Uint8Array(64).fill(7);
    expect(bytesToHex(deriveSolanaSeed(seed))).toBe(bytesToHex(deriveSolanaSeed(seed)));
  });

  it("produces different output for different input seeds", () => {
    const seedA = new Uint8Array(64).fill(1);
    const seedB = new Uint8Array(64).fill(2);
    expect(bytesToHex(deriveSolanaSeed(seedA))).not.toBe(bytesToHex(deriveSolanaSeed(seedB)));
  });

  it("always returns a 32-byte seed", () => {
    const seed = new Uint8Array(64).fill(9);
    expect(deriveSolanaSeed(seed).length).toBe(32);
  });
});
