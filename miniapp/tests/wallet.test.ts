import { Keypair } from "@solana/web3.js";
import bs58 from "bs58";
import { describe, expect, it } from "vitest";

import {
  createWallet,
  importFromMnemonic,
  importFromSecretKey,
  revealMnemonic,
  unlockKeypair,
} from "../src/crypto/wallet";

describe("wallet create/import/unlock", () => {
  it("creating a wallet, then unlocking it, yields a keypair matching the reported public key", async () => {
    const created = await createWallet("a reasonably strong passphrase 42");
    const keypair = await unlockKeypair(created.vault, "a reasonably strong passphrase 42");
    expect(keypair.publicKey.toBase58()).toBe(created.publicKey);
  });

  it("importing the same mnemonic twice derives the same public key both times", async () => {
    const created = await createWallet("first passphrase here");
    const imported = await importFromMnemonic(created.mnemonic, "a different passphrase entirely");
    expect(imported.publicKey).toBe(created.publicKey);
  });

  it("rejects an invalid mnemonic before ever touching the passphrase", async () => {
    await expect(
      importFromMnemonic("not a real recovery phrase at all", "some passphrase"),
    ).rejects.toThrow();
  });

  it("unlockKeypair fails with a wrong passphrase and never returns a keypair", async () => {
    const created = await createWallet("the-actual-passphrase-99");
    await expect(unlockKeypair(created.vault, "wrong-passphrase-00")).rejects.toThrow();
  });

  it("revealMnemonic returns the same phrase that was shown at creation time", async () => {
    const created = await createWallet("reveal-test-passphrase-7");
    const revealed = await revealMnemonic(created.vault, "reveal-test-passphrase-7");
    expect(revealed).toBe(created.mnemonic);
  });

  it("revealMnemonic returns null for a raw-secret-key wallet, which has no phrase", async () => {
    const rawSecretKey = bs58.encode(Keypair.generate().secretKey);
    const imported = await importFromSecretKey(rawSecretKey, "raw-key-passphrase");
    const revealed = await revealMnemonic(imported.vault, "raw-key-passphrase");
    expect(revealed).toBeNull();
  });

  it("rejects an obviously malformed raw secret key", async () => {
    await expect(importFromSecretKey("not-base58!!!", "some passphrase")).rejects.toThrow();
    await expect(importFromSecretKey("11111111", "some passphrase")).rejects.toThrow();
  });
});
