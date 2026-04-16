/**
 * Hook that tracks which blob IDs are inside secure galleries.
 *
 * Polls the server every 5 s so photos moved to / from secure
 * galleries on other devices are reflected without a manual reload.
 */
import { useEffect, useRef, useState } from "react";
import { api } from "../../api/client";

export interface SecureBlobFilterResult {
  /** Set of blob IDs currently inside a secure gallery. */
  secureBlobIds: Set<string>;
  /** One-shot fetch; used during init. */
  refreshSecureBlobIds: () => Promise<void>;
  /** Start the 5 s polling interval. */
  startPolling: () => void;
}

export function useSecureBlobFilter(): SecureBlobFilterResult {
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  async function refreshSecureBlobIds() {
    try {
      const res = await api.secureGalleries.secureBlobIds();
      setSecureBlobIds(new Set(res.blob_ids));
    } catch {
      // Secure galleries may not be available — ignore
    }
  }

  function startPolling() {
    if (pollRef.current) return;
    pollRef.current = setInterval(async () => {
      try {
        const res = await api.secureGalleries.secureBlobIds();
        const fresh = new Set(res.blob_ids);
        setSecureBlobIds((prev) => {
          if (prev.size !== fresh.size) return fresh;
          for (const id of fresh) {
            if (!prev.has(id)) return fresh;
          }
          return prev;
        });
      } catch {
        // Non-critical — ignore transient failures
      }
    }, 5_000);
  }

  useEffect(() => {
    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, []);

  return { secureBlobIds, refreshSecureBlobIds, startPolling };
}
