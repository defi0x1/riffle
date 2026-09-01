import type { VaultBlob } from "../crypto/types";

/**
 * Device-local storage for the encrypted vault, scoped to this origin by the browser -- chosen
 * over Telegram's CloudStorage API specifically so a compromised Telegram account does not also
 * hand over the ciphertext (CloudStorage syncs through Telegram's own servers; IndexedDB does
 * not). The tradeoff this accepts: no multi-device sync, and clearing site data or losing the
 * device loses the ciphertext for good -- see components/ExportKey for the mitigation (a
 * deliberate, warned export step), and README.md for why this is not treated as a bug to fix
 * later.
 */

const DB_NAME = "riffle-wallet";
const DB_VERSION = 1;
const STORE_NAME = "vault";
const RECORD_KEY = "current";

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (!("indexedDB" in globalThis)) {
      reject(new Error("IndexedDB is not available in this browser context"));
      return;
    }
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME);
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("failed to open IndexedDB"));
  });
}

export async function saveVault(vault: VaultBlob): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      tx.objectStore(STORE_NAME).put(vault, RECORD_KEY);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("failed to save vault"));
      tx.onabort = () => reject(tx.error ?? new Error("save transaction aborted"));
    });
  } finally {
    db.close();
  }
}

/**
 * Returns null when there is nothing stored -- a cleared cache, a fresh install, or a device
 * that never registered a wallet all look identical from here, and the UI must say so plainly
 * (see components' empty-state copy) rather than treat it as an unexpected error.
 */
export async function loadVault(): Promise<VaultBlob | null> {
  const db = await openDb();
  try {
    return await new Promise<VaultBlob | null>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readonly");
      const request = tx.objectStore(STORE_NAME).get(RECORD_KEY);
      request.onsuccess = () => resolve((request.result as VaultBlob | undefined) ?? null);
      request.onerror = () => reject(request.error ?? new Error("failed to load vault"));
    });
  } finally {
    db.close();
  }
}

export async function clearVault(): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      tx.objectStore(STORE_NAME).delete(RECORD_KEY);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("failed to clear vault"));
    });
  } finally {
    db.close();
  }
}
