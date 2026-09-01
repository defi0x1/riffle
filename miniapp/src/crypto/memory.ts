/**
 * Best-effort zeroing of sensitive byte buffers.
 *
 * Honest limitation: JavaScript gives no guarantee this actually scrubs memory. Overwriting a
 * Uint8Array's bytes removes that one reference to the plaintext, but:
 *  - the JS engine may have copied the bytes elsewhere (string interning, JIT-visible temporaries,
 *    a moved/compacted heap) that this call cannot reach or knows nothing about;
 *  - garbage collection is not synchronous or guaranteed to run at any particular time, so an
 *    unreferenced copy can sit in memory for a while regardless of what this function does;
 *  - swap, hibernation images, and crash dumps can persist memory contents to disk outside the
 *    JS runtime's control entirely.
 *
 * Calling this is still worth doing -- it shrinks the window a decrypted key is reachable from a
 * live reference -- but it is a mitigation, not a guarantee. Do not represent it as one elsewhere
 * in this codebase.
 */
export function wipe(bytes: Uint8Array): void {
  bytes.fill(0);
}

/**
 * Runs `fn` with a secret buffer and wipes it afterward, including when `fn` throws. Prefer this
 * over manual try/finally at call sites so the wipe is never accidentally skipped.
 */
export async function withSecretBytes<T>(
  bytes: Uint8Array,
  fn: (bytes: Uint8Array) => Promise<T> | T,
): Promise<T> {
  try {
    return await fn(bytes);
  } finally {
    wipe(bytes);
  }
}
