import { Keypair } from "@solana/web3.js";
import bs58 from "bs58";

import {
  DEFAULT_WORD_COUNT,
  entropyBytesToMnemonic,
  generateNewMnemonic,
  isValidMnemonic,
  mnemonicToEntropyBytes,
  mnemonicToSeedBytes,
} from "./mnemonic";
import { deriveSolanaSeed } from "./slip10";
import { wipe } from "./memory";
import { encryptVault, decryptVault } from "./vault";
import type { VaultBlob, WalletSecret } from "./types";

export class InvalidSecretKeyError extends Error {
  constructor() {
    super("that does not look like a valid Solana secret key");
    this.name = "InvalidSecretKeyError";
  }
}

/**
 * Expands a WalletSecret into a signing Keypair. Every intermediate seed buffer is wiped before
 * returning -- only the final Keypair (which itself holds the expanded 64-byte secret key
 * internally, outside this module's control) survives the call. Callers must drop their
 * reference to the returned Keypair as soon as the signature it was needed for is produced; see
 * crypto/memory.ts for what "wipe" can and cannot promise in this runtime.
 */
export async function keypairFromSecret(secret: WalletSecret): Promise<Keypair> {
  if (secret.kind === "raw-secret-key") {
    return Keypair.fromSecretKey(secret.secretKey);
  }

  const mnemonic = entropyBytesToMnemonic(secret.entropy);
  const bip39Seed = await mnemonicToSeedBytes(mnemonic);
  try {
    const solanaSeed = deriveSolanaSeed(bip39Seed);
    try {
      return Keypair.fromSeed(solanaSeed);
    } finally {
      wipe(solanaSeed);
    }
  } finally {
    wipe(bip39Seed);
    // `mnemonic` is a JS string holding the recovery phrase; strings cannot be wiped (see
    // crypto/vault.ts's decodeSecret comment for the same limitation) -- it becomes unreachable
    // once this function returns and is left for garbage collection, best-effort only.
  }
}

export interface CreatedWallet {
  /** Shown to the user exactly once at creation time so they can back it up; never persisted. */
  mnemonic: string;
  publicKey: string;
  vault: VaultBlob;
}

export async function createWallet(
  passphrase: string,
  wordCount: 12 | 24 = DEFAULT_WORD_COUNT,
): Promise<CreatedWallet> {
  const mnemonic = generateNewMnemonic(wordCount);
  const entropy = mnemonicToEntropyBytes(mnemonic);
  const secret: WalletSecret = { kind: "mnemonic", entropy, wordCount };
  const keypair = await keypairFromSecret(secret);
  const publicKey = keypair.publicKey.toBase58();
  const vault = await encryptVault(secret, passphrase, publicKey);
  wipe(entropy);
  return { mnemonic, publicKey, vault };
}

export async function importFromMnemonic(
  phrase: string,
  passphrase: string,
): Promise<{ publicKey: string; vault: VaultBlob }> {
  const normalized = phrase.trim().toLowerCase();
  if (!isValidMnemonic(normalized)) {
    throw new Error("that recovery phrase is not valid -- check the word order and spelling");
  }
  const wordCount = normalized.split(/\s+/).length === 24 ? 24 : 12;
  const entropy = mnemonicToEntropyBytes(normalized);
  const secret: WalletSecret = { kind: "mnemonic", entropy, wordCount };
  const keypair = await keypairFromSecret(secret);
  const publicKey = keypair.publicKey.toBase58();
  const vault = await encryptVault(secret, passphrase, publicKey);
  wipe(entropy);
  return { publicKey, vault };
}

/**
 * Secondary, unpromoted import path (see the README's dependency/design notes): a pasted base58
 * secret key has no built-in checksum the way a BIP-39 phrase does, so a typo fails quietly as
 * "wrong key" rather than "invalid phrase" -- callers should steer users toward the mnemonic
 * path and only offer this for someone migrating a key that never had a phrase.
 */
export async function importFromSecretKey(
  base58SecretKey: string,
  passphrase: string,
): Promise<{ publicKey: string; vault: VaultBlob }> {
  let decoded: Uint8Array;
  try {
    decoded = bs58.decode(base58SecretKey.trim());
  } catch {
    throw new InvalidSecretKeyError();
  }
  if (decoded.length !== 64) {
    throw new InvalidSecretKeyError();
  }
  let keypair: Keypair;
  try {
    keypair = Keypair.fromSecretKey(decoded);
  } catch {
    throw new InvalidSecretKeyError();
  }
  const secret: WalletSecret = { kind: "raw-secret-key", secretKey: decoded };
  const publicKey = keypair.publicKey.toBase58();
  const vault = await encryptVault(secret, passphrase, publicKey);
  wipe(decoded);
  return { publicKey, vault };
}

/**
 * Decrypts the vault and expands it to a signing Keypair in one step -- the only path anything
 * in this app should use to get a Keypair capable of producing a real signature. There is no
 * "unlocked session": call this immediately before signing, use the result, and let it go.
 */
export async function unlockKeypair(vault: VaultBlob, passphrase: string): Promise<Keypair> {
  const secret = await decryptVault(vault, passphrase);
  try {
    return await keypairFromSecret(secret);
  } finally {
    if (secret.kind === "mnemonic") {
      wipe(secret.entropy);
    } else {
      wipe(secret.secretKey);
    }
  }
}

/** For the export/reveal-phrase screen only -- see components/ExportKey for the warning copy
 * this must always be paired with. Returns null for a raw-secret-key wallet, which has no
 * mnemonic to show. */
export async function revealMnemonic(vault: VaultBlob, passphrase: string): Promise<string | null> {
  const secret = await decryptVault(vault, passphrase);
  if (secret.kind !== "mnemonic") {
    wipe(secret.secretKey);
    return null;
  }
  const mnemonic = entropyBytesToMnemonic(secret.entropy);
  wipe(secret.entropy);
  return mnemonic;
}
