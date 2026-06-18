/** Street-level (precise) geo-resolution progress banner.
 *
 *  Companion to {@link GeoBanner}. That banner tracks the offline city/country
 *  pass; this one tracks the *opt-in* street-address pass that calls an external
 *  geocoder (Nominatim/Photon) at ~1 request/second. Because precise lookups are
 *  slow and network-bound, without a dedicated banner there was no way to tell
 *  whether they were running at all.
 *
 *  Polls `/api/status/activity` for `precise_progress`, which the server zeroes
 *  out unless the user enabled "Precise Street Addresses" — so this banner stays
 *  hidden for everyone else. Stacks directly above GeoBanner (`bottom-56`). */
import { useState, useEffect, useRef, useCallback } from "react";
import { request } from "../api/core";
import { useAuthStore } from "../store/auth";
import { useProcessingStore } from "../store/processing";
import { ProgressBanner } from "./ProgressBanner";

interface ActivityResponse {
  precise_progress?: {
    active: boolean;
    total: number;
    done: number;
    pending: number;
  };
}

/** Format seconds as HH:MM:SS, clamped to 0. */
function formatEta(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

export default function PreciseGeoBanner() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  // Track the pending queue across polls so we can render "this batch" progress
  // (0 → N) instead of an absolute count that would start mid-way for users who
  // already had some addresses resolved. Mirrors GeoBanner's approach.
  const batchSizeRef = useRef(0);
  const prevPendingRef = useRef(0);
  const batchStartRef = useRef(0);

  const poll = useCallback(async () => {
    try {
      const res = await request<ActivityResponse>("/status/activity");
      const precise = res.precise_progress;
      if (!precise) {
        endTask("geoPrecise");
        return;
      }
      const pending = Math.max(0, precise.pending);

      if (pending === 0) {
        batchSizeRef.current = 0;
        prevPendingRef.current = 0;
        batchStartRef.current = 0;
        setCounts(null);
        setEta(null);
        endTask("geoPrecise");
        return;
      }

      if (prevPendingRef.current === 0) {
        batchSizeRef.current = pending;
        batchStartRef.current = Date.now();
        prevPendingRef.current = pending;
        setCounts({ total: pending, done: 0 });
        setEta(null);
        startTask("geoPrecise");
        return;
      }

      // The queue can grow mid-batch as the coarse pass resolves more cities
      // (precise only becomes eligible once geo_city is filled in).
      if (pending > prevPendingRef.current) {
        const added = pending - prevPendingRef.current;
        batchSizeRef.current += added;
      }
      prevPendingRef.current = pending;

      const batchDone = batchSizeRef.current - pending;
      setCounts({ total: batchSizeRef.current, done: batchDone });
      startTask("geoPrecise");

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
      endTask("geoPrecise");
    };
  }, [isAuthenticated, poll, endTask]);

  if (dismissed) return null;
  if (!counts || counts.total === 0) return null;

  const pct = counts.total > 0 ? (counts.done / counts.total) * 100 : 0;

  return (
    <ProgressBanner
      id="geoPrecise"
      tone="purple"
      label={`Resolving street addresses… ${counts.done}/${counts.total}`}
      description="Looking up precise locations via OpenStreetMap (~1/sec)."
      eta={eta}
      pct={pct}
      onDismiss={() => setDismissed(true)}
    />
  );
}
