import { useMemo, useState } from "react";
import { Keypair, PublicKey, VersionedTransaction } from "@solana/web3.js";

import type { BuildTxResponse } from "../api/types";
import { ApiClient } from "../api/client";
import { base64ToBytes, bytesToBase64 } from "../crypto/base64";
import { wipe } from "../crypto/memory";
import { unlockKeypair } from "../crypto/wallet";
import type { VaultBlob } from "../crypto/types";
import { getConnection } from "../solana/connection";
import { expectedActionFromSummary } from "../verify/fromSummary";
import { verifyTransaction } from "../verify/txVerifier";
import { PassphraseModal } from "./PassphraseModal";

interface TxReviewProps {
  vault: VaultBlob;
  apiClient: ApiClient;
  buildResponse: BuildTxResponse;
  /** Only present for open-position: the client-generated position keypair this transaction
   * needs as a second signer, held in memory since the moment the build request was made. */
  ephemeralPositionKeypair?: Keypair;
  onSigned: (signature: string) => void;
  onCancel: () => void;
}

/**
 * The approve screen. Nothing here should be read as a rubber stamp: verification runs once,
 * eagerly, against the exact bytes the backend returned, and a failure disables signing outright
 * -- there is no "sign anyway" override anywhere in this component. See verify/txVerifier.ts for
 * what the check does and does not cover.
 */
export function TxReview({
  vault,
  apiClient,
  buildResponse,
  ephemeralPositionKeypair,
  onSigned,
  onCancel,
}: TxReviewProps): JSX.Element {
  const walletPubkey = useMemo(() => new PublicKey(vault.publicKey), [vault.publicKey]);
  const unsignedBytes = useMemo(
    () => base64ToBytes(buildResponse.unsignedTransaction),
    [buildResponse.unsignedTransaction],
  );
  const expected = useMemo(
    () => expectedActionFromSummary(buildResponse.summary),
    [buildResponse.summary],
  );
  const verification = useMemo(
    () => verifyTransaction(unsignedBytes, { walletPubkey, expected }),
    [unsignedBytes, walletPubkey, expected],
  );

  const [showPassphrase, setShowPassphrase] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preSignSimError, setPreSignSimError] = useState<string | null>(null);

  async function handleApproveClicked(): Promise<void> {
    setError(null);
    setPreSignSimError(null);
    setBusy(true);
    try {
      // Re-simulate independently, against this app's own RPC connection, immediately before
      // asking for the passphrase -- closes the gap where the backend's own bundled simulation
      // result (or the RPC it used to produce one) cannot be trusted on its own (see
      // txVerifier.ts's module comment and README's threat-model notes).
      const tx = VersionedTransaction.deserialize(unsignedBytes);
      const sim = await getConnection().simulateTransaction(tx, { sigVerify: false });
      if (sim.value.err) {
        setPreSignSimError(
          `independent simulation failed: ${JSON.stringify(sim.value.err)} -- refusing to sign`,
        );
        return;
      }
      setShowPassphrase(true);
    } catch (err) {
      setPreSignSimError(
        `could not independently simulate this transaction, refusing to sign: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    } finally {
      setBusy(false);
    }
  }

  async function handlePassphraseSubmit(passphrase: string): Promise<void> {
    setBusy(true);
    setError(null);
    let keypair: Keypair | null = null;
    try {
      const connection = getConnection();

      // Blockhash freshness is checked once when the build response arrives and again here,
      // right before submission -- entering a passphrase can take a while, and a blockhash that
      // was valid when this screen opened may have expired by the time signing finishes.
      const currentHeight = await connection.getBlockHeight("confirmed");
      if (currentHeight > buildResponse.expiryLastValidBlockHeight) {
        setError("This transaction has expired (its blockhash is no longer valid). Please retry.");
        return;
      }

      keypair = await unlockKeypair(vault, passphrase);
      const tx = VersionedTransaction.deserialize(unsignedBytes);
      const signers = ephemeralPositionKeypair ? [keypair, ephemeralPositionKeypair] : [keypair];
      tx.sign(signers);

      const signedBytes = tx.serialize();
      const result = await apiClient.submitTransaction({
        signedTransaction: bytesToBase64(signedBytes),
        idempotencyKey: buildResponse.idempotencyKey,
      });
      setShowPassphrase(false);
      onSigned(result.signature);
    } catch (err) {
      setError(err instanceof Error ? err.message : "signing failed");
    } finally {
      // keypair.secretKey is a Uint8Array web3.js owns internally; wiping the local reference's
      // bytes here is best-effort (see crypto/memory.ts) but costs nothing to attempt, and drops
      // the only reference this component holds either way.
      if (keypair) wipe(keypair.secretKey);
      keypair = null;
      setBusy(false);
    }
  }

  return (
    <div className="screen">
      <h1>Review transaction</h1>
      <SummaryView summary={buildResponse.summary} />

      <div className={verification.ok ? "verify-ok" : "verify-fail"}>
        {verification.ok ? (
          <p>
            Verified: the transaction bytes match this summary exactly ({verification.decoded.dlmmAction},{" "}
            {verification.decoded.instructionCount} instruction(s)).
          </p>
        ) : (
          <p>Refusing to sign: {verification.reason}</p>
        )}
      </div>

      {buildResponse.simulation.error && (
        <p className="warning-text">Backend simulation reported: {buildResponse.simulation.error}</p>
      )}
      {preSignSimError && <p className="error-text">{preSignSimError}</p>}
      {error && <p className="error-text">{error}</p>}

      <p>Estimated network fee: {buildResponse.estimatedNetworkFeeLamports} lamports</p>

      <div className="modal-actions">
        <button onClick={onCancel} disabled={busy}>
          Cancel
        </button>
        <button disabled={!verification.ok || busy} onClick={() => void handleApproveClicked()}>
          {busy ? "Working..." : "Approve"}
        </button>
      </div>

      {showPassphrase && (
        <PassphraseModal
          title="Confirm your passphrase"
          description="Signing requires your passphrase every time -- this app never keeps your key unlocked between actions."
          confirmLabel="Sign and submit"
          submitting={busy}
          errorMessage={error}
          onSubmit={(p) => void handlePassphraseSubmit(p)}
          onCancel={() => setShowPassphrase(false)}
        />
      )}
    </div>
  );
}

function SummaryView({ summary }: { summary: BuildTxResponse["summary"] }): JSX.Element {
  switch (summary.action) {
    case "open-position":
      return (
        <dl>
          <dt>Pool</dt>
          <dd>
            {summary.tokenXSymbol}/{summary.tokenYSymbol} ({summary.poolAddress})
          </dd>
          <dt>Bin range</dt>
          <dd>
            {summary.lowerBinId} to {summary.lowerBinId + summary.width}
          </dd>
        </dl>
      );
    case "add-liquidity":
      return (
        <dl>
          <dt>Position</dt>
          <dd>{summary.positionAddress}</dd>
          <dt>Depositing</dt>
          <dd>
            {summary.amountXRaw} {summary.tokenXSymbol}
            {summary.amountXUsd !== null ? ` (~$${summary.amountXUsd.toFixed(2)})` : ""} and{" "}
            {summary.amountYRaw} {summary.tokenYSymbol}
            {summary.amountYUsd !== null ? ` (~$${summary.amountYUsd.toFixed(2)})` : ""}
          </dd>
          <dt>Slippage bound</dt>
          <dd>{(summary.maxActiveBinSlippageBps / 100).toFixed(2)}% price movement before this fails</dd>
        </dl>
      );
    case "remove-liquidity":
      return (
        <dl>
          <dt>Position</dt>
          <dd>{summary.positionAddress}</dd>
          <dt>Removing</dt>
          <dd>{(summary.bpsToRemove / 100).toFixed(2)}% of liquidity in the selected range</dd>
        </dl>
      );
    case "claim-fees":
      return (
        <dl>
          <dt>Position</dt>
          <dd>{summary.positionAddress}</dd>
          <dt>Estimated fees</dt>
          <dd>
            {summary.estimatedFeesXRaw} token X, {summary.estimatedFeesYRaw} token Y
          </dd>
        </dl>
      );
    case "close-position":
      return (
        <dl>
          <dt>Position</dt>
          <dd>{summary.positionAddress}</dd>
          <dt>Rent returned to</dt>
          <dd>{summary.rentReceiver}</dd>
        </dl>
      );
  }
}
