import { useState, useEffect, useRef } from "react";
import { useActivityStore } from "../store/activity";

// ── Minimum-visibility hook ──────────────────────────────────────────────────
// Once a banner appears, it stays for at least `minMs` before hiding.
// Returns [visible, completed] where `completed` means the work finished but
// the minimum display time hasn't elapsed yet (show a "done" state).
function useMinimumDisplay(
  active: boolean,
  minMs: number = 8000,
): [boolean, boolean] {
  const [visible, setVisible] = useState(false);
  const showSinceRef = useRef<number | null>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (active) {
      // Work started — show immediately, record start time
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
      if (!showSinceRef.current) showSinceRef.current = Date.now();
      setVisible(true);
    } else if (visible && showSinceRef.current) {
      // Work finished — compute how long banner has been showing
      const elapsed = Date.now() - showSinceRef.current;
      const remaining = Math.max(0, minMs - elapsed);
      if (remaining === 0) {
        // Minimum time already met — hide now
        setVisible(false);
        showSinceRef.current = null;
      } else {
        // Keep visible for the remaining minimum time
        hideTimerRef.current = setTimeout(() => {
          setVisible(false);
          showSinceRef.current = null;
          hideTimerRef.current = null;
        }, remaining);
      }
    }
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, [active]); // eslint-disable-line react-hooks/exhaustive-deps

  return [visible, visible && !active];
}

/**
 * Fixed-position floating progress banners (bottom-right corner).
 *
 * Uses `fixed` positioning so they never affect page layout or push the nav bar
 * down. Styled as compact floating cards similar to download/copy-progress
 * indicators in desktop apps.
 *
 * Uses minimum-display timing: once a banner appears it stays for at least 8s.
 * When work finishes early, the banner shows a “Complete” state before hiding.
 */
export default function GlobalProgressBanners() {
  const {
    conversionPending,
    conversionMissingThumbs,
    conversionActive,
    migrationStatus,
    migrationTotal,
    migrationCompleted,
  } = useActivityStore();

  // Raw activity flags from the server
  const conversionBusy = conversionPending > 0 || conversionMissingThumbs > 0 || conversionActive;
  const migrationBusy =
    (migrationStatus === "encrypting" || migrationStatus === "decrypting") &&
    migrationTotal > 0;

  // Apply minimum-display behavior
  const [showConversion, conversionDone] = useMinimumDisplay(conversionBusy);
  const [showMigration, migrationDone] = useMinimumDisplay(migrationBusy);

  if (!showConversion && !showMigration) return null;

  const migPct =
    migrationTotal > 0
      ? Math.min(Math.round((migrationCompleted / migrationTotal) * 100), 100)
      : 0;
  const migAction =
    migrationStatus === "encrypting" ? "Encrypting" : "Decrypting";

  return (
    <div className="fixed bottom-4 right-4 z-[60] flex flex-col gap-2 w-80 pointer-events-none">
      {/* ── Conversion card ─────────────────────────────────────────── */}
      {showConversion && (
        <div className={`pointer-events-auto rounded-xl shadow-lg shadow-black/10 dark:shadow-black/30 p-3 transition-colors duration-500 ${
          conversionDone
            ? "bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-700"
            : "bg-white dark:bg-gray-800 border border-amber-200 dark:border-amber-700"
        }`}>
          <div className="flex items-center gap-2.5">
            {conversionDone ? (
              <svg className="w-4 h-4 text-green-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
              </svg>
            ) : (
              <div className="w-4 h-4 border-2 border-amber-500 border-t-transparent rounded-full animate-spin flex-shrink-0" />
            )}
            <div className="min-w-0">
              <p className={`text-sm font-medium truncate ${
                conversionDone
                  ? "text-green-700 dark:text-green-300"
                  : "text-amber-800 dark:text-amber-300"
              }`}>
                {conversionDone ? "Conversion complete" : "Converting media…"}
              </p>
              {!conversionDone && (
                <p className="text-xs text-amber-600/80 dark:text-amber-400/80">
                  {conversionPending > 0 || conversionMissingThumbs > 0
                    ? [
                        conversionPending > 0
                          ? `${conversionPending} file${conversionPending !== 1 ? "s" : ""} pending`
                          : "",
                        conversionMissingThumbs > 0
                          ? `${conversionMissingThumbs} thumbnail${conversionMissingThumbs !== 1 ? "s" : ""}`
                          : "",
                      ]
                        .filter(Boolean)
                        .join(", ")
                    : "Processing in background…"}
                </p>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ── Migration card ──────────────────────────────────────────── */}
      {showMigration && (
        <div className={`pointer-events-auto rounded-xl shadow-lg shadow-black/10 dark:shadow-black/30 p-3 transition-colors duration-500 ${
          migrationDone
            ? "bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-700"
            : "bg-white dark:bg-gray-800 border border-blue-200 dark:border-blue-700"
        }`}>
          {migrationDone ? (
            <div className="flex items-center gap-2.5">
              <svg className="w-4 h-4 text-green-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              <p className="text-sm font-medium text-green-700 dark:text-green-300">
                {migrationStatus === "encrypting" ? "Encryption" : "Decryption"} complete
              </p>
            </div>
          ) : (
            <>
              <div className="flex items-center gap-2.5 mb-2">
                <div className="w-4 h-4 border-2 border-blue-500 border-t-transparent rounded-full animate-spin flex-shrink-0" />
                <p className="text-sm font-medium text-blue-800 dark:text-blue-300">
                  {migAction}… {migrationCompleted}/{migrationTotal}
                </p>
                <span className="ml-auto text-xs font-medium text-blue-600 dark:text-blue-400">
                  {migPct}%
                </span>
              </div>
              <div className="w-full bg-blue-100 dark:bg-blue-900/50 rounded-full h-1.5">
                <div
                  className="bg-blue-500 h-1.5 rounded-full transition-all duration-300"
                  style={{ width: `${migPct}%` }}
                />
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
