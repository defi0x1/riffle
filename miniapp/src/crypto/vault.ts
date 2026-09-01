import { base64ToBytes, bytesToBase64 } from "./base64";
import { decrypt, encrypt, randomNonce } from "./cipher";
import { defaultKdfParams, deriveKey, randomSalt } from "./kdf";
import { wipe } from "./memory";
import type { VaultBlob, WalletSecret } from "./types";

/**
 * Plaintext wire shape encrypted inside the vault. Plain JSON over a hand-rolled binary format:
 * secrecy comes entirely from the AEAD, not from the encoding, and JSON keeps this auditable at
 * a glance rather than needing a parser to inspect.
 */
type SecretPayload =
  | { kind: "mnemonic"; entropy: string; wordCount: 12 | 24 } // entropy: base64
  | { kind: "raw-secret-key"; secretKey: string }; // secretKey: base64

function encodeSecret(secret: WalletSecret): Uint8Array {
  const payload: SecretPayload =
    secret.kind === "mnemonic"
      ? { kind: "mnemonic", entropy: bytesToBase64(secret.entropy), wordCount: secret.wordCount }
      : { kind: "raw-secret-key", secretKey: bytesToBase64(secret.secretKey) };
  return new TextEncoder().encode(JSON.stringify(payload));
}

// Passes through a JS string (TextDecoder's output, then JSON.parse's internal string values)
// on the way to the returned Uint8Arrays. Strings are immutable in JS, so unlike the Uint8Array
// buffers elsewhere in this module, that intermediate copy of the secret's base64 text cannot be
// wiped -- it is only reachable for as long as something references it, same caveat as
// crypto/memory.ts documents for engine-internal copies in general.
function decodeSecret(bytes: Uint8Array): WalletSecret {
  const payload = JSON.parse(new TextDecoder().decode(bytes)) as SecretPayload;
  if (payload.kind === "mnemonic") {
    return { kind: "mnemonic", entropy: base64ToBytes(payload.entropy), wordCount: payload.wordCount };
  }
  if (payload.kind === "raw-secret-key") {
    return { kind: "raw-secret-key", secretKey: base64ToBytes(payload.secretKey) };
  }
  throw new Error(`unknown vault payload kind: ${(payload as { kind: string }).kind}`);
}

/**
 * Encrypts `secret` under `passphrase`, producing the blob that gets persisted. `publicKey` is
 * stored alongside in the clear -- it is meant to be public, and storing it lets the app show
 * "wallet: <pubkey>" without a decrypt. The passphrase is read once here and never stored.
 */
export async function encryptVault(
  secret: WalletSecret,
  passphrase: string,
  publicKey: string,
): Promise<VaultBlob> {
  const kdf = defaultKdfParams();
  const salt = await randomSalt();
  const nonce = await randomNonce();
  const key = await deriveKey(passphrase, salt, kdf);
  const plaintext = encodeSecret(secret);
  try {
    const ciphertext = await encrypt(plaintext, key, nonce);
    return {
      version: 1,
      kdf,
      salt: bytesToBase64(salt),
      nonce: bytesToBase64(nonce),
      ciphertext: bytesToBase64(ciphertext),
      publicKey,
      createdAt: new Date().toISOString(),
    };
  } finally {
    wipe(plaintext);
    wipe(key);
  }
}

/**
 * Decrypts `blob` under `passphrase`. Throws on a wrong passphrase or a corrupted/tampered blob
 * -- AEAD authentication failure and JSON-parse failure both surface as the same generic error
 * from cipher.ts's decrypt(), which is intentional (see the comment there).
 *
 * The returned WalletSecret's byte arrays are live plaintext key material. The caller owns
 * wiping them (crypto/memory.ts's `wipe`/`withSecretBytes`) as soon as the signing operation
 * that needed them is done -- this function cannot do that on the caller's behalf, since the
 * whole point of returning them is that the caller is about to use them.
 */
export async function decryptVault(blob: VaultBlob, passphrase: string): Promise<WalletSecret> {
  const salt = base64ToBytes(blob.salt);
  const nonce = base64ToBytes(blob.nonce);
  const ciphertext = base64ToBytes(blob.ciphertext);
  const key = await deriveKey(passphrase, salt, blob.kdf);
  let plaintext: Uint8Array | null = null;
  try {
    plaintext = await decrypt(ciphertext, key, nonce);
    return decodeSecret(plaintext);
  } finally {
    wipe(key);
    if (plaintext !== null) {
      wipe(plaintext);
    }
  }
}
