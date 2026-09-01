import { getSodium } from "./sodium";

/**
 * Argon2id, memory-hard so a stolen ciphertext resists GPU/ASIC parallel brute force better than
 * PBKDF2 or bcrypt. Parameters are an OWASP mobile-friendly starting point:
 *   - memlimit: 64 MiB
 *   - opslimit (iterations): 3
 *   - parallelism: fixed at 1 lane by libsodium's Argon2id implementation itself, which is the
 *     right choice for a phone-class device with no threading guarantee in a browser tab.
 *
 * These are meant to cost roughly a second on a mid-range Android phone inside Telegram's
 * in-app browser, which is the binding constraint -- not a desktop, where this would finish
 * far faster and could safely be tuned heavier. If field data ever shows this running
 * uncomfortably slow or fast on real devices, raise or lower `opslimit` first; `memlimit`
 * changes are more disruptive because every existing vault's stored params must keep decrypting
 * under whatever value was in force when it was written (see VaultKdfParams below).
 */
export const DEFAULT_KDF_PARAMS: Readonly<{ opslimit: number; memlimitBytes: number }> = {
  opslimit: 3,
  memlimitBytes: 64 * 1024 * 1024,
};

export const SALT_BYTES = 16; // crypto_pwhash_argon2id_SALTBYTES
export const DERIVED_KEY_BYTES = 32; // crypto_aead_xchacha20poly1305_ietf_KEYBYTES

export interface KdfParams {
  algorithm: "argon2id";
  opslimit: number;
  memlimitBytes: number;
}

export function defaultKdfParams(): KdfParams {
  return { algorithm: "argon2id", ...DEFAULT_KDF_PARAMS };
}

export async function randomSalt(): Promise<Uint8Array> {
  const sodium = await getSodium();
  return sodium.randombytes_buf(SALT_BYTES);
}

/**
 * Derives a 32-byte symmetric key from a passphrase and salt. The passphrase itself is never
 * returned, stored, or logged by this function or any caller in this codebase -- it exists only
 * as a parameter for the duration of this call.
 */
export async function deriveKey(
  passphrase: string,
  salt: Uint8Array,
  params: KdfParams,
): Promise<Uint8Array> {
  if (params.algorithm !== "argon2id") {
    throw new Error(`unsupported KDF algorithm: ${params.algorithm satisfies never}`);
  }
  if (salt.length !== SALT_BYTES) {
    throw new Error(`expected a ${SALT_BYTES}-byte salt, got ${salt.length}`);
  }
  const sodium = await getSodium();
  return sodium.crypto_pwhash(
    DERIVED_KEY_BYTES,
    passphrase,
    salt,
    params.opslimit,
    params.memlimitBytes,
    sodium.crypto_pwhash_ALG_ARGON2ID13,
  );
}
