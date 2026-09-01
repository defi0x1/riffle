import {
  generateMnemonic,
  entropyToMnemonic,
  mnemonicToEntropy,
  mnemonicToSeed,
  validateMnemonic,
} from "@scure/bip39";
import { wordlist } from "@scure/bip39/wordlists/english";

/**
 * 24 words (256 bits of entropy) is the default for a freshly created wallet -- more margin
 * than the 12-word/128-bit minimum, at the cost of a longer phrase to back up. 12-word import is
 * still accepted, since plenty of wallets a user might migrate from use it.
 */
export const DEFAULT_WORD_COUNT = 24;

export function generateNewMnemonic(wordCount: 12 | 24 = DEFAULT_WORD_COUNT): string {
  const entropyBits = wordCount === 24 ? 256 : 128;
  return generateMnemonic(wordlist, entropyBits);
}

export function isValidMnemonic(phrase: string): boolean {
  return validateMnemonic(phrase.trim().toLowerCase(), wordlist);
}

/** Entropy is what gets encrypted at rest -- more compact than the sentence, and the sentence
 * is trivially recoverable from it (entropyToMnemonic is deterministic), so storing both would
 * just be redundant plaintext-shaped surface. */
export function mnemonicToEntropyBytes(phrase: string): Uint8Array {
  return mnemonicToEntropy(phrase.trim().toLowerCase(), wordlist);
}

export function entropyBytesToMnemonic(entropy: Uint8Array): string {
  return entropyToMnemonic(entropy, wordlist);
}

/** BIP-39 seed derivation (PBKDF2-HMAC-SHA512, 2048 rounds over the mnemonic) -- this is not the
 * passphrase-based vault KDF (see crypto/kdf.ts); it is a fixed, unsalted-by-secret step BIP-39
 * itself defines to turn a mnemonic into key-derivation input. The optional BIP-39 "passphrase"
 * parameter (a 25th word) is not used here -- this app's own vault passphrase already serves an
 * equivalent purpose at the storage layer, and stacking both would mean two different secrets a
 * user could forget independently for no real benefit. */
export async function mnemonicToSeedBytes(phrase: string): Promise<Uint8Array> {
  return mnemonicToSeed(phrase.trim().toLowerCase());
}
