import { useState } from "react";
import type { FormEvent } from "react";

interface PassphraseModalProps {
  title: string;
  description: string;
  confirmLabel: string;
  submitting: boolean;
  errorMessage: string | null;
  onSubmit: (passphrase: string) => void;
  onCancel: () => void;
}

/**
 * The only place in this app a passphrase is typed. There is no "remember this" option and no
 * session cache -- every signing action mounts one of these fresh, the passphrase is read into a
 * local variable, handed to onSubmit, and the input is cleared. This is what makes "the app
 * cannot act without the user present" true in practice: nothing upstream of this component ever
 * holds a decrypted key on the user's behalf between actions.
 */
export function PassphraseModal({
  title,
  description,
  confirmLabel,
  submitting,
  errorMessage,
  onSubmit,
  onCancel,
}: PassphraseModalProps): JSX.Element {
  const [passphrase, setPassphrase] = useState("");

  function handleSubmit(e: FormEvent): void {
    e.preventDefault();
    onSubmit(passphrase);
    setPassphrase("");
  }

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <form className="modal" onSubmit={handleSubmit}>
        <h2>{title}</h2>
        <p>{description}</p>
        <input
          type="password"
          autoFocus
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          placeholder="Passphrase"
          disabled={submitting}
        />
        {errorMessage && <p className="error-text">{errorMessage}</p>}
        <div className="modal-actions">
          <button type="button" onClick={onCancel} disabled={submitting}>
            Cancel
          </button>
          <button type="submit" disabled={submitting || passphrase.length === 0}>
            {submitting ? "Working..." : confirmLabel}
          </button>
        </div>
      </form>
    </div>
  );
}
