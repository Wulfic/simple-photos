/**
 * Persistent progress banners for long-running background tasks
 * (encryption migration, file conversion, missing thumbnails).
 *
 * Uses a "minimum display" pattern: once a banner appears, it stays
 * visible for at least 8 seconds even if the task finishes sooner,
 * showing a "done" state to prevent jarring flash-and-vanish behavior.
 */
import { useState, useEffect, useRef } from "react";
import { useActivityStore } from "../store/activity";

// ── Minimum-visibility hook ──────────────────────────────────────────────────
// Once a banner appears, it stays for at least `minMs` before hiding.
// Returns [visible, completed] where `completed` means the work finished but
// the minimum display time hasn't elapsed yet (show a "done" state).
function useMinimumDisplay(
  active: boolean,
  minMs: number = 8000,
): [boolean, boolean, () => void] {
  const [visible, setVisible] = useState(false);
  const showSinceRef = useRef<number | null>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Tracks whether the user manually dismissed this banner.
  const isDismissedRef = useRef(false);
  // Tracks previous active value so we can detect genuine false→true transitions.
  const prevActiveRef = useRef(false);

  const forceHide = () => {
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
    isDismissedRef.current = true;
    setVisible(false);
    showSinceRef.current = null;
  };

  useEffect(() => {
    const wasActive = prevActiveRef.current;
    prevActiveRef.current = active;

    if (active) {
      // Only reset the dismissed state on a genuine new job start (false→true).
      // This prevents brief polling flickers (true→false→true) from resurfacing
      // a banner the user already dismissed.
      if (!wasActive) {
        isDismissedRef.current = false;
      }
      if (isDismissedRef.current) return;
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
        if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
        setVisible(false);
        showSinceRef.current = null;
      } else {
        hideTimerRef.current = setTimeout(() => {
          hideTimerRef.current = null;
          setVisible(false);
          showSinceRef.current = null;
        }, remaining);
      }
    }
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, [active, minMs, visible]);

  return [visible, visible && !active, forceHide];
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

function useETA(pendingCount: number): string {
  const [etaStr, setEtaStr] = useState("");
  const lastCountRef = useRef(pendingCount);
  const lastTimeRef = useRef(Date.now());
  const velocityEmaRef = useRef<number | null>(null);
  const samplesRef = useRef(0);

  useEffect(() => {
    const now = Date.now();
    const dt = (now - lastTimeRef.current) / 1000;
    
    if (pendingCount < lastCountRef.current && dt > 0.5) {
      const delta = lastCountRef.current - pendingCount;
      const currentVelocity = delta / dt;
      if (velocityEmaRef.current === null) {
        velocityEmaRef.current = currentVelocity;
      } else {
        velocityEmaRef.current = velocityEmaRef.current * 0.7 + currentVelocity * 0.3;
      }
      samplesRef.current += 1;
    } else if (pendingCount > lastCountRef.current) {
      // Queue grew (new scan, etc.) — reset velocity calculation
      velocityEmaRef.current = null;
      samplesRef.current = 0;
    }
    
    lastCountRef.current = pendingCount;
    lastTimeRef.current = now;
    
    // Require at least 2 velocity samples and >3 pending items before showing ETA
    if (pendingCount <= 3 || velocityEmaRef.current === null || velocityEmaRef.current <= 0 || samplesRef.current < 2) {
      setEtaStr("");
      return;
    }
    
    const etaSeconds = Math.round(pendingCount / velocityEmaRef.current);
    if (etaSeconds < 5) {
      setEtaStr(""); // Don't show very short ETAs — they flicker
    } else if (etaSeconds < 60) {
      setEtaStr(`~${etaSeconds}s`);
    } else {
      const minutes = Math.floor(etaSeconds / 60);
      setEtaStr(`~${minutes}m`);
    }
  }, [pendingCount]);
  
  return etaStr;
}

export default function GlobalProgressBanners() {

  const {
    conversionPending,
    conversionAwaitingKey,
    conversionMissingThumbs,
    conversionActive,
    encryptionMode,
    migrationStatus,
    migrationTotal,
    migrationCompleted,
  } = useActivityStore();

  // Raw activity flags from the server
  const migrationActive =
    migrationStatus === "encrypting" || migrationStatus === "decrypting";

  // Only show conversion banner when there's ACTIONABLE work (items the
  // converter can actually process right now). Items "awaiting key" are not
  // actionable and shouldn't drive the banner or ETA timer — that was
  // confusing users by showing conversion progress that was actually
  // tracking encryption speed.
  const conversionBusy =
    conversionPending > 0 || conversionMissingThumbs > 0 || conversionActive;
  const migrationBusy = migrationActive && migrationTotal > 0;

  // Apply minimum-display behavior
  const [showConversion, conversionDone, hideConversion] = useMinimumDisplay(conversionBusy);
  const [showMigration, migrationDone, hideMigration] = useMinimumDisplay(migrationBusy);

  const totalConversionItems = conversionPending + conversionMissingThumbs;
  const conversionEta = useETA(totalConversionItems);

  // ── Stuck detection: if pending count hasn't decreased for ~30s ────────
  const stallCountRef = useRef(0);
  const prevPendingRef = useRef(totalConversionItems);
  const [conversionStalled, setConversionStalled] = useState(false);

  useEffect(() => {
    if (totalConversionItems === 0) {
      stallCountRef.current = 0;
      setConversionStalled(false);
    } else if (totalConversionItems >= prevPendingRef.current && conversionBusy) {
      stallCountRef.current += 1;
      // 10 polls × 3s interval = ~30 seconds with no progress
      if (stallCountRef.current >= 10) setConversionStalled(true);
    } else {
      stallCountRef.current = 0;
      setConversionStalled(false);
    }
    prevPendingRef.current = totalConversionItems;
  }, [totalConversionItems, conversionBusy]);

  // ── Diagnostic logging ────────────────────────────────────────────────
  const prevDiagRef = useRef("");
  const diagKey = `cb=${conversionBusy}|ma=${migrationActive}|sc=${showConversion}|cd=${conversionDone}|mb=${migrationBusy}|sm=${showMigration}|md=${migrationDone}|cp=${conversionPending}|cak=${conversionAwaitingKey}|ct=${conversionMissingThumbs}|ca=${conversionActive}`;
  if (diagKey !== prevDiagRef.current) {
    console.log(`[DIAG:BANNER] prev=${prevDiagRef.current} -> new=${diagKey}`);
    prevDiagRef.current = diagKey;
  }

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
        <div className={`pointer-events-auto rounded-xl shadow-lg shadow-black/10 dark:shadow-black/30 p-3 transition-colors duration-500 relative ${
          conversionDone
            ? "bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-700"
            : "bg-white dark:bg-gray-800 border border-amber-200 dark:border-amber-700"
        }`}>
          <button
            onClick={hideConversion}
            className="absolute top-2 right-2 w-6 h-6 z-10 flex items-center justify-center rounded-full text-gray-500 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-100 hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors bg-white/50 dark:bg-black/50 backdrop-blur-sm"
            title="Dismiss"
          >
            <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
          <div className="flex items-center gap-2.5 pr-5">
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
                <div className="flex flex-col gap-0.5">
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
                  {conversionEta && (
                    <p className="text-xs font-mono text-amber-500/90 dark:text-amber-300/90">
                      {conversionEta}
                    </p>
                  )}
                  {conversionStalled && (
                    <p className="text-xs text-red-500 dark:text-red-400 mt-0.5">
                      ⚠ Conversion may be stuck — some files might not be convertible
                    </p>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ── Migration card ──────────────────────────────────────────── */}
      {showMigration && (
        <div className={`pointer-events-auto rounded-xl shadow-lg shadow-black/10 dark:shadow-black/30 p-3 transition-colors duration-500 relative ${
          migrationDone
            ? "bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-700"
            : "bg-white dark:bg-gray-800 border border-blue-200 dark:border-blue-700"
        }`}>
          <button
            onClick={hideMigration}
            className="absolute top-2 right-2 w-6 h-6 z-10 flex items-center justify-center rounded-full text-gray-500 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-100 hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors bg-white/50 dark:bg-black/50 backdrop-blur-sm"
            title="Dismiss"
          >
            <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
          {migrationDone ? (
            <div className="flex items-center gap-2.5 pr-5">
              <svg className="w-4 h-4 text-green-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              <p className="text-sm font-medium text-green-700 dark:text-green-300">
                {encryptionMode === "encrypted" ? "Encryption" : "Decryption"} complete
              </p>
            </div>
          ) : (
            <>
              <div className="flex items-center gap-2.5 mb-2 pr-5">
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
