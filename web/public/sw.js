/**
 * Simple Photos — Service Worker
 *
 * Minimal service worker required for PWA installability. We intentionally
 * keep it network-passthrough (no offline cache) because:
 *   1. The app is already an SPA backed by a session-bound API; serving stale
 *      authenticated responses from cache would be a security risk.
 *   2. Vite produces hashed asset filenames, so the browser's HTTP cache is
 *      sufficient for static asset re-use.
 *
 * HTTPS: Service workers require HTTPS (or localhost). This worker forwards
 * all requests to the network unchanged, so TLS certificates are validated
 * by the browser exactly as if no service worker were present. Self-signed
 * certs on local networks work the same way — the user's browser decision
 * (accept/reject) applies here too.
 *
 * Bumping `SW_VERSION` forces all installed clients to fetch a new copy of
 * this file (browsers byte-compare service workers and re-install on any
 * change).
 */
const SW_VERSION = "2";

self.addEventListener("install", (event) => {
  // Activate immediately on first install / update.
  self.skipWaiting();
  event.waitUntil(Promise.resolve(SW_VERSION));
});

self.addEventListener("activate", (event) => {
  // Take control of any already-open tabs without requiring a reload.
  event.waitUntil(self.clients.claim());
});

/**
 * Fetch handler — pure network passthrough.
 *
 * A fetch listener is required for installability (browsers won't offer the
 * install prompt without one). We call event.respondWith(fetch(...)) so the
 * browser sees this as a proper controlled fetch rather than a SW bypass,
 * while still hitting the network exactly as the app requested.
 *
 * Non-GET requests, cross-origin requests (e.g. analytics, fonts from CDN)
 * and anything that fails (network error, TLS error) are handled gracefully:
 * - Cross-origin: let the browser handle it natively (no respondWith).
 * - GET same-origin: respondWith the network fetch; if the network is down
 *   return a minimal 503 so the app receives a real Response object rather
 *   than an opaque network error.
 */
self.addEventListener("fetch", (event) => {
  const { request } = event;

  // Let cross-origin requests pass through the browser unchanged.
  // (avoids mixed-content and CORS complications)
  try {
    const url = new URL(request.url);
    if (url.origin !== self.location.origin) return;
  } catch {
    return;
  }

  // Only intercept GET requests — mutations must always reach the server.
  if (request.method !== "GET") return;

  event.respondWith(
    fetch(request).catch(() => {
      // Network or TLS error — return a 503 so React's fetch error handlers
      // fire normally rather than getting an unhandled TypeError.
      return new Response(JSON.stringify({ error: "offline" }), {
        status: 503,
        statusText: "Service Unavailable",
        headers: { "Content-Type": "application/json" },
      });
    })
  );
});

