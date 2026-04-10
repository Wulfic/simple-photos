/** Global encryption-progress banner.
 *
 *  Polls the encrypted-sync API to count photos pending encryption
 *  (encrypted_blob_id IS NULL).  Shown across all pages via
 *  ProtectedLayout; dismissible with a close button.
 *
 *  Tracks progress relative to the *current batch* — when new items
 *  arrive the counter resets to the new pending count instead of
 *  showing totals against the entire library.
 *
 *  Displays a countdown timer estimating time remaining. */
import { useState, useEffect, useRef, useCallback } from "react";
import { api } from "../api/client";
import { hasCryptoKey } from "../crypto/crypto";
import { useProcessingStore } from "../store/processing";

/** Format seconds as HH:MM:SS, clamped to 0. */
function formatEta(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

export default function EncryptionBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ batchTotal: number; batchDone: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  // Batch tracking refs (survive across renders without causing re-renders)
  const batchSizeRef = useRef(0);
  const prevPendingRef = useRef(0);
  const batchStartRef = useRef(0);

  const poll = useCallback(async () => {
    try {
      // While conversion is active, suppress the encryption banner.
      // The ingest engine will trigger a final encryption pass after
      // all conversions complete — only then should this banner appear.
      try {
        const convStatus = await api.admin.conversionStatus();
        if (convStatus.active) {
          // Conversion in progress — hide encryption banner, reset state
          batchSizeRef.current = 0;
          prevPendingRef.current = 0;
          batchStartRef.current = 0;
          setCounts(null);
          setEta(null);
          endTask("encryption");
          return;
        }
      } catch {
        // Non-admin users won't have access — that's fine, proceed normally
      }

      type SyncRecord = Awaited<ReturnType<typeof api.photos.encryptedSync>>["photos"][number];
      const all: SyncRecord[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.encryptedSync({ after: cursor, limit: 500 });
        all.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      const pending = all.filter((p) => !p.encrypted_blob_id).length;

      // ── Batch tracking ────────────────────────────────────────────
      if (pending === 0) {
        // Nothing to encrypt — reset batch state
        batchSizeRef.current = 0;
        prevPendingRef.current = 0;
        batchStartRef.current = 0;
        setCounts(null);
        setEta(null);
        endTask("encryption");
      } else if (prevPendingRef.current === 0) {
        // New batch starting (was idle, now has pending items)
        batchSizeRef.current = pending;
        batchStartRef.current = Date.now();
        prevPendingRef.current = pending;
        setCounts({ batchTotal: pending, batchDone: 0 });
        setEta(null);
        startTask("encryption");
      } else {
        // Batch in progress
        if (pending > prevPendingRef.current) {
          // More items arrived mid-batch — expand the batch
          const added = pending - prevPendingRef.current;
          batchSizeRef.current += added;
        }
        prevPendingRef.current = pending;

        const batchDone = batchSizeRef.current - pending;
        setCounts({ batchTotal: batchSizeRef.current, batchDone });
        startTask("encryption");

        // ── ETA estimation ────────────────────────────────────────
        if (batchDone > 0 && batchStartRef.current > 0) {
          const elapsedMs = Date.now() - batchStartRef.current;
          const msPerItem = elapsedMs / batchDone;
          const remainingSec = Math.max(0, (pending * msPerItem) / 1000);
          setEta(formatEta(remainingSec));
        }
      }
    } catch {
      // Non-critical — will retry on next interval
    }
  }, [startTask, endTask]);

  useEffect(() => {
    if (dismissed) return;
    if (!hasCryptoKey()) return;

    // Initial check
    poll();

    // Poll every 2 s
    timerRef.current = setInterval(poll, 2_000);
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      endTask("encryption");
    };
  }, [dismissed, poll, endTask]);

  if (dismissed || !counts || counts.batchTotal === 0) return null;

  const pct = counts.batchTotal > 0 ? (counts.batchDone / counts.batchTotal) * 100 : 0;

  return (
    <div className="fixed bottom-6 left-4 right-4 z-50 pointer-events-none">
      <div className="pointer-events-auto max-w-md mx-auto flex items-center gap-3 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg px-4 py-3 shadow-lg">
        <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-blue-500 dark:border-t-blue-400 rounded-full animate-spin flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between">
            <p className="text-sm font-medium text-gray-700 dark:text-gray-200">
              Encrypting photos… {counts.batchDone}/{counts.batchTotal}
            </p>
            {eta && (
              <span className="text-xs tabular-nums text-gray-500 dark:text-gray-400 ml-2 flex-shrink-0">
                {eta} remaining
              </span>
            )}
          </div>
          <div className="mt-1.5 h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
            <div
              className="h-full bg-blue-500 dark:bg-blue-400 rounded-full transition-all duration-500"
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
