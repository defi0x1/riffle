import { useState } from "react";

import { createWallet } from "../crypto/wallet";
import type { CreatedWallet } from "../crypto/wallet";
import { estimatePassphraseStrength, MIN_ACCEPTABLE_SCORE } from "../crypto/passphraseStrength";
import { useWallet } from "../state/WalletContext";

type Step = "passphrase" | "reveal" | "confirm-backup";

/**
 * The recovery phrase is shown here exactly once, inside this screen, and never again without
 * going through the export flow (components/ExportKey.tsx) with its own passphrase prompt. It is
 * never sent anywhere -- not to the backend, not into a Telegram chat message (a phrase typed
 * into chat lives in Telegram's own message history indefinitely, which this screen never risks
 * since the phrase only ever exists inside this Mini App's own UI).
 *
 * The vault is only persisted (setVault, which writes to IndexedDB and flips the app's own
 * routing over to the dashboard) once the user has clicked through the backup warning -- not
 * the moment it is generated -- so this component, not global vault presence, controls when the
 * reveal screen is shown.
 */
export function CreateWallet(): JSX.Element {
  const { setVault } = useWallet();
  const [step, setStep] = useState<Step>("passphrase");
  const [passphrase, setPassphrase] = useState("");
  const [confirmPassphrase, setConfirmPassphrase] = useState("");
  const [created, setCreated] = useState<CreatedWallet | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const strength = estimatePassphraseStrength(passphrase);
  const passphrasesMatch = passphrase.length > 0 && passphrase === confirmPassphrase;
  const canProceed = strength.score >= MIN_ACCEPTABLE_SCORE && passphrasesMatch;

  async function handleGenerate(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const wallet = await createWallet(passphrase);
      setCreated(wallet);
      setStep("reveal");
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to create wallet");
    } finally {
      setBusy(false);
    }
  }

  async function handleConfirmed(): Promise<void> {
    if (!created) return;
    setBusy(true);
    try {
      await setVault(created.vault);
      setStep("confirm-backup");
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to save wallet to this device");
    } finally {
      setBusy(false);
    }
  }

  if (step === "reveal" && created) {
    return (
      <div className="screen">
        <h1>Back up your recovery phrase</h1>
        <p className="warning-text">
          Write these {created.mnemonic.split(" ").length} words down in order and keep them
          somewhere offline. Anyone who has them can take every asset in this wallet. There is no
          password reset and no support request that can recover a lost phrase -- if this is lost
          and the passphrase is also lost, the funds are gone permanently.
        </p>
        <div className="mnemonic-grid">
          {created.mnemonic.split(" ").map((word, i) => (
            <span key={i} className="mnemonic-word">
              {i + 1}. {word}
            </span>
          ))}
        </div>
        {error && <p className="error-text">{error}</p>}
        <button disabled={busy} onClick={handleConfirmed}>
          {busy ? "Saving..." : "I have written this down"}
        </button>
      </div>
    );
  }

  if (step === "confirm-backup") {
    return (
      <div className="screen">
        <h1>Wallet ready</h1>
        <p>Your wallet has been created and encrypted on this device.</p>
      </div>
    );
  }

  return (
    <div className="screen">
      <h1>Create a wallet</h1>
      <p>
        Choose a passphrase. This encrypts your key on this device -- it is never sent anywhere,
        never stored anywhere but in your own memory, and cannot be reset if forgotten.
      </p>
      <input
        type="password"
        placeholder="Passphrase"
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
          {strength.feedback.map((line, i) => (
            <p key={i} className="hint-text">
              {line}
            </p>
          ))}
        </div>
      )}
      {confirmPassphrase.length > 0 && !passphrasesMatch && (
        <p className="error-text">Passphrases do not match.</p>
      )}
      {error && <p className="error-text">{error}</p>}
      <button disabled={!canProceed || busy} onClick={handleGenerate}>
        {busy ? "Generating..." : "Generate wallet"}
      </button>
    </div>
  );
}
