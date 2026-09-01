/**
 * btoa/atob operate on a "binary string" (one code unit per byte, Latin1-range only), not on a
 * Uint8Array directly -- these two functions do the conversion in both directions. Hand-rolled
 * rather than a dependency: it is encoding, not cryptography, and every mainstream browser
 * (including Telegram's in-app WebViews) has had btoa/atob for well over a decade.
 */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i] as number);
  }
  return btoa(binary);
}

export function base64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
