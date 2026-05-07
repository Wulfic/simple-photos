/**
 * Google Cast (Chromecast) sender integration.
 *
 * Uses the browser-native **Presentation API** (W3C standard, built into
 * Chrome / Brave / Edge 72+) instead of the Google Cast Web SDK script.
 * This avoids the `cast_sender.js` CDN script that Brave Shields blocks —
 * no external scripts, no extension required.
 *
 * The receiver page is served from this app at `/cast-view`.  It listens for
 * `PresentationConnection` messages and renders the current photo full-screen.
 *
 * Known limitation: Chromecast fetches the receiver URL directly, so it must
 * trust the server's TLS certificate.  For local self-signed certs add the CA
 * to the SYSTEM trust store (not only the browser).  Alternatively use the
 * browser's built-in "Cast tab" button — that streams the rendered page to the
 * Chromecast without any direct certificate checks.
 */

export type CastState = "no_devices" | "available" | "connecting" | "connected" | "unsupported";

type Listener = (state: CastState, deviceName?: string) => void;

const _listeners = new Set<Listener>();
let _state: CastState = "no_devices";
let _device: string | undefined;

let _req: PresentationRequest | null = null;
let _avail: PresentationAvailability | null = null;
let _conn: PresentationConnection | null = null;
let _initDone = false;

function receiverUrl(): string {
  return `${window.location.origin}/cast-view`;
}

function emit() {
  _listeners.forEach((l) => {
    try { l(_state, _device); } catch (e) { console.error("[cast] listener error", e); }
  });
}

function setState(s: CastState, d?: string) {
  _state = s;
  _device = d;
  emit();
}

/** Subscribe to cast state changes. Returns unsubscribe fn. Fires immediately with current state. */
export function subscribeCastState(listener: Listener): () => void {
  _listeners.add(listener);
  listener(_state, _device);
  return () => _listeners.delete(listener);
}

export function getCastState(): { state: CastState; device?: string } {
  return { state: _state, device: _device };
}

/**
 * Initialise cast using the native Presentation API.
 * Safe to call multiple times — subsequent calls are no-ops.
 */
export async function initCast(): Promise<void> {
  if (_initDone) return;
  _initDone = true;

  if (typeof window.PresentationRequest === "undefined") {
    setState("unsupported");
    return;
  }

  try {
    _req = new PresentationRequest([receiverUrl()]);

    // Register as the page's default presentation so browsers may show
    // a Cast icon in the address bar automatically.
    if (navigator.presentation) {
      navigator.presentation.defaultRequest = _req;
    }

    const avail = await _req.getAvailability();
    _avail = avail;
    setState(avail.value ? "available" : "no_devices");

    avail.onchange = () => {
      if (_state === "connecting" || _state === "connected") return;
      setState(avail.value ? "available" : "no_devices");
    };
  } catch (e) {
    // SecurityError if page is not served over a secure context,
    // or NotSupportedError if the browser does not implement this feature.
    console.warn("[cast] initCast:", e);
    setState("unsupported");
  }
}

/** Wire lifecycle handlers onto a freshly-opened PresentationConnection. */
function wireConnection(conn: PresentationConnection) {
  _conn = conn;
  setState("connected");

  conn.onclose = () => {
    _conn = null;
    setState(_avail?.value ? "available" : "no_devices");
  };
  conn.onterminate = () => {
    _conn = null;
    setState(_avail?.value ? "available" : "no_devices");
  };
}

/**
 * Open the browser's native device picker and connect to the chosen Chromecast.
 * Resolves when connected; throws on failure (but NOT when the user cancels).
 */
export async function requestCastSession(): Promise<void> {
  if (!_req) await initCast();
  if (!_req) throw new Error("Presentation API not available in this browser");

  let conn: PresentationConnection;
  try {
    conn = await _req.start();
  } catch (e: unknown) {
    // AbortError / "cancel" = user dismissed the picker — not an error.
    const name = (e instanceof Error) ? e.name : String(e);
    if (name === "AbortError" || name === "cancel" || String(e).includes("cancel")) return;
    throw e;
  }

  wireConnection(conn);
}

/** Terminate the current cast session, if any. */
export async function endCastSession(): Promise<void> {
  if (_conn) {
    try { _conn.terminate(); } catch { /* already closed */ }
    _conn = null;
  }
  setState(_avail?.value ? "available" : "no_devices");
}

/**
 * Send a photo URL to the active receiver page over the PresentationConnection.
 * No-op when no session is active or the URL is not HTTP(S).
 */
export function castMedia(url: string, _contentType = "image/jpeg"): void {
  if (_conn?.state !== "connected") return;
  if (!/^https?:/i.test(url)) return; // blob:/data: not reachable by Chromecast
  try {
    _conn.send(JSON.stringify({ type: "LOAD_MEDIA", url }));
  } catch (e) {
    console.warn("[cast] send failed", e);
  }
}
