/** Global encryption-progress banner.
 *
 *  Polls the encrypted-sync API to count photos pending encryption
 *  (encrypted_blob_id IS NULL).  Shown across all pages via
 *  ProtectedLayout; dismissible with a close button. */
import { useState, useEffect, useRef, useCallback } from "react";
import { api } from "../api/client";
import { hasCryptoKey } from "../crypto/crypto";

export default function EncryptionBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; pending: number; encrypted: number } | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const poll = useCallback(async () => {
    try {
      type SyncRecord = Awaited<ReturnType<typeof api.photos.encryptedSync>>["photos"][number];
      const all: SyncRecord[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.encryptedSync({ after: cursor, limit: 500 });
        all.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      const pending = all.filter((p) => !p.encrypted_blob_id);
      const encrypted = all.length - pending.length;
      setCounts({ total: all.length, pending: pending.length, encrypted });
    } catch {
      // Non-critical — will retry on next interval
    }
  }, []);

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
    };
  }, [dismissed, poll]);

  if (dismissed || !counts || counts.pending === 0) return null;

  const pct = counts.total > 0 ? (counts.encrypted / counts.total) * 100 : 0;

  return (
    <div className="fixed bottom-6 left-4 right-4 z-50 pointer-events-none">
      <div className="pointer-events-auto max-w-md mx-auto flex items-center gap-3 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg px-4 py-3 shadow-lg">
        <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-blue-500 dark:border-t-blue-400 rounded-full animate-spin flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-gray-700 dark:text-gray-200">
            Encrypting photos… {counts.encrypted}/{counts.total}
          </p>
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
