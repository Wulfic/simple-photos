/** Global geo-resolution progress banner.
 *
 *  Polls `/api/status/activity` for the authenticated user's geo-backfill
 *  work (reverse-geocoding GPS coordinates to city / state / country).
 *  Same UX pattern as the encryption and conversion banners.
 *
 *  Drives `useProcessingStore` so the profile-avatar spinner reflects
 *  whether the server is actively resolving locations for this user. */
import { useState, useEffect, useRef, useCallback } from "react";
import { request } from "../api/core";
import { useAuthStore } from "../store/auth";
import { useProcessingStore } from "../store/processing";
import { ProgressBanner } from "./ProgressBanner";

interface ActivityResponse {
  geo_progress?: {
    active: boolean;
    total: number;
    done: number;
    pending: number;
    /** `false` ⇒ photos are waiting but the GeoNames dataset isn't loadable,
     *  so they can never resolve. Omitted by older servers (treat as `true`). */
    available?: boolean;
    /** `true` ⇒ the server is downloading the dataset right now (self-healing
     *  a failed install). Omitted by older servers (treat as `false`). */
    downloading?: boolean;
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

export default function GeoBanner() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const [dismissed, setDismissed] = useState(false);
  const [counts, setCounts] = useState<{ total: number; done: number } | null>(null);
  const [eta, setEta] = useState<string | null>(null);
  // True when photos are awaiting geocoding but the server reports the
  // GeoNames dataset is missing/unloadable — they will never resolve, so we
  // show a static notice instead of a spinner that runs forever.
  const [unavailable, setUnavailable] = useState(false);
  // True while the server is actively downloading the GeoNames dataset to
  // self-heal a failed install — shown as a transient progress notice.
  const [downloading, setDownloading] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { startTask, endTask } = useProcessingStore();

  const batchSizeRef = useRef(0);
  const prevPendingRef = useRef(0);
  const batchStartRef = useRef(0);

  const poll = useCallback(async () => {
    try {
      const res = await request<ActivityResponse>("/status/activity");
      const geo = res.geo_progress;
      if (!geo) {
        endTask("geo");
        return;
      }
      const pending = Math.max(0, geo.pending);

      if (pending === 0) {
        batchSizeRef.current = 0;
        prevPendingRef.current = 0;
        batchStartRef.current = 0;
        setCounts(null);
        setEta(null);
        setUnavailable(false);
        setDownloading(false);
        endTask("geo");
        return;
      }

      // Server is fetching the dataset right now (self-healing a failed
      // install). Transient — show a "downloading" notice and keep the avatar
      // spinning; resolution kicks in automatically once it lands.
      if (geo.downloading === true) {
        batchSizeRef.current = 0;
        prevPendingRef.current = 0;
        batchStartRef.current = 0;
        setCounts(null);
        setEta(null);
        setUnavailable(false);
        setDownloading(true);
        startTask("geo");
        return;
      }
      setDownloading(false);

      // Dataset can't load (default `true` keeps old servers working): photos
      // are stuck, not progressing. Stop the spinner and surface why.
      if (geo.available === false) {
        batchSizeRef.current = 0;
        prevPendingRef.current = 0;
        batchStartRef.current = 0;
        setCounts(null);
        setEta(null);
        setUnavailable(true);
        endTask("geo");
        return;
      }

      setUnavailable(false);

      if (prevPendingRef.current === 0) {
        batchSizeRef.current = pending;
        batchStartRef.current = Date.now();
        prevPendingRef.current = pending;
        setCounts({ total: pending, done: 0 });
        setEta(null);
        startTask("geo");
        return;
      }

      if (pending > prevPendingRef.current) {
        const added = pending - prevPendingRef.current;
        batchSizeRef.current += added;
      }
      prevPendingRef.current = pending;

      const batchDone = batchSizeRef.current - pending;
      setCounts({ total: batchSizeRef.current, done: batchDone });
      startTask("geo");

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
      endTask("geo");
    };
  }, [isAuthenticated, poll, endTask]);

  if (dismissed) return null;

  // Dataset is being downloaded right now: transient progress notice (spinner,
  // no percentage — the fetch is a single ~25 MB archive).
  if (downloading) {
    return (
      <ProgressBanner
        position="bottom-44"
        tone="emerald"
        label="Downloading location data…"
        description="Fetching the GeoNames dataset. Photos with GPS will resolve once it finishes."
      />
    );
  }

  // Dataset unavailable: static, dismissible warning (no spinner, no progress).
  if (unavailable) {
    return (
      <div className="fixed bottom-44 left-4 right-4 z-50 pointer-events-none">
        <div className="card shadow-card-hover border-amber-300 dark:border-amber-700 pointer-events-auto max-w-md mx-auto flex items-center gap-3 px-4 py-3">
          <svg className="w-5 h-5 text-amber-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9-.75a9 9 0 11-18 0 9 9 0 0118 0zm-9 3.75h.008v.008H12v-.008z" />
          </svg>
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium text-fg-muted">
              Location data unavailable
            </p>
            <p className="text-xs text-fg-muted mt-0.5">
              Reverse geocoding is paused — the GeoNames dataset isn't installed
              on the server. Photos with GPS will resolve once it's available.
            </p>
          </div>
          <button
            onClick={() => setDismissed(true)}
            className="icon-btn p-1 flex-shrink-0"
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

  if (!counts || counts.total === 0) return null;

  const pct = counts.total > 0 ? (counts.done / counts.total) * 100 : 0;

  return (
    <ProgressBanner
      position="bottom-44"
      tone="emerald"
      label={`Resolving locations… ${counts.done}/${counts.total}`}
      eta={eta}
      pct={pct}
      onDismiss={() => setDismissed(true)}
    />
  );
}
