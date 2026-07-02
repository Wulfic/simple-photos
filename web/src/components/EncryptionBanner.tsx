/** Global encryption-progress banner.
 *
 *  Reads the **server-authoritative** `/status/encryption` endpoint (item #1)
 *  instead of paginating the whole library client-side. The server owns the
 *  batch/ETA state machine and folds in every client's contributed upload
 *  count, so web and Android render identical totals and ETA.
 *
 *  Shown across all pages via ProtectedLayout; dismissible with a close button.
 *  Suppressed while media conversion is active so the user sees a single
 *  banner at a time (the ingest engine runs a final encryption pass after all
 *  conversions complete). */
import { useState, useEffect, useRef, useCallback } from "react";
import { api } from "../api/client";
import { hasCryptoKey } from "../crypto/crypto";
import { useProcessingStore } from "../store/processing";
import { ProgressBanner } from "./ProgressBanner";
import { formatEta } from "../utils/formatters";

export default function EncryptionBanner() {
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  const poll = useCallback(async () => {
    try {
      // While conversion is active, suppress the encryption banner. The ingest
      // engine triggers a final encryption pass once all conversions complete —
      // only then should this banner appear.
      try {
        const convStatus = await api.admin.conversionStatus();
        if (convStatus.active) {
          setCounts(null);
          setEta(null);
          endTask("encryption");
          return;
        }
      } catch {
        // Non-admin users won't have access — that's fine, proceed normally.
      }

      const status = await api.encryption.status();

      if (!status.active || status.total === 0) {
        setCounts(null);
        setEta(null);
        endTask("encryption");
        return;
      }

      setCounts({ total: status.total, done: status.done });
      setEta(status.eta_seconds != null ? formatEta(status.eta_seconds) : null);
      startTask("encryption");
    } catch {
      // Non-critical — will retry on next interval.
    }
  }, [startTask, endTask]);

  useEffect(() => {
    if (!hasCryptoKey()) return;

    // Initial check
    poll();

    // Poll every 2 s — continues even when banner is dismissed so the profile
    // icon keeps spinning until the server finishes.
    timerRef.current = setInterval(poll, 2_000);
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      endTask("encryption");
    };
  }, [poll, endTask]);

  if (dismissed || !counts || counts.total === 0) return null;

  const pct = counts.total > 0 ? (counts.done / counts.total) * 100 : 0;

  return (
    <ProgressBanner
      id="encryption"
      tone="accent"
      label={`Encrypting photos… ${counts.done}/${counts.total}`}
      eta={eta}
      pct={pct}
      onDismiss={() => setDismissed(true)}
    />
  );
}
