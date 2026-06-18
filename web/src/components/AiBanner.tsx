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
import { ProgressBanner } from "./ProgressBanner";

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
    <ProgressBanner
      id="ai"
      tone="purple"
      label={`AI processing… ${counts.done}/${counts.total}`}
      eta={eta}
      pct={pct}
      onDismiss={() => setDismissed(true)}
    />
  );
}
