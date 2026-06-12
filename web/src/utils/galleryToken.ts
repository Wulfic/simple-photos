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
