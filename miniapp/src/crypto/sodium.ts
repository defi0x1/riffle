// The "sumo" build, not the default libsodium-wrappers: the base build deliberately excludes
// crypto_pwhash (Argon2) and several AEAD constructions to stay small, which is exactly the KDF
// and cipher this app needs (crypto/kdf.ts, crypto/cipher.ts). Sumo is the same audited codebase
// with the fuller API surface compiled in, not a different or less-reviewed library.
import sodium from "libsodium-wrappers-sumo";

// libsodium-wrappers loads its WASM module asynchronously; every call site awaits this once.
// The promise is cached so repeated calls after the first are free.
let readyPromise: Promise<typeof sodium> | null = null;

export function getSodium(): Promise<typeof sodium> {
  if (readyPromise === null) {
    readyPromise = sodium.ready.then(() => sodium);
  }
  return readyPromise;
}
