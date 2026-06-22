/**
 * Secure-gallery unlock token (client side).
 *
 * When the user unlocks their secure albums (re-entering the account
 * password), the server returns a short-lived token. The server now requires
 * that token to serve secure-album media via the generic photo/blob endpoints,
 * so it must accompany every such request:
 *
 * - `fetch()` based calls send it as the `X-Gallery-Token` header
 *   (injected centrally in `api/core.ts`).
 * - `<img>` / `<video>` `src` URLs — which cannot set headers — carry it as a
 *   `gallery_token` query parameter via {@link appendGalleryTokenParam}.
 *
 * We keep the token in `sessionStorage` (not `localStorage`) so it is scoped to
 * the tab/session and cleared when the browser session ends — appropriate for a
 * second-factor gate. It is also cleared on logout.
 *
 * Appending the token to a non-secure media URL is harmless: the server only
 * checks it for items that actually live in a secure gallery.
 */

const STORAGE_KEY = "sp_gallery_token";

/**
 * Server-side unlock-token lifetime, in seconds. Mirrors
 * `server/src/gallery/secure_token.rs::TOKEN_TTL_SECS` (1 hour). The client
 * keeps its own copy so it can drop back to the password gate *before* the
 * server would start rejecting the token — otherwise a token that has expired
 * server-side still lives in `sessionStorage`, the page treats the user as
 * "unlocked", and every secure request 401s with no way to re-enter the
 * password.
 */
const TOKEN_TTL_SECS = 3600;

/**
 * Safety margin (seconds) subtracted from the TTL so we re-prompt a little
 * before the hard server expiry, covering request round-trip + minor clock
 * skew. Also used as the tolerance for a token that appears slightly
 * future-dated relative to the client clock.
 */
const TOKEN_SKEW_SECS = 60;

/**
 * Parse the `issued_at` unix timestamp out of a `sec_<issued_at>_<tag>` token.
 * Returns `null` for anything that isn't a well-formed token.
 */
function parseIssuedAt(token: string): number | null {
  const parts = token.split("_");
  if (parts.length < 3 || parts[0] !== "sec") return null;
  const issuedAt = Number(parts[1]);
  return Number.isFinite(issuedAt) ? issuedAt : null;
}

/**
 * True only when a persisted unlock token exists *and* is still within its
 * server-side TTL. This — not the mere presence of a token string — is what the
 * secure-album page must use to decide whether to skip the password gate.
 */
export function hasFreshGalleryToken(): boolean {
  const token = getGalleryToken();
  if (!token) return false;
  const issuedAt = parseIssuedAt(token);
  if (issuedAt === null) return false;
  const ageSecs = Date.now() / 1000 - issuedAt;
  // Reject expired tokens, and tokens dated implausibly far in the future
  // (clock skew beyond the grace window ⇒ treat as untrustworthy → re-prompt).
  return ageSecs >= -TOKEN_SKEW_SECS && ageSecs < TOKEN_TTL_SECS - TOKEN_SKEW_SECS;
}

/**
 * Whether an error thrown from a secure-gallery request is the server saying
 * the unlock token is missing/expired/invalid (HTTP 401 with one of the
 * gallery-token messages from `secure.rs::list_gallery_items`). When this is
 * true the caller should clear the token and return the user to the password
 * gate so they can re-unlock — as opposed to showing a generic failure.
 */
export function isGalleryTokenRejection(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err ?? "");
  return /gallery token|unlock the gallery/i.test(msg);
}

/** Persist the unlock token for the current session. */
export function setGalleryToken(token: string): void {
  try {
    if (token) sessionStorage.setItem(STORAGE_KEY, token);
  } catch {
    /* sessionStorage unavailable (private mode quota, etc.) — non-fatal */
  }
}

/** Read the current unlock token, or `null` if the user hasn't unlocked. */
export function getGalleryToken(): string | null {
  try {
    return sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

/** Drop the unlock token (on logout / lock). */
export function clearGalleryToken(): void {
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    /* non-fatal */
  }
}

/**
 * Append `?gallery_token=…` (or `&gallery_token=…`) to a media URL when an
 * unlock token is present. No-op when the user hasn't unlocked a secure album.
 */
export function appendGalleryTokenParam(url: string): string {
  const token = getGalleryToken();
  if (!token) return url;
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}gallery_token=${encodeURIComponent(token)}`;
}
