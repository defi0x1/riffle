import { useEffect, useState } from "react";
import { Keypair } from "@solana/web3.js";

import { ApiClient } from "../api/client";
import type { BalancesResponse, BuildTxResponse, PositionSummary, PositionsResponse } from "../api/types";

interface DashboardProps {
  publicKey: string;
  apiClient: ApiClient;
  onBuildResult: (response: BuildTxResponse, ephemeralPositionKeypair?: Keypair) => void;
  onExport: () => void;
  onForget: () => void;
}

export function Dashboard({
  publicKey,
  apiClient,
  onBuildResult,
  onExport,
  onForget,
}: DashboardProps): JSX.Element {
  const [balances, setBalances] = useState<BalancesResponse | null>(null);
  const [positions, setPositions] = useState<PositionsResponse | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([apiClient.getBalances(), apiClient.getPositions()])
      .then(([b, p]) => {
        if (cancelled) return;
        setBalances(b);
        setPositions(p);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setLoadError(err instanceof Error ? err.message : "failed to load account data");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [apiClient]);

  return (
    <div className="screen">
      <h1>Wallet</h1>
      <p className="pubkey-text">{publicKey}</p>

      {loadError && (
        <p className="warning-text">
          Could not reach the backend ({loadError}). Balances and positions are unavailable, but
          your key is safe on this device either way.
        </p>
      )}

      {balances && (
        <section>
          <h2>Balances</h2>
          <p>{balances.solLamports} lamports SOL</p>
          <ul>
            {balances.tokens.map((t) => (
              <li key={t.mint}>
                {t.amountRaw} raw units of {t.mint}
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <h2>Positions</h2>
        {positions && positions.positions.length === 0 && <p>No open positions.</p>}
        {positions?.positions.map((p) => (
          <PositionRow key={p.positionAddress} position={p} apiClient={apiClient} onBuildResult={onBuildResult} />
        ))}
      </section>

      <OpenPositionForm apiClient={apiClient} onBuildResult={onBuildResult} />

      <div className="modal-actions">
        <button onClick={onExport}>Export recovery phrase</button>
        <button onClick={onForget}>Remove wallet from this device</button>
      </div>
    </div>
  );
}

function OpenPositionForm({
  apiClient,
  onBuildResult,
}: {
  apiClient: ApiClient;
  onBuildResult: (response: BuildTxResponse, ephemeralPositionKeypair?: Keypair) => void;
}): JSX.Element {
  const [poolAddress, setPoolAddress] = useState("");
  const [lowerBinId, setLowerBinId] = useState("");
  const [width, setWidth] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      // Generated on this device -- its public half goes to the backend so it can be named in
      // the built instruction; its private half never leaves this component's memory and is only
      // ever used, later, to co-sign alongside the wallet's own key in TxReview.
      const ephemeralPositionKeypair = Keypair.generate();
      const response = await apiClient.buildOpenPosition({
        poolAddress,
        lowerBinId: Number(lowerBinId),
        width: Number(width),
        ephemeralPositionPubkey: ephemeralPositionKeypair.publicKey.toBase58(),
        idempotencyKey: crypto.randomUUID(),
      });
      onBuildResult(response, ephemeralPositionKeypair);
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to build transaction");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h2>Open a new position</h2>
      <input placeholder="Pool address" value={poolAddress} onChange={(e) => setPoolAddress(e.target.value)} />
      <input placeholder="Lower bin id" value={lowerBinId} onChange={(e) => setLowerBinId(e.target.value)} />
      <input placeholder="Width (bins)" value={width} onChange={(e) => setWidth(e.target.value)} />
      {error && <p className="error-text">{error}</p>}
      <button disabled={busy || !poolAddress || !lowerBinId || !width} onClick={() => void handleSubmit()}>
        {busy ? "Building..." : "Review transaction"}
      </button>
    </section>
  );
}

function PositionRow({
  position,
  apiClient,
  onBuildResult,
}: {
  position: PositionSummary;
  apiClient: ApiClient;
  onBuildResult: (response: BuildTxResponse) => void;
}): JSX.Element {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [amountX, setAmountX] = useState("");
  const [amountY, setAmountY] = useState("");
  const [bpsToRemove, setBpsToRemove] = useState("10000");

  async function run(label: string, fn: () => Promise<BuildTxResponse>): Promise<void> {
    setBusy(label);
    setError(null);
    try {
      const response = await fn();
      onBuildResult(response);
    } catch (err) {
      setError(err instanceof Error ? err.message : `failed to build ${label}`);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="position-row">
      <p>
        {position.poolAddress} ({position.status}), bins {position.lowerBinId}..{position.upperBinId}
      </p>
      <p>
        Pending fees: {position.feesXPending} / {position.feesYPending}
      </p>
      <div className="inline-form">
        <input placeholder="Amount X" value={amountX} onChange={(e) => setAmountX(e.target.value)} />
        <input placeholder="Amount Y" value={amountY} onChange={(e) => setAmountY(e.target.value)} />
        <button
          disabled={busy !== null}
          onClick={() =>
            void run("add-liquidity", () =>
              apiClient.buildAddLiquidity({
                poolAddress: position.poolAddress,
                positionAddress: position.positionAddress,
                amountXRaw: amountX,
                amountYRaw: amountY,
                maxActiveBinSlippageBps: 100,
                minBinId: position.lowerBinId,
                maxBinId: position.upperBinId,
                strategy: "spot-balanced",
                idempotencyKey: crypto.randomUUID(),
              }),
            )
          }
        >
          Add liquidity
        </button>
      </div>
      <div className="inline-form">
        <input placeholder="Bps to remove" value={bpsToRemove} onChange={(e) => setBpsToRemove(e.target.value)} />
        <button
          disabled={busy !== null}
          onClick={() =>
            void run("remove-liquidity", () =>
              apiClient.buildRemoveLiquidity({
                poolAddress: position.poolAddress,
                positionAddress: position.positionAddress,
                fromBinId: position.lowerBinId,
                toBinId: position.upperBinId,
                bpsToRemove: Number(bpsToRemove),
                idempotencyKey: crypto.randomUUID(),
              }),
            )
          }
        >
          Remove liquidity
        </button>
      </div>
      <button
        disabled={busy !== null}
        onClick={() =>
          void run("claim-fees", () =>
            apiClient.buildClaimFees({
              poolAddress: position.poolAddress,
              positionAddress: position.positionAddress,
              minBinId: position.lowerBinId,
              maxBinId: position.upperBinId,
              idempotencyKey: crypto.randomUUID(),
            }),
          )
        }
      >
        Claim fees
      </button>
      <button
        disabled={busy !== null}
        onClick={() =>
          void run("close-position", () =>
            apiClient.buildClosePosition({
              positionAddress: position.positionAddress,
              idempotencyKey: crypto.randomUUID(),
            }),
          )
        }
      >
        Close position
      </button>
      {busy && <p>Building {busy}...</p>}
      {error && <p className="error-text">{error}</p>}
    </div>
  );
}
