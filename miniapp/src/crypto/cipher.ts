import { getSodium } from "./sodium";

/**
 * XChaCha20-Poly1305 (IETF variant). Chosen over AES-256-GCM for its 24-byte extended nonce:
 * with a 12-byte GCM nonce, encrypting under the same key twice with an accidentally-reused
 * nonce (e.g. a careless passphrase-change implementation) breaks confidentiality outright.
 * XChaCha20's 192-bit nonce makes a random collision practically impossible even across many
 * re-encryptions, so "always generate a fresh random nonce" is sufficient on its own, with no
 * separate nonce-counter bookkeeping to get wrong. Implemented by libsodium's audited WASM
 * build, not hand-rolled.
 */
export const NONCE_BYTES = 24; // crypto_aead_xchacha20poly1305_ietf_NPUBBYTES
export const KEY_BYTES = 32; // crypto_aead_xchacha20poly1305_ietf_KEYBYTES

/**
 * Binds ciphertext to this application and format version as associated data, so a vault blob
 * decrypted correctly under the right key still fails if it were ever placed in a different
 * context than the one it was encrypted for. Not secret, does not need to be stored -- it is
 * fixed and reconstructed identically on both encrypt and decrypt.
 */
const ASSOCIATED_DATA = new TextEncoder().encode("riffle-miniapp-vault-v1");

export async function randomNonce(): Promise<Uint8Array> {
  const sodium = await getSodium();
  return sodium.randombytes_buf(NONCE_BYTES);
}

export async function encrypt(
  plaintext: Uint8Array,
  key: Uint8Array,
  nonce: Uint8Array,
): Promise<Uint8Array> {
  if (key.length !== KEY_BYTES) {
    throw new Error(`expected a ${KEY_BYTES}-byte key, got ${key.length}`);
  }
  if (nonce.length !== NONCE_BYTES) {
    throw new Error(`expected a ${NONCE_BYTES}-byte nonce, got ${nonce.length}`);
  }
  const sodium = await getSodium();
  return sodium.crypto_aead_xchacha20poly1305_ietf_encrypt(
    plaintext,
    ASSOCIATED_DATA,
    null,
    nonce,
    key,
  );
}

/**
 * Throws if the ciphertext was tampered with, or the key/nonce/associated data do not match --
 * libsodium's AEAD decrypt returns null on any authentication failure, which this wraps into an
 * error so callers cannot accidentally treat a failed decrypt as an empty successful one.
 */
export async function decrypt(
  ciphertext: Uint8Array,
  key: Uint8Array,
  nonce: Uint8Array,
): Promise<Uint8Array> {
  if (key.length !== KEY_BYTES) {
    throw new Error(`expected a ${KEY_BYTES}-byte key, got ${key.length}`);
  }
  if (nonce.length !== NONCE_BYTES) {
    throw new Error(`expected a ${NONCE_BYTES}-byte nonce, got ${nonce.length}`);
  }
  const sodium = await getSodium();
  try {
    return sodium.crypto_aead_xchacha20poly1305_ietf_decrypt(
      null,
      ciphertext,
      ASSOCIATED_DATA,
      nonce,
      key,
    );
  } catch {
    // libsodium-wrappers throws on an authentication failure rather than returning null in some
    // builds; normalise both shapes to the same error so callers have one failure mode to handle
    // (wrong passphrase, corrupted storage, or a tampered blob all look identical from here,
    // which is correct -- distinguishing them would leak information to an attacker probing the
    // decrypt).
    throw new Error("decryption failed: wrong passphrase or corrupted vault");
  }
}
