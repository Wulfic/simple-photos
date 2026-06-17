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
import { ProgressBanner } from "./ProgressBanner";

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

      const pending = all.filter((p) => p.encrypted_blob_id === null || p.encrypted_blob_id === undefined).length;

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
    if (!hasCryptoKey()) return;

    // Initial check
    poll();

    // Poll every 2 s — continues even when banner is dismissed so the
    // profile icon keeps spinning until the server finishes.
    timerRef.current = setInterval(poll, 2_000);
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      endTask("encryption");
    };
  }, [poll, endTask]);

  if (dismissed || !counts || counts.batchTotal === 0) return null;

  const pct = counts.batchTotal > 0 ? (counts.batchDone / counts.batchTotal) * 100 : 0;

  return (
    <ProgressBanner
      position="bottom-6"
      tone="accent"
      label={`Encrypting photos… ${counts.batchDone}/${counts.batchTotal}`}
      eta={eta}
      pct={pct}
      onDismiss={() => setDismissed(true)}
    />
  );
}
