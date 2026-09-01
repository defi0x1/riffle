import { useState } from "react";

import { estimatePassphraseStrength, MIN_ACCEPTABLE_SCORE } from "../crypto/passphraseStrength";
import { importFromMnemonic, importFromSecretKey } from "../crypto/wallet";
import { useWallet } from "../state/WalletContext";

type Mode = "mnemonic" | "secret-key";

/**
 * Mnemonic import is the promoted path: a BIP-39 phrase is self-checksummed (a typo is caught
 * before anything is encrypted) and portable to any other SLIP-0010-derived wallet. Raw
 * secret-key import is offered as a secondary option, unpromoted in the UI copy below, for a key
 * that never had a phrase -- a pasted base58 secret key has no checksum, so a typo there fails
 * as a generic invalid-key error rather than a specific, correctable one.
 */
export function ImportWallet(): JSX.Element {
  const { setVault } = useWallet();
  const [mode, setMode] = useState<Mode>("mnemonic");
  const [secret, setSecret] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [confirmPassphrase, setConfirmPassphrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const strength = estimatePassphraseStrength(passphrase);
  const passphrasesMatch = passphrase.length > 0 && passphrase === confirmPassphrase;
  const canProceed =
    strength.score >= MIN_ACCEPTABLE_SCORE && passphrasesMatch && secret.trim().length > 0;

  async function handleImport(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const result =
        mode === "mnemonic"
          ? await importFromMnemonic(secret, passphrase)
          : await importFromSecretKey(secret, passphrase);
      await setVault(result.vault);
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to import wallet");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="screen">
      <h1>Import a wallet</h1>
      <div className="tab-row">
        <button
          className={mode === "mnemonic" ? "tab active" : "tab"}
          onClick={() => setMode("mnemonic")}
        >
          Recovery phrase
        </button>
        <button
          className={mode === "secret-key" ? "tab active" : "tab"}
          onClick={() => setMode("secret-key")}
        >
          Raw secret key (advanced)
        </button>
      </div>
      {mode === "secret-key" && (
        <p className="warning-text">
          Only use this if you have a base58-encoded secret key with no recovery phrase. A pasted
          key has no built-in checksum, so a typo here fails as a generic error rather than a
          specific one.
        </p>
      )}
      <textarea
        placeholder={mode === "mnemonic" ? "12 or 24 word recovery phrase" : "Base58 secret key"}
        value={secret}
        onChange={(e) => setSecret(e.target.value)}
        rows={mode === "mnemonic" ? 3 : 2}
      />
      <input
        type="password"
        placeholder="New passphrase for this device"
        value={passphrase}
        onChange={(e) => setPassphrase(e.target.value)}
      />
      <input
        type="password"
        placeholder="Confirm passphrase"
        value={confirmPassphrase}
        onChange={(e) => setConfirmPassphrase(e.target.value)}
      />
      {passphrase.length > 0 && (
        <div className="strength-meter" data-score={strength.score}>
          Strength: {["very weak", "weak", "okay", "good", "strong"][strength.score]}
        </div>
      )}
      {error && <p className="error-text">{error}</p>}
      <button disabled={!canProceed || busy} onClick={handleImport}>
        {busy ? "Importing..." : "Import wallet"}
      </button>
    </div>
  );
}
