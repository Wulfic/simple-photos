/**
 * Generate a RFC4122-ish v4 UUID.
 *
 * `crypto.randomUUID()` is only available in secure contexts (HTTPS / localhost).
 * This app is frequently served over plain HTTP on a LAN, so we fall back to a
 * `crypto.getRandomValues`-backed generator when `randomUUID` is missing.
 */
export function randomUuid(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  // Per RFC4122 §4.4: set version (4) and variant (10xx) bits.
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
