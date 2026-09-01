import { describe, expect, it } from "vitest";

import { decryptVault, encryptVault } from "../src/crypto/vault";
import type { WalletSecret } from "../src/crypto/types";

describe("vault encrypt/decrypt round trip", () => {
  it("round-trips a mnemonic-derived secret under the right passphrase", async () => {
    const secret: WalletSecret = {
      kind: "mnemonic",
      entropy: crypto.getRandomValues(new Uint8Array(32)),
      wordCount: 24,
    };
    const blob = await encryptVault(secret, "correct horse battery staple", "SomePubkeyBase58");
    const decrypted = await decryptVault(blob, "correct horse battery staple");

    expect(decrypted.kind).toBe("mnemonic");
    if (decrypted.kind === "mnemonic") {
      expect(Array.from(decrypted.entropy)).toEqual(Array.from(secret.entropy));
      expect(decrypted.wordCount).toBe(24);
    }
  });

  it("round-trips a raw-secret-key secret under the right passphrase", async () => {
    const secret: WalletSecret = {
      kind: "raw-secret-key",
      secretKey: crypto.getRandomValues(new Uint8Array(64)),
    };
    const blob = await encryptVault(secret, "another passphrase entirely", "SomePubkeyBase58");
    const decrypted = await decryptVault(blob, "another passphrase entirely");

    expect(decrypted.kind).toBe("raw-secret-key");
    if (decrypted.kind === "raw-secret-key") {
      expect(Array.from(decrypted.secretKey)).toEqual(Array.from(secret.secretKey));
    }
  });

  it("never stores the passphrase or plaintext in the persisted blob", async () => {
    const secret: WalletSecret = {
      kind: "mnemonic",
      entropy: crypto.getRandomValues(new Uint8Array(32)),
      wordCount: 24,
    };
    const passphrase = "a very specific passphrase marker 12345";
    const blob = await encryptVault(secret, passphrase, "SomePubkeyBase58");
    const serialized = JSON.stringify(blob);

    expect(serialized).not.toContain(passphrase);
    expect(serialized).not.toContain(Buffer.from(secret.entropy).toString("base64"));
    // Only the fields the design allows to be persisted should be present.
    expect(Object.keys(blob).sort()).toEqual(
      ["ciphertext", "createdAt", "kdf", "nonce", "publicKey", "salt", "version"].sort(),
    );
  });

  it("fails cleanly, without leaking any plaintext, when the passphrase is wrong", async () => {
    const secret: WalletSecret = {
      kind: "mnemonic",
      entropy: crypto.getRandomValues(new Uint8Array(32)),
      wordCount: 24,
    };
    const blob = await encryptVault(secret, "the-real-passphrase", "SomePubkeyBase58");

    await expect(decryptVault(blob, "a-completely-wrong-passphrase")).rejects.toThrow();
  });

  it("fails cleanly when the ciphertext has been tampered with", async () => {
    const secret: WalletSecret = {
      kind: "mnemonic",
      entropy: crypto.getRandomValues(new Uint8Array(32)),
      wordCount: 24,
    };
    const blob = await encryptVault(secret, "the-real-passphrase", "SomePubkeyBase58");
    const tampered = { ...blob, ciphertext: blob.ciphertext.slice(0, -4) + "AAAA" };

    await expect(decryptVault(tampered, "the-real-passphrase")).rejects.toThrow();
  });
});
