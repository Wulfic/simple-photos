/**
 * Modal dialog that initiates and manages a Chromecast session.
 *
 * Uses the browser-native Presentation API — no external scripts,
 * no Chrome extension required.  Works in Brave with media casting enabled.
 *
 * Flow:
 *   1. Dialog opens → initCast() is called.  Brave / Chrome scans the LAN for
 *      Chromecast devices using its built-in cast stack.
 *   2. If devices are found the button becomes active.  Click → native browser
 *      device picker appears.
 *   3. User selects a device → receiver page (/cast-view) loads on the
 *      Chromecast → "connected" state.
 *   4. Disconnect clears the session.
 *
 * Fallback: if the Chromecast cannot reach the local HTTPS server (self-signed
 * cert not in the system trust store), offer "Cast Tab" instructions so the
 * user can cast the browser tab via Brave's built-in cast button instead.
 */
import { useEffect, useState } from "react";
import {
  initCast,
  subscribeCastState,
  requestCastSession,
  endCastSession,
  type CastState,
} from "../utils/cast";

interface CastDialogProps {
  open: boolean;
  onClose: () => void;
}

export function CastIcon({ className = "w-5 h-5" }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className={className} aria-hidden="true">
      <path d="M21 3H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11z" />
    </svg>
  );
}

export default function CastDialog({ open, onClose }: CastDialogProps) {
  const [state, setState] = useState<CastState>("no_devices");
  const [device, setDevice] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showTabCastHelp, setShowTabCastHelp] = useState(false);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setError(null);
    setShowTabCastHelp(false);
    initCast().catch(() => {});
    const unsub = subscribeCastState((s, d) => {
      if (!active) return;
      setState(s);
      setDevice(d);
    });
    return () => {
      active = false;
      unsub();
    };
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  async function handleConnect() {
    setBusy(true);
    setError(null);
    try {
      await requestCastSession();
    } catch (e: unknown) {
      console.error("[cast] requestSession failed", e);
      const msg = (e instanceof Error) ? e.message : String(e);
      setError(msg || "Failed to start cast session");
    } finally {
      setBusy(false);
    }
  }

  async function handleDisconnect() {
    setBusy(true);
    try { await endCastSession(); } catch { /* ignore */ } finally { setBusy(false); }
  }

  // ── Tab-cast help panel ──────────────────────────────────────────────────
  const tabCastHelp = (
    <div className="mt-3 p-3 rounded-md bg-accent-50 dark:bg-accent-900/20 border border-accent-200 dark:border-accent-800 text-xs text-accent-800 dark:text-accent-200 space-y-1">
      <p className="font-medium">How to cast a tab in Brave:</p>
      <ol className="list-decimal list-inside space-y-0.5 text-accent-700 dark:text-accent-300">
        <li>Open the photo you want to cast in the viewer.</li>
        <li>Click the Cast icon in the Brave toolbar (or go to Menu → Cast…).</li>
        <li>Under "Cast to", choose <strong>Cast tab</strong>.</li>
        <li>Select your Chromecast device.</li>
      </ol>
      <p className="text-accent-600 dark:text-accent-400 pt-1">
        Tab casting works even with a local HTTPS certificate because the
        browser streams its rendered output — the Chromecast never fetches
        the URL directly.
      </p>
    </div>
  );

  // ── Body per state ───────────────────────────────────────────────────────
  let body: React.ReactNode;

  switch (state) {
    case "unsupported":
      body = (
        <div className="space-y-2">
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Your browser doesn't expose the Presentation API needed for
            automatic device discovery.  Casting requires a Chromium-based
            browser (Brave, Chrome, Edge) with media casting enabled.
          </p>
          <button
            onClick={() => setShowTabCastHelp((v) => !v)}
            className="text-xs text-accent-600 dark:text-accent-400 underline"
          >
            {showTabCastHelp ? "Hide" : "Show"} tab-casting instructions
          </button>
          {showTabCastHelp && tabCastHelp}
        </div>
      );
      break;

    case "no_devices":
      body = (
        <div className="space-y-3">
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Scanning for Chromecast devices on your network…  Make sure your
            computer and Chromecast are on the same Wi-Fi network.
          </p>
          <button
            onClick={handleConnect}
            disabled={busy}
            className="btn btn-primary btn-md w-full flex items-center justify-center"
          >
            <CastIcon className="w-4 h-4" />
            {busy ? "Opening picker…" : "Open device picker"}
          </button>
          <button
            onClick={() => setShowTabCastHelp((v) => !v)}
            className="w-full text-xs text-gray-700 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 underline"
          >
            {showTabCastHelp ? "Hide" : "No devices found? Try tab casting"}
          </button>
          {showTabCastHelp && tabCastHelp}
        </div>
      );
      break;

    case "available":
      body = (
        <div className="space-y-3">
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Cast devices are available on your network.  Click below to choose
            one and begin casting.
          </p>
          <button
            onClick={handleConnect}
            disabled={busy}
            className="btn btn-primary btn-md w-full flex items-center justify-center"
          >
            <CastIcon className="w-4 h-4" />
            {busy ? "Opening picker…" : "Choose a device"}
          </button>
          <p className="text-xs text-gray-700 dark:text-gray-400">
            Note: the Chromecast loads the receiver page directly from this
            server.  If your TLS certificate is only trusted in the browser
            (not the system trust store), use{" "}
            <button
              onClick={() => setShowTabCastHelp((v) => !v)}
              className="underline"
            >
              tab casting
            </button>{" "}
            instead.
          </p>
          {showTabCastHelp && tabCastHelp}
        </div>
      );
      break;

    case "connecting":
      body = (
        <div className="flex items-center gap-3 py-2">
          <div className="w-4 h-4 border-2 border-accent-500 border-t-transparent rounded-full animate-spin shrink-0" />
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Connecting{device ? ` to ${device}` : ""}…
          </p>
        </div>
      );
      break;

    case "connected":
      body = (
        <div className="space-y-3">
          <div className="flex items-center gap-3 p-3 rounded-md bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800">
            <span className="w-2.5 h-2.5 rounded-full bg-green-500 shrink-0" />
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium text-gray-900 dark:text-white truncate">
                Casting{device ? ` to ${device}` : ""}
              </p>
              <p className="text-xs text-gray-700 dark:text-gray-400">
                Open a photo in the viewer — it will appear on your TV.
              </p>
            </div>
          </div>
          <button
            onClick={handleDisconnect}
            disabled={busy}
            className="btn btn-danger btn-md w-full"
          >
            Stop casting
          </button>
        </div>
      );
      break;
  }

  return (
    <div
      className="fixed inset-0 z-[10000] flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="card shadow-pop w-full max-w-sm mx-4"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="cast-dialog-title"
      >
        <div className="px-5 py-4 border-b border-gray-200 dark:border-gray-700 flex items-center gap-2">
          <CastIcon className="w-5 h-5 text-gray-700 dark:text-gray-200" />
          <h2
            id="cast-dialog-title"
            className="text-base font-semibold text-gray-900 dark:text-white flex-1"
          >
            Cast to TV
          </h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="text-gray-700 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white text-xl leading-none px-1"
          >
            ×
          </button>
        </div>
        <div className="px-5 py-4">
          {error && (
            <p className="text-xs text-red-600 dark:text-red-400 mb-3">{error}</p>
          )}
          {body}
        </div>
      </div>
    </div>
  );
}
