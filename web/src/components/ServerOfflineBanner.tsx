/**
 * ServerOfflineBanner — fixed top-of-screen banner that appears when the
 * server becomes unreachable at runtime.
 *
 * Lifecycle:
 *  - Hidden while online and no recovery flash.
 *  - Shows a red bar while offline with elapsed time + countdown to next retry.
 *  - Briefly shows a green "Reconnected" bar when the server comes back.
 *
 * Mounted at the App root (outside ProtectedLayout) so it shows on every
 * page including login and the welcome wizard.
 */
import { useEffect, useRef, useState } from "react";
import { useServerHealth } from "../hooks/useServerHealth";

/** Formats elapsed seconds as "Xm Ys" or "Xs". */
function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

export default function ServerOfflineBanner() {
  const { isOnline, justRecovered, secondsUntilRetry, retryCount } = useServerHealth();
  const [elapsedSec, setElapsedSec] = useState(0);
  const offlineSinceRef = useRef<number | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Track elapsed-time-offline counter.
  useEffect(() => {
    if (!isOnline) {
      if (offlineSinceRef.current === null) {
        offlineSinceRef.current = Date.now();
        setElapsedSec(0);
      }
      elapsedTimerRef.current = setInterval(() => {
        setElapsedSec(Math.floor((Date.now() - (offlineSinceRef.current ?? Date.now())) / 1000));
      }, 1000);
    } else {
      offlineSinceRef.current = null;
      setElapsedSec(0);
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    }
    return () => {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    };
  }, [isOnline]);

  // Nothing to show.
  if (isOnline && !justRecovered) return null;

  if (justRecovered) {
    return (
      <div
        role="status"
        aria-live="polite"
        className="fixed top-0 inset-x-0 z-[9999] flex items-center justify-center gap-2 px-4 py-1.5 text-sm font-medium bg-green-600 text-white shadow-md"
      >
        <span aria-hidden>✓</span>
        <span>Reconnected</span>
      </div>
    );
  }

  // Offline banner.
  const checking  = secondsUntilRetry === null || secondsUntilRetry <= 0;
  const statusMsg = checking
    ? "Reconnecting…"
    : `Reconnecting in ${secondsUntilRetry}s`;

  return (
    <div
      role="alert"
      aria-live="assertive"
      className="fixed top-0 inset-x-0 z-[9999] flex items-center gap-2 px-4 py-1.5 text-sm bg-red-700 text-white shadow-md"
    >
      {/* Spinner */}
      <span
        className="inline-block w-3.5 h-3.5 rounded-full border-2 border-white border-t-transparent animate-spin flex-shrink-0"
        aria-hidden
      />

      <span className="font-medium">Server offline</span>

      {elapsedSec > 0 && (
        <span className="text-red-200 text-xs">
          — down {formatElapsed(elapsedSec)}
        </span>
      )}

      {/* Divider */}
      <span className="ml-auto text-xs text-red-200 whitespace-nowrap">
        {statusMsg}
        {retryCount > 1 && (
          <span className="ml-1 opacity-70">
            (attempt {retryCount})
          </span>
        )}
      </span>
    </div>
  );
}
