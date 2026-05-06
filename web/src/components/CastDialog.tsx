/**
 * Modal dialog that initiates and manages a Google Cast (Chromecast) session.
 *
 * Flow:
 *   1. On open: initialise the Cast SDK if needed, then show a status panel.
 *   2. User clicks "Search for devices" → SDK presents the native device
 *      picker. After selection, the dialog reflects "connected" state and
 *      the connected device's friendly name.
 *   3. User can disconnect from this dialog while connected.
 */
import { useEffect, useState } from "react";
import {
  initCast,
  subscribeCastState,
  requestCastSession,
  endCastSession,
  getCastUnsupportedReason,
  type CastState,
  type CastUnsupportedReason,
} from "../utils/cast";

interface CastDialogProps {
  open: boolean;
  onClose: () => void;
}

function CastIcon({ className = "w-5 h-5" }: { className?: string }) {
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
  const [unsupportedReason, setUnsupportedReason] =
    useState<CastUnsupportedReason>(null);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setError(null);
    initCast().catch(() => {});
    const unsub = subscribeCastState((s, d) => {
      if (!active) return;
      setState(s);
      setDevice(d);
      // Refresh the reason whenever state flips — only meaningful when
      // `s === "unsupported"`, but cheap to read otherwise.
      setUnsupportedReason(getCastUnsupportedReason());
    });
    return () => {
      active = false;
      unsub();
    };
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  async function handleConnect() {
    setBusy(true);
    setError(null);
    try {
      await requestCastSession();
    } catch (e: any) {
      console.error("[cast] requestSession failed", e);
      setError(typeof e === "string" ? e : e?.description || e?.message || "Failed to start cast session");
    } finally {
      setBusy(false);
    }
  }

  async function handleDisconnect() {
    setBusy(true);
    setError(null);
    try {
      await endCastSession(true);
    } catch (e: any) {
      console.error("[cast] endSession failed", e);
      setError(e?.message || "Failed to disconnect");
    } finally {
      setBusy(false);
    }
  }

  let body: React.ReactNode;
  switch (state) {
    case "unsupported":
      body = (
        <div className="space-y-3 text-sm text-gray-600 dark:text-gray-300">
          {unsupportedReason === "insecure_origin" ? (
            <>
              <div className="rounded-md border border-amber-300 bg-amber-50 dark:bg-amber-900/20 dark:border-amber-700 p-3">
                <p className="font-semibold text-amber-900 dark:text-amber-200">
                  Cast requires a secure connection (HTTPS).
                </p>
                <p className="mt-1 text-amber-800 dark:text-amber-300">
                  This page is loaded over plain HTTP, so Chrome and Brave
                  refuse to expose the Cast SDK. Reach the server over
                  <code className="mx-1">https://</code> or open it on
                  <code className="mx-1">http://localhost</code> to enable
                  casting.
                </p>
              </div>
              <p className="text-xs text-gray-500">
                Configure TLS in the server’s welcome wizard or via
                <code className="mx-1">config.toml</code>.
              </p>
            </>
          ) : (
            <p>
              Casting isn’t available in this browser. Google Cast requires a Chromium-based
              browser (Chrome, Edge, Brave) on the same network as your Chromecast device.
            </p>
          )}
          <details className="rounded-md border border-gray-200 dark:border-gray-700 p-3">
            <summary className="cursor-pointer font-medium">Brave: enable Media Router</summary>
            <ol className="list-decimal pl-5 mt-2 space-y-1">
              <li>
                Open <code>brave://settings/extensions</code> and turn on
                <strong> “Media Router”</strong> (Hangouts/Cast).
              </li>
              <li>
                Lower Brave Shields for this site (click the lion icon → set
                Shields to <em>Down</em>) so the Cast SDK script can load from
                gstatic.com.
              </li>
              <li>Reload the page.</li>
            </ol>
          </details>
          <p className="text-xs text-gray-500">
            No client install is required — the Cast SDK is loaded from Google’s CDN.
          </p>
        </div>
      );
      break;
    case "no_devices":
      body = (
        <div className="space-y-3">
          <p className="text-sm text-gray-600 dark:text-gray-300">
            No Chromecast devices were detected on this network. Make sure your computer
            and Chromecast are on the same Wi-Fi network and try again.
          </p>
          <button
            onClick={handleConnect}
            disabled={busy}
            className="w-full px-4 py-2 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium disabled:opacity-50"
          >
            Search for devices
          </button>
        </div>
      );
      break;
    case "available":
      body = (
        <div className="space-y-3">
          <p className="text-sm text-gray-600 dark:text-gray-300">
            Cast devices are available on your network. Click below to choose one.
          </p>
          <button
            onClick={handleConnect}
            disabled={busy}
            className="w-full px-4 py-2 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium disabled:opacity-50 flex items-center justify-center gap-2"
          >
            <CastIcon className="w-4 h-4" />
            {busy ? "Opening picker…" : "Choose a device"}
          </button>
        </div>
      );
      break;
    case "connecting":
      body = (
        <p className="text-sm text-gray-600 dark:text-gray-300">
          Connecting{device ? ` to ${device}` : ""}…
        </p>
      );
      break;
    case "connected":
      body = (
        <div className="space-y-3">
          <div className="flex items-center gap-3 p-3 rounded-md bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800">
            <span className="w-2.5 h-2.5 rounded-full bg-green-500" />
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium text-gray-900 dark:text-white truncate">
                Casting to {device || "device"}
              </p>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                Open a photo to send it to your TV.
              </p>
            </div>
          </div>
          <button
            onClick={handleDisconnect}
            disabled={busy}
            className="w-full px-4 py-2 rounded-md bg-red-600 hover:bg-red-700 text-white text-sm font-medium disabled:opacity-50"
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
        className="bg-white dark:bg-gray-800 rounded-xl shadow-2xl border border-gray-200 dark:border-gray-700 w-full max-w-sm mx-4"
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
            Cast
          </h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white text-xl leading-none px-1"
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

export { CastIcon };
