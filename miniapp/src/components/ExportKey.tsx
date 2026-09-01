import { useEffect, useState } from "react";

import { revealMnemonic } from "../crypto/wallet";
import { useWallet } from "../state/WalletContext";
import { PassphraseModal } from "./PassphraseModal";

/** How long the revealed phrase stays on screen before it is cleared automatically -- a export
 * screen left open and unattended (a locked phone found by someone else, say) should not leave
 * the phrase visible indefinitely. The user can still re-run export as many times as needed. */
const AUTO_CLEAR_MS = 60_000;

/**
 * Every export path is also an exfiltration path -- the same screen that legitimately restores
 * access on a new device is exactly what a malicious or compromised copy of this app would want
 * to show. The only levers available are friction (the passphrase requirement below) and this
 * warning; there is no way to make export convenient for the legitimate case without it being
 * equally convenient for the attack case, so this does not pretend otherwise.
 */
export function ExportKey(): JSX.Element {
  const { vault } = useWallet();
  const [showPrompt, setShowPrompt] = useState(false);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [noMnemonic, setNoMnemonic] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!revealed) return;
    const timer = setTimeout(() => setRevealed(null), AUTO_CLEAR_MS);
    return () => clearTimeout(timer);
  }, [revealed]);

  async function handleSubmit(passphrase: string): Promise<void> {
    if (!vault) return;
    setBusy(true);
    setError(null);
    try {
      const mnemonic = await revealMnemonic(vault, passphrase);
      if (mnemonic === null) {
        setNoMnemonic(true);
      } else {
        setRevealed(mnemonic);
      }
      setShowPrompt(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "incorrect passphrase");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="screen">
      <h1>Export recovery phrase</h1>
      <p className="warning-text">
        Anyone who sees this phrase can take every asset in this wallet, permanently and without
        needing your passphrase again. Only reveal it somewhere private. Never paste it into a
        Telegram chat, a screenshot you might share, or any website -- there is no legitimate
        reason this app or anyone else would ever ask you to send it.
      </p>
      {revealed && (
        <>
          <div className="mnemonic-grid">
            {revealed.split(" ").map((word, i) => (
              <span key={i} className="mnemonic-word">
                {i + 1}. {word}
              </span>
            ))}
          </div>
          <button onClick={() => setRevealed(null)}>Clear from screen now</button>
        </>
      )}
      {noMnemonic && (
        <p>This wallet was imported from a raw secret key and has no recovery phrase to show.</p>
      )}
      {!revealed && !noMnemonic && (
        <button onClick={() => setShowPrompt(true)}>Reveal recovery phrase</button>
      )}
      {showPrompt && (
        <PassphraseModal
          title="Confirm your passphrase"
          description="Enter your passphrase to reveal the recovery phrase."
          confirmLabel="Reveal"
          submitting={busy}
          errorMessage={error}
          onSubmit={(p) => void handleSubmit(p)}
          onCancel={() => setShowPrompt(false)}
        />
      )}
    </div>
  );
}
