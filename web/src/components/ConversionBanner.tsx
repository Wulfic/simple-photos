/** Global conversion-progress banner.
 *
 *  Polls the conversion-status endpoint to track files being converted
 *  from non-native formats (HEIC, MKV, TIFF, etc.) to browser-native
 *  equivalents (JPEG, MP4, MP3).
 *
 *  Shown across all pages via ProtectedLayout; dismissible with a close
 *  button.  Displays a progress bar and countdown timer (same pattern
 *  as EncryptionBanner). */
import { useState, useEffect, useRef, useCallback } from "react";
import { api } from "../api/client";
import { useProcessingStore } from "../store/processing";

/** Format seconds as HH:MM:SS, clamped to 0. */
function formatEta(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

export default function ConversionBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  const batchStartRef = useRef(0);
  const prevDoneRef = useRef(0);

  const poll = useCallback(async () => {
    try {
      const res = await api.admin.conversionStatus();

      if (!res.active || res.total === 0) {
        setCounts(null);
        setEta(null);
        endTask("conversion");
        batchStartRef.current = 0;
        prevDoneRef.current = 0;
        return;
      }

      // New batch detected
      if (batchStartRef.current === 0 && res.done === 0) {
        batchStartRef.current = Date.now();
      }

      setCounts({ total: res.total, done: res.done });
      startTask("conversion");

      // ETA estimation
      if (res.done > 0 && batchStartRef.current > 0) {
        const elapsedMs = Date.now() - batchStartRef.current;
        const msPerItem = elapsedMs / res.done;
        const remaining = res.total - res.done;
        const remainingSec = Math.max(0, (remaining * msPerItem) / 1000);
        setEta(formatEta(remainingSec));
      }

      prevDoneRef.current = res.done;
    } catch {
      // Non-critical — will retry on next interval
    }
  }, [startTask, endTask]);

  useEffect(() => {
    if (dismissed) return;

    poll();
    timerRef.current = setInterval(poll, 2_000);
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      endTask("conversion");
    };
  }, [dismissed, poll, endTask]);

  if (dismissed || !counts || counts.total === 0) return null;

  const pct = counts.total > 0 ? (counts.done / counts.total) * 100 : 0;

  return (
    <div className="fixed bottom-20 left-4 right-4 z-50 pointer-events-none">
      <div className="pointer-events-auto max-w-md mx-auto flex items-center gap-3 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg px-4 py-3 shadow-lg">
        <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-orange-500 dark:border-t-orange-400 rounded-full animate-spin flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between">
            <p className="text-sm font-medium text-gray-700 dark:text-gray-200">
              Converting media… {counts.done}/{counts.total}
            </p>
            {eta && (
              <span className="text-xs tabular-nums text-gray-500 dark:text-gray-400 ml-2 flex-shrink-0">
                {eta} remaining
              </span>
            )}
          </div>
          <div className="mt-1.5 h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
            <div
              className="h-full bg-orange-500 dark:bg-orange-400 rounded-full transition-all duration-500"
              style={{ width: `${pct}%` }}
            />
          </div>
        </div>
        <button
          onClick={() => setDismissed(true)}
          className="p-1 text-gray-400 hover:text-gray-600 dark:text-gray-500 dark:hover:text-gray-300 transition-colors flex-shrink-0"
          aria-label="Dismiss"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
}
