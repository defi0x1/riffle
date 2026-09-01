import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import type { VaultBlob } from "../crypto/types";
import { clearVault, loadVault, saveVault } from "../storage/idb";

interface WalletContextValue {
  vault: VaultBlob | null;
  loading: boolean;
  error: string | null;
  setVault: (vault: VaultBlob) => Promise<void>;
  forgetWallet: () => Promise<void>;
}

const WalletContext = createContext<WalletContextValue | null>(null);

export function WalletProvider({ children }: { children: ReactNode }): JSX.Element {
  const [vault, setVaultState] = useState<VaultBlob | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadVault()
      .then((v) => {
        if (!cancelled) setVaultState(v);
      })
      .catch((err: unknown) => {
        // Storage being unavailable (e.g. a private-browsing context that disables IndexedDB) is
        // not the same failure as "no wallet was ever created" -- the empty-state UI needs to
        // say the two apart rather than silently showing an empty-wallet screen either way.
        if (!cancelled) setError(err instanceof Error ? err.message : "failed to read local storage");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setVault = useCallback(async (v: VaultBlob) => {
    await saveVault(v);
    setVaultState(v);
  }, []);

  const forgetWallet = useCallback(async () => {
    await clearVault();
    setVaultState(null);
  }, []);

  const value = useMemo<WalletContextValue>(
    () => ({ vault, loading, error, setVault, forgetWallet }),
    [vault, loading, error, setVault, forgetWallet],
  );

  return <WalletContext.Provider value={value}>{children}</WalletContext.Provider>;
}

export function useWallet(): WalletContextValue {
  const ctx = useContext(WalletContext);
  if (!ctx) throw new Error("useWallet must be used inside WalletProvider");
  return ctx;
}
