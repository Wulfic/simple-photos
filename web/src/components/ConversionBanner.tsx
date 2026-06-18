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
import { ProgressBanner } from "./ProgressBanner";

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

      setCounts({ total: res.total, done: Math.min(res.done, res.total) });
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
    poll();
    // Poll continues even when banner is dismissed so the profile icon
    // keeps spinning until the server finishes converting.
    timerRef.current = setInterval(poll, 2_000);
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      endTask("conversion");
    };
  }, [poll, endTask]);

  if (dismissed || !counts || counts.total === 0) return null;

  const pct = counts.total > 0 ? (counts.done / counts.total) * 100 : 0;

  return (
    <ProgressBanner
      id="conversion"
      tone="orange"
      label={`Converting media… ${counts.done}/${counts.total}`}
      eta={eta}
      pct={pct}
      onDismiss={() => setDismissed(true)}
    />
  );
}
