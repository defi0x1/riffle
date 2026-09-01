import { hmac } from "@noble/hashes/hmac";
import { sha512 } from "@noble/hashes/sha512";

/**
 * SLIP-0010 hierarchical derivation for ed25519, hand-rolled against the spec rather than
 * depending on a wrapper package: ed25519 has no public-key derivation without hardening
 * (unlike secp256k1), so the whole scheme is just two HMAC-SHA512 calls, well within what is
 * reasonable to implement directly and verify against published test vectors (see
 * tests/slip10.test.ts). Uses @noble/hashes, already a dependency for other hashing in this
 * codebase, instead of pulling in a dedicated key-derivation package for ~30 lines of logic.
 */

const ED25519_SEED_KEY = new TextEncoder().encode("ed25519 seed");
const HARDENED_OFFSET = 0x80000000;

export interface Slip10Node {
  key: Uint8Array; // 32-byte ed25519 private seed
  chainCode: Uint8Array; // 32 bytes
}

function hmacSha512(key: Uint8Array, data: Uint8Array): Uint8Array {
  return hmac(sha512, key, data);
}

export function masterNodeFromSeed(seed: Uint8Array): Slip10Node {
  const I = hmacSha512(ED25519_SEED_KEY, seed);
  return { key: I.slice(0, 32), chainCode: I.slice(32, 64) };
}

/**
 * Every ed25519 SLIP-0010 derivation step is hardened -- there is no non-hardened child key
 * scheme for this curve, so `index` here is always the unhardened path component; the hardened
 * offset is added internally.
 */
export function deriveHardenedChild(parent: Slip10Node, index: number): Slip10Node {
  if (!Number.isInteger(index) || index < 0 || index >= HARDENED_OFFSET) {
    throw new Error(`path index out of range: ${index}`);
  }
  const hardenedIndex = index + HARDENED_OFFSET;
  const data = new Uint8Array(1 + 32 + 4);
  data[0] = 0x00;
  data.set(parent.key, 1);
  new DataView(data.buffer).setUint32(33, hardenedIndex, false); // big-endian, per SLIP-0010
  const I = hmacSha512(parent.chainCode, data);
  return { key: I.slice(0, 32), chainCode: I.slice(32, 64) };
}

/**
 * Derives the ed25519 seed at `m/44'/501'/0'/0'` -- the path Phantom and Solflare use for a
 * wallet's default account, so a phrase created or imported here produces the same keypair those
 * wallets would derive from it, and an exported phrase is portable rather than locked to this
 * app. All four path components are hardened, matching every other ed25519 SLIP-0010 wallet.
 */
export function deriveSolanaSeed(seed: Uint8Array): Uint8Array {
  let node = masterNodeFromSeed(seed);
  for (const index of [44, 501, 0, 0]) {
    node = deriveHardenedChild(node, index);
  }
  return node.key;
}
