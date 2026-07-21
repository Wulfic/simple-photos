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

/** Show the manual "Reset" affordance once a conversion has made zero progress
 *  for this long. Far shorter than the server watchdog's 2h auto-recovery so an
 *  operator watching the banner can unstick it immediately (#18). */
const STALL_HINT_MS = 3 * 60 * 1000;

export default function ConversionBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const [stalled, setStalled] = useState(false);
  const [resetting, setResetting] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // Track when `done` last advanced so we can detect a wedged pass client-side
  // without trusting cross-machine clocks.
  const progressRef = useRef<{ done: number; at: number }>({ done: -1, at: Date.now() });
  const { startTask, endTask } = useProcessingStore();

  const poll = useCallback(async () => {
    try {
      const res = await api.admin.conversionStatus();

      if (!res.active || res.total === 0) {
        setCounts(null);
        setEta(null);
        setStalled(false);
        progressRef.current = { done: -1, at: Date.now() };
        endTask("conversion");
        return;
      }

      const done = Math.min(res.done, res.total);
      setCounts({ total: res.total, done });
      startTask("conversion");

      // Client-side stall detection: if `done` hasn't advanced in STALL_HINT_MS
      // while still active, surface the manual reset button.
      if (done !== progressRef.current.done) {
        progressRef.current = { done, at: Date.now() };
        setStalled(false);
      } else if (Date.now() - progressRef.current.at > STALL_HINT_MS) {
        setStalled(true);
      }

      // ETA is server-authoritative (item #4), so there is no client-side clock
      // to drift. Since #40 it is *not* the encryption banner's estimator: the
      // conversion queue deliberately mixes categories whose per-item costs
      // differ by orders of magnitude, so it uses the work-weighted,
      // per-category one instead (server `progress::ConversionEta`).
      setEta(res.eta_seconds != null ? formatEta(res.eta_seconds) : null);
    } catch {
      // Non-critical — will retry on next interval
    }
  }, [startTask, endTask]);

  const handleReset = useCallback(async () => {
    setResetting(true);
    try {
      await api.admin.conversionReset();
      setStalled(false);
      progressRef.current = { done: -1, at: Date.now() };
      await poll();
    } catch {
      // Best-effort — the server watchdog remains the backstop.
    } finally {
      setResetting(false);
    }
  }, [poll]);

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
      description={stalled ? "This conversion looks stuck." : undefined}
      eta={eta}
      pct={pct}
      action={
        stalled
          ? {
              label: "Reset stuck conversion",
              busyLabel: "Resetting…",
              busy: resetting,
              onClick: handleReset,
            }
          : undefined
      }
      onDismiss={() => setDismissed(true)}
    />
  );
}
