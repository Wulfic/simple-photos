/** Global encryption-progress banner.
 *
 *  Reads from IndexedDB (via Dexie live query) to count photos that are still
 *  pending server-side encryption (`serverSide === true`). Shown across all
 *  pages via ProtectedLayout; dismissible with a close button. */
import { useState } from "react";
import { useLiveQuery } from "dexie-react-hooks";
import { db } from "../db";

export default function EncryptionBanner() {
  const [dismissed, setDismissed] = useState(false);

  const counts = useLiveQuery(async () => {
    const all = await db.photos.count();
    const pending = await db.photos.filter((p) => !!p.serverSide).count();
    return { total: all, pending, encrypted: all - pending };
  });

  if (dismissed || !counts || counts.pending === 0) return null;

  const pct = counts.total > 0 ? (counts.encrypted / counts.total) * 100 : 0;

  return (
    <div className="sticky top-0 z-40 px-4 pt-2">
      <div className="flex items-center gap-3 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg px-4 py-3">
        <div className="w-5 h-5 border-2 border-amber-400 border-t-amber-600 rounded-full animate-spin flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-amber-800 dark:text-amber-200">
            Encrypting photos… {counts.encrypted}/{counts.total}
          </p>
          <div className="mt-1.5 h-1.5 bg-amber-200 dark:bg-amber-900 rounded-full overflow-hidden">
            <div
              className="h-full bg-amber-500 dark:bg-amber-400 rounded-full transition-all duration-500"
              style={{ width: `${pct}%` }}
            />
          </div>
        </div>
        <button
          onClick={() => setDismissed(true)}
          className="p-1 text-amber-500 hover:text-amber-700 dark:text-amber-400 dark:hover:text-amber-200 transition-colors flex-shrink-0"
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
