import { useAuthStore } from "../store/auth";

export const BASE = "/api";

/**
 * Centralized, security-hardened API client.
 *
 * Security features:
 * - X-Requested-With header on all requests (basic CSRF protection)
 * - Full refresh-token rotation (server returns new refresh token on each refresh)
 * - Automatic token refresh on 401, with single-flight deduplication
 * - Rate-limit aware: surfaces 429 messages to the user
 * - Rejects blobs with non-2xx status even on download
 * - Never logs tokens
 */

// ── Single-flight refresh deduplication ──────────────────────────────────────
// If multiple requests 401 at the same time, only one refresh attempt runs.
let refreshPromise: Promise<boolean> | null = null;

export async function request<T>(
  path: string,
  options: RequestInit = {}
): Promise<T> {
  const { accessToken } = useAuthStore.getState();

  const headers: Record<string, string> = {
    ...(options.headers as Record<string, string>),
    // Basic CSRF protection — server can reject requests without this header
    "X-Requested-With": "SimplePhotos",
  };

  if (accessToken) {
    headers["Authorization"] = `Bearer ${accessToken}`;
  }

  // Only set Content-Type for JSON bodies (not raw blob uploads)
  if (options.body && typeof options.body === "string") {
    headers["Content-Type"] = "application/json";
  }

  const res = await fetch(`${BASE}${path}`, { ...options, headers });

  // ── Rate limiting ────────────────────────────────────────────────────────
  if (res.status === 429) {
    const retryAfter = res.headers.get("Retry-After");
    const err = await res.json().catch(() => ({ error: "Too many requests" }));
    const msg = retryAfter
      ? `${err.error || "Too many requests"}. Try again in ${retryAfter}s.`
      : err.error || "Too many requests. Please wait and try again.";
    throw new Error(msg);
  }

  // ── Automatic token refresh on 401 ──────────────────────────────────────
  // Skip refresh logic for auth endpoints — their 401s are real errors
  // (wrong password, invalid token), not expired-session indicators.
  const isAuthEndpoint = path.startsWith("/auth/");
  if (res.status === 401 && !isAuthEndpoint) {
    const refreshed = await tryRefresh();
    if (refreshed) {
      const newToken = useAuthStore.getState().accessToken;
      headers["Authorization"] = `Bearer ${newToken}`;
      const retry = await fetch(`${BASE}${path}`, { ...options, headers });

      if (retry.status === 429) {
        throw new Error("Too many requests. Please wait and try again.");
      }
      if (!retry.ok) {
        const rawText = await retry.text().catch(() => "");
        let errorMessage: string;
        try {
          const parsed = JSON.parse(rawText);
          errorMessage = parsed.error || `HTTP ${retry.status}`;
        } catch {
          errorMessage = rawText
            ? `HTTP ${retry.status}: ${rawText.substring(0, 200)}`
            : `HTTP ${retry.status}`;
        }
        console.error(`[API] ${options.method || "GET"} ${path} failed after token refresh: ${retry.status}`, rawText.substring(0, 500));
        throw new Error(errorMessage);
      }
      if (retry.status === 204) return undefined as T;
      const retryText = await retry.text();
      if (!retryText) return undefined as T;
      return JSON.parse(retryText) as T;
    }
    // Refresh failed — force logout
    useAuthStore.getState().logout();
    throw new Error("Session expired. Please sign in again.");
  }

  if (!res.ok) {
    const rawText = await res.text().catch(() => "");
    let errorMessage: string;
    try {
      const parsed = JSON.parse(rawText);
      errorMessage = parsed.error || `HTTP ${res.status}`;
    } catch {
      errorMessage = rawText
        ? `HTTP ${res.status}: ${rawText.substring(0, 200)}`
        : `HTTP ${res.status}`;
    }
    console.error(`[API] ${options.method || "GET"} ${path} failed: ${res.status}`, rawText.substring(0, 500));
    throw new Error(errorMessage);
  }

  if (res.status === 204) return undefined as T;

  // Handle empty response bodies (e.g. 200 OK with no content)
  const text = await res.text();
  if (!text) return undefined as T;

  // Guard against SPA fallback returning HTML for unmatched API routes.
  // This happens when the server binary is stale or a route isn't registered.
  const contentType = res.headers.get("content-type") || "";
  if (contentType.includes("text/html") || text.trimStart().startsWith("<!")) {
    console.error(
      `[API] ${options.method || "GET"} ${path} returned HTML instead of JSON. ` +
      `The server may need to be rebuilt, or this endpoint is not registered.`
    );
    throw new Error(
      `Server returned HTML instead of JSON for ${path}. ` +
      `Please rebuild the server or check that the endpoint exists.`
    );
  }

  return JSON.parse(text) as T;
}

/**
 * Attempt to refresh the access token.
 *
 * Supports full token rotation: the server returns a NEW refresh token
 * alongside the new access token. Both are persisted.
 *
 * Uses single-flight deduplication so concurrent 401s don't cause
 * multiple refresh attempts (which would fail with revoked-token detection).
 */
export async function tryRefresh(): Promise<boolean> {
  // If a refresh is already in flight, piggyback on it
  if (refreshPromise) return refreshPromise;

  refreshPromise = (async () => {
    const { refreshToken, setTokens } = useAuthStore.getState();
    if (!refreshToken) return false;

    try {
      const res = await fetch(`${BASE}/auth/refresh`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Requested-With": "SimplePhotos",
        },
        body: JSON.stringify({ refresh_token: refreshToken }),
      });
      if (!res.ok) return false;

      const data = await res.json();
      // Server returns rotated refresh token — persist both new tokens
      const newRefresh = data.refresh_token || refreshToken;
      setTokens(data.access_token, newRefresh);
      return true;
    } catch {
      return false;
    }
  })();

  try {
    return await refreshPromise;
  } finally {
    refreshPromise = null;
  }
}

/**
 * Download raw binary data from a URL with automatic 401 token refresh.
 * Used for non-JSON endpoints (photo files, thumbnails, blobs) where the
 * standard `request()` helper can't be used because it parses JSON.
 */
export async function downloadRaw(url: string): Promise<ArrayBuffer> {
  const { accessToken } = useAuthStore.getState();
  const headers: Record<string, string> = {
    "X-Requested-With": "SimplePhotos",
  };
  if (accessToken) {
    headers["Authorization"] = `Bearer ${accessToken}`;
  }

  const res = await fetch(url, { headers });

  if (res.status === 401) {
    const refreshed = await tryRefresh();
    if (refreshed) {
      const newToken = useAuthStore.getState().accessToken;
      headers["Authorization"] = `Bearer ${newToken}`;
      const retry = await fetch(url, { headers });
      if (!retry.ok) {
        console.error(`[API] Download ${url} failed after refresh: ${retry.status}`);
        throw new Error(`Download failed: HTTP ${retry.status}`);
      }
      return retry.arrayBuffer();
    }
    useAuthStore.getState().logout();
    throw new Error("Session expired");
  }

  if (!res.ok) {
    console.error(`[API] Download ${url} failed: ${res.status}`);
    throw new Error(`Download failed: HTTP ${res.status}`);
  }
  return res.arrayBuffer();
}
