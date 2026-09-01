import { describe, expect, it } from "vitest";

import {
  entropyBytesToMnemonic,
  generateNewMnemonic,
  isValidMnemonic,
  mnemonicToEntropyBytes,
} from "../src/crypto/mnemonic";

describe("mnemonic generation and round trip", () => {
  it("generates a valid 24-word mnemonic by default", () => {
    const phrase = generateNewMnemonic();
    expect(phrase.trim().split(/\s+/)).toHaveLength(24);
    expect(isValidMnemonic(phrase)).toBe(true);
  });

  it("generates a valid 12-word mnemonic when asked", () => {
    const phrase = generateNewMnemonic(12);
    expect(phrase.trim().split(/\s+/)).toHaveLength(12);
    expect(isValidMnemonic(phrase)).toBe(true);
  });

  it("entropy <-> mnemonic is a lossless round trip", () => {
    const phrase = generateNewMnemonic(24);
    const entropy = mnemonicToEntropyBytes(phrase);
    expect(entropyBytesToMnemonic(entropy)).toBe(phrase);
  });

  it("rejects a mnemonic with a broken checksum", () => {
    // The standard all-zero-entropy BIP-39 test phrase, with its last word swapped for another
    // in-wordlist word that does not satisfy the checksum -- a fixed, deterministic case rather
    // than a randomly generated one, so this test cannot flake.
    const valid = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    expect(isValidMnemonic(valid)).toBe(true);
    const invalid = valid.replace(/ about$/, " zoo");
    expect(isValidMnemonic(invalid)).toBe(false);
  });

  it("rejects garbage input", () => {
    expect(isValidMnemonic("this is not a bip39 phrase")).toBe(false);
  });
});
