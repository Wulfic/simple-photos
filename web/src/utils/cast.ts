/**
 * Google Cast (Chromecast) sender integration.
 *
 * Loads lazily once the SDK script (added to index.html) signals readiness via
 * `window.__onGCastApiAvailable`. Exposes a tiny event-driven API used by the
 * CastDialog component and the AppHeader menu entry.
 *
 * Casting media: call `castMedia(url, contentType)` while a session is active.
 * For Simple Photos this is intended for HTTP(S)-reachable photo URLs — blob:
 * URLs (e.g. decrypted secure-album content) cannot be cast.
 */

// Minimal ambient typings — we avoid pulling in @types/chromecast-caf-sender.
declare global {
  interface Window {
    __onGCastApiAvailable?: (isAvailable: boolean) => void;
    __castApiPreload?: {
      received: boolean;
      available: boolean;
      listeners: ((isAvailable: boolean) => void)[];
    };
    chrome?: any;
    cast?: any;
  }
}

export type CastState = "no_devices" | "available" | "connecting" | "connected" | "unsupported";

type Listener = (state: CastState, deviceName?: string) => void;

const listeners = new Set<Listener>();
let currentState: CastState = "no_devices";
let currentDevice: string | undefined;
let initStarted = false;
let initResolved = false;

/**
 * Cause of an `unsupported` state, set by `initCast()`. Surfaced to the
 * UI by `getCastUnsupportedReason()` so the user gets actionable feedback
 * (e.g. "Cast requires HTTPS") instead of a generic "unsupported".
 */
export type CastUnsupportedReason =
  | "insecure_origin"
  | "sdk_blocked"
  | "sdk_load_timeout"
  | "init_failed"
  | null;

let unsupportedReason: CastUnsupportedReason = null;

export function getCastUnsupportedReason(): CastUnsupportedReason {
  return unsupportedReason;
}

/**
 * Returns true when the current page origin is one the Cast Sender SDK
 * will accept. Cast requires a secure context — HTTPS, `localhost`, or
 * `127.0.0.1`. Plain HTTP to a LAN IP silently fails because
 * `window.chrome.cast` is never exposed in non-secure contexts.
 */
export function isCastOriginSupported(): boolean {
  if (typeof window === "undefined") return false;
  if (window.isSecureContext) return true;
  const host = window.location.hostname;
  // `isSecureContext` already covers localhost in modern browsers, but be
  // defensive in case of a polyfill / older Chromium.
  return host === "localhost" || host === "127.0.0.1" || host === "::1";
}

function emit() {
  for (const l of listeners) {
    try {
      l(currentState, currentDevice);
    } catch (e) {
      console.error("[cast] listener error", e);
    }
  }
}

function setState(state: CastState, deviceName?: string) {
  currentState = state;
  currentDevice = deviceName;
  emit();
}

/**
 * Subscribe to cast state changes. Returns unsubscribe function.
 * Listener is called immediately with the current state.
 */
export function subscribeCastState(listener: Listener): () => void {
  listeners.add(listener);
  listener(currentState, currentDevice);
  return () => listeners.delete(listener);
}

export function getCastState(): { state: CastState; device?: string } {
  return { state: currentState, device: currentDevice };
}

/**
 * Initialise the Cast framework. Safe to call multiple times — subsequent
 * calls are no-ops. Returns a promise that resolves once the SDK reports
 * its initial availability.
 */
export function initCast(): Promise<void> {
  if (initResolved) return Promise.resolve();
  if (initStarted) {
    return new Promise((resolve) => {
      const off = subscribeCastState(() => {
        if (initResolved) {
          off();
          resolve();
        }
      });
    });
  }
  initStarted = true;

  // Fail fast on insecure origins — the Cast SDK never exposes
  // `window.chrome.cast` outside a secure context, so waiting 12s for the
  // SDK callback just to land on "unsupported" wastes the user's time and
  // hides the real cause.
  if (!isCastOriginSupported()) {
    initResolved = true;
    unsupportedReason = "insecure_origin";
    setState("unsupported");
    return Promise.resolve();
  }

  return new Promise((resolve) => {
    // Hard timeout: if the SDK never loads (offline / blocked / network slow),
    // report unsupported. Bumped to 12s because Brave + slow connections can
    // take >5s to fetch gstatic.com when Shields lazily allow the request.
    const timeout = window.setTimeout(() => {
      if (!initResolved) {
        initResolved = true;
        unsupportedReason = "sdk_load_timeout";
        setState("unsupported");
        resolve();
      }
    }, 12000);

    const onAvailable = (isAvailable: boolean) => {
      window.clearTimeout(timeout);
      if (!isAvailable || !window.cast?.framework) {
        initResolved = true;
        unsupportedReason = "sdk_blocked";
        setState("unsupported");
        resolve();
        return;
      }

      try {
        const cf = window.cast.framework;
        const ctx = cf.CastContext.getInstance();
        ctx.setOptions({
          // Default Media Receiver — plays generic image/video URLs.
          receiverApplicationId:
            window.chrome?.cast?.media?.DEFAULT_MEDIA_RECEIVER_APP_ID || "CC1AD845",
          autoJoinPolicy: cf.AutoJoinPolicy.ORIGIN_SCOPED,
        });

        // Initial state mapping
        const mapState = (s: string): CastState => {
          switch (s) {
            case "NO_DEVICES_AVAILABLE":
              return "no_devices";
            case "NOT_CONNECTED":
              return "available";
            case "CONNECTING":
              return "connecting";
            case "CONNECTED":
              return "connected";
            default:
              return "available";
          }
        };

        const refresh = () => {
          const sdkState = ctx.getCastState();
          const session = ctx.getCurrentSession();
          const device = session?.getCastDevice?.()?.friendlyName;
          setState(mapState(sdkState), device);
        };

        ctx.addEventListener(cf.CastContextEventType.CAST_STATE_CHANGED, refresh);
        ctx.addEventListener(cf.CastContextEventType.SESSION_STATE_CHANGED, refresh);

        refresh();
        initResolved = true;
        resolve();
      } catch (e) {
        console.error("[cast] init failed", e);
        initResolved = true;
        unsupportedReason = "init_failed";
        setState("unsupported");
        resolve();
      }
    };

    // Two paths:
    //   1. The inline preload script in index.html already received the SDK
    //      callback before our React bundle ran — read the cached result.
    //   2. The SDK has not arrived yet — register a listener with the preload
    //      shim so we get notified when it does.
    const preload = window.__castApiPreload;
    if (preload?.received) {
      onAvailable(preload.available);
    } else if (preload) {
      preload.listeners.push(onAvailable);
    } else {
      // No preload shim (older index.html) — fall back to overwriting the
      // global directly. Race-prone but preserves backwards compatibility.
      window.__onGCastApiAvailable = onAvailable;
    }
  });
}

/**
 * Show the native Chromecast device picker and connect to the chosen device.
 * Resolves once a session is established or the user cancels.
 */
export async function requestCastSession(): Promise<void> {
  await initCast();
  if (!window.cast?.framework) throw new Error("Cast SDK not available");
  const ctx = window.cast.framework.CastContext.getInstance();
  try {
    await ctx.requestSession();
  } catch (e: any) {
    // "cancel" is the normal user-cancel path — silence it.
    if (e === "cancel" || e?.code === "cancel") return;
    throw e;
  }
}

/** End the current cast session, if any. */
export async function endCastSession(stopCasting = true): Promise<void> {
  if (!window.cast?.framework) return;
  const ctx = window.cast.framework.CastContext.getInstance();
  const session = ctx.getCurrentSession();
  if (session) await session.endSession(stopCasting);
}

/**
 * Send a media URL (image or video) to the active cast session.
 * No-op when no session is active. Resolves when the receiver loads media.
 */
export async function castMedia(
  url: string,
  contentType: string = "image/jpeg",
  metadata?: { title?: string }
): Promise<void> {
  if (!window.cast?.framework || !window.chrome?.cast?.media) return;
  if (!/^https?:/i.test(url)) {
    // Chromecast cannot fetch blob:/data: URLs — silently skip.
    return;
  }
  const ctx = window.cast.framework.CastContext.getInstance();
  const session = ctx.getCurrentSession();
  if (!session) return;

  const mediaInfo = new window.chrome.cast.media.MediaInfo(url, contentType);
  const meta = new window.chrome.cast.media.GenericMediaMetadata();
  if (metadata?.title) meta.title = metadata.title;
  mediaInfo.metadata = meta;

  const request = new window.chrome.cast.media.LoadRequest(mediaInfo);
  await session.loadMedia(request);
}
