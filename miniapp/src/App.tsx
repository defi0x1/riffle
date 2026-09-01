import { useMemo, useState } from "react";
import type { Keypair } from "@solana/web3.js";

import { ApiClient } from "./api/client";
import type { BuildTxResponse } from "./api/types";
import { CreateWallet } from "./components/CreateWallet";
import { Dashboard } from "./components/Dashboard";
import { ExportKey } from "./components/ExportKey";
import { ImportWallet } from "./components/ImportWallet";
import { TxReview } from "./components/TxReview";
import { WalletProvider, useWallet } from "./state/WalletContext";

type View = "dashboard" | "export";

interface PendingTx {
  response: BuildTxResponse;
  ephemeralPositionKeypair?: Keypair;
}

function AppInner(): JSX.Element {
  const { vault, loading, error, forgetWallet } = useWallet();
  const [onboardingMode, setOnboardingMode] = useState<"create" | "import" | null>(null);
  const [view, setView] = useState<View>("dashboard");
  const [pendingTx, setPendingTx] = useState<PendingTx | null>(null);
  const [lastSignature, setLastSignature] = useState<string | null>(null);

  const apiClient = useMemo(() => new ApiClient({ baseUrl: import.meta.env.VITE_API_BASE_URL }), []);

  if (loading) {
    return <div className="screen">Loading...</div>;
  }

  if (error) {
    return (
      <div className="screen">
        <h1>Storage unavailable</h1>
        <p className="error-text">{error}</p>
        <p>
          This app stores your encrypted wallet in this browser's local storage. If that storage
          is disabled (private browsing, a restrictive browser setting), the wallet cannot be
          read here.
        </p>
      </div>
    );
  }

  if (!vault) {
    if (onboardingMode === "create") return <CreateWallet />;
    if (onboardingMode === "import") return <ImportWallet />;
    return (
      <div className="screen">
        <h1>No wallet found on this device</h1>
        <p>
          Either this is the first time opening the app here, or this device's local storage was
          cleared. If a wallet was created before and the recovery phrase was backed up, import it
          below.
        </p>
        <button onClick={() => setOnboardingMode("create")}>Create a new wallet</button>
        <button onClick={() => setOnboardingMode("import")}>Import an existing wallet</button>
      </div>
    );
  }

  if (pendingTx) {
    return (
      <TxReview
        vault={vault}
        apiClient={apiClient}
        buildResponse={pendingTx.response}
        {...(pendingTx.ephemeralPositionKeypair
          ? { ephemeralPositionKeypair: pendingTx.ephemeralPositionKeypair }
          : {})}
        onSigned={(signature) => {
          setLastSignature(signature);
          setPendingTx(null);
        }}
        onCancel={() => setPendingTx(null)}
      />
    );
  }

  if (view === "export") {
    return (
      <div className="screen">
        <ExportKey />
        <button onClick={() => setView("dashboard")}>Back</button>
      </div>
    );
  }

  return (
    <div>
      {lastSignature && (
        <p className="verify-ok">Last transaction confirmed: {lastSignature}</p>
      )}
      <Dashboard
        publicKey={vault.publicKey}
        apiClient={apiClient}
        onBuildResult={(response, ephemeralPositionKeypair) =>
          setPendingTx(
            ephemeralPositionKeypair ? { response, ephemeralPositionKeypair } : { response },
          )
        }
        onExport={() => setView("export")}
        onForget={() => void forgetWallet()}
      />
    </div>
  );
}

export function App(): JSX.Element {
  return (
    <WalletProvider>
      <AppInner />
    </WalletProvider>
  );
}
