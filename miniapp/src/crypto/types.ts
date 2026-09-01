import type { KdfParams } from "./kdf";

/**
 * What actually gets encrypted. Two shapes because the design supports two import paths (see
 * crypto/wallet.ts): the promoted mnemonic path, and a secondary raw-secret-key path for a user
 * migrating a key that did not come from a BIP-39 phrase. Storing entropy rather than the
 * mnemonic sentence itself is just a size optimisation -- entropyToMnemonic is a deterministic,
 * lossless reconstruction, so the two are equivalent as plaintext.
 */
export type WalletSecret =
  | { kind: "mnemonic"; entropy: Uint8Array; wordCount: 12 | 24 }
  | { kind: "raw-secret-key"; secretKey: Uint8Array }; // 64-byte ed25519 secret key, Solana convention

/**
 * Persisted at rest. Only ciphertext, salt, nonce and KDF parameters ever get written to
 * storage -- never the passphrase, never plaintext. `version` lets a future change to the KDF
 * defaults or blob shape stay readable against vaults written under an older version without
 * guessing.
 */
export interface VaultBlob {
  version: 1;
  kdf: KdfParams;
  salt: string; // base64
  nonce: string; // base64
  ciphertext: string; // base64, includes the Poly1305 tag (combined AEAD mode)
  publicKey: string; // base58 -- public by definition, stored alongside for lookup without a decrypt
  createdAt: string; // ISO 8601, display only, not security-relevant
}
