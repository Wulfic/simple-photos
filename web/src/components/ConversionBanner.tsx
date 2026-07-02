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
import { formatEta } from "../utils/formatters";

export default function ConversionBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  const poll = useCallback(async () => {
    try {
      const res = await api.admin.conversionStatus();

      if (!res.active || res.total === 0) {
        setCounts(null);
        setEta(null);
        endTask("conversion");
        return;
      }

      setCounts({ total: res.total, done: Math.min(res.done, res.total) });
      startTask("conversion");

      // ETA is now server-authoritative (item #4) — same throughput estimator
      // as the encryption banner, so no client-side clock to drift.
      setEta(res.eta_seconds != null ? formatEta(res.eta_seconds) : null);
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
