/** Global AI-processing progress banner.
 *
 *  Polls `/api/status/activity` for the authenticated user's AI work
 *  (face / object detection, clustering).  Same UX pattern as the
 *  encryption and conversion banners — a dismissible card with a spinner,
 *  a `done/total` counter, an ETA, and a progress bar.
 *
 *  Drives `useProcessingStore` so the profile-avatar spinner in the
 *  nav bar reflects whether the server is actively working on this user's
 *  AI backlog. */
import { useState, useEffect, useRef, useCallback } from "react";
import { request } from "../api/core";
import { useAuthStore } from "../store/auth";
import { useProcessingStore } from "../store/processing";

interface ActivityResponse {
  ai_progress?: { active: boolean; total: number; done: number; pending: number };
}

/** Format seconds as HH:MM:SS, clamped to 0. */
function formatEta(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

export default function AiBanner() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  // Batch tracking (mirrors EncryptionBanner).
  const batchSizeRef = useRef(0);
  const prevPendingRef = useRef(0);
  const batchStartRef = useRef(0);

  const poll = useCallback(async () => {
    try {
      const res = await request<ActivityResponse>("/status/activity");
      const ai = res.ai_progress;
      if (!ai) {
        endTask("ai");
        return;
      }
      const pending = Math.max(0, ai.pending);

      if (pending === 0) {
        // Idle — clear banner and stop the spinner immediately.
        batchSizeRef.current = 0;
        prevPendingRef.current = 0;
        batchStartRef.current = 0;
        setCounts(null);
        setEta(null);
        endTask("ai");
        return;
      }

      if (prevPendingRef.current === 0) {
        batchSizeRef.current = pending;
        batchStartRef.current = Date.now();
        prevPendingRef.current = pending;
        setCounts({ total: pending, done: 0 });
        setEta(null);
        startTask("ai");
        return;
      }

      if (pending > prevPendingRef.current) {
        const added = pending - prevPendingRef.current;
        batchSizeRef.current += added;
      }
      prevPendingRef.current = pending;

      const batchDone = batchSizeRef.current - pending;
      setCounts({ total: batchSizeRef.current, done: batchDone });
      startTask("ai");

      if (batchDone > 0 && batchStartRef.current > 0) {
        const elapsedMs = Date.now() - batchStartRef.current;
        const msPerItem = elapsedMs / batchDone;
        const remainingSec = Math.max(0, (pending * msPerItem) / 1000);
        setEta(formatEta(remainingSec));
      }
    } catch {
      // Non-critical — retry on next interval.
    }
  }, [startTask, endTask]);

  useEffect(() => {
    if (!isAuthenticated) return;
    poll();
    timerRef.current = setInterval(poll, 3_000);
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      endTask("ai");
    };
  }, [isAuthenticated, poll, endTask]);

  if (dismissed || !counts || counts.total === 0) return null;

  const pct = counts.total > 0 ? (counts.done / counts.total) * 100 : 0;

  return (
    <div className="fixed bottom-32 left-4 right-4 z-50 pointer-events-none">
      <div className="pointer-events-auto max-w-md mx-auto flex items-center gap-3 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg px-4 py-3 shadow-lg">
        <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-purple-500 dark:border-t-purple-400 rounded-full animate-spin flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between">
            <p className="text-sm font-medium text-gray-700 dark:text-gray-200">
              AI processing… {counts.done}/{counts.total}
            </p>
            {eta && (
              <span className="text-xs tabular-nums text-gray-700 dark:text-gray-400 ml-2 flex-shrink-0">
                {eta} remaining
              </span>
            )}
          </div>
          <div className="mt-1.5 h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
            <div
              className="h-full bg-purple-500 dark:bg-purple-400 rounded-full transition-all duration-500"
              style={{ width: `${pct}%` }}
            />
          </div>
        </div>
        <button
          onClick={() => setDismissed(true)}
          className="p-1 text-gray-600 hover:text-gray-600 dark:text-gray-500 dark:hover:text-gray-300 transition-colors flex-shrink-0"
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
