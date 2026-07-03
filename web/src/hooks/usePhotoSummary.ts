/**
 * Precomputed gallery-count summary (Issue 3).
 *
 * Fetches `GET /api/photos/summary` — a cheap, server-side, TTL-cached aggregate
 * of the smart-album counts — so badges can render **instantly on a cold cache**
 * without paginating the whole `encrypted-sync` endpoint to recount. Once the
 * local IndexedDB mirror is populated, callers should prefer the live local
 * counts (they update instantly on favorite toggles etc.); the summary only
 * bridges the gap before/while that mirror fills.
 *
 * Refetches whenever the real-time sync signal ({@link useSyncSignal}) bumps, so
 * a change made on another device is reflected without waiting out the server
 * cache TTL.
 */
import { useEffect, useState } from "react";
import { photosApi } from "../api/photos";
import { useAuthStore } from "../store/auth";
import { useSyncSignal } from "../store/syncSignal";

export interface PhotoSummary {
  total: number;
  collapsed_total: number;
  photos: number;
  gifs: number;
  videos: number;
  audio: number;
  favorites: number;
}

export function usePhotoSummary(): PhotoSummary | null {
  const [summary, setSummary] = useState<PhotoSummary | null>(null);
  const accessToken = useAuthStore((s) => s.accessToken);
  const version = useSyncSignal((s) => s.version);

  useEffect(() => {
    if (!accessToken) {
      setSummary(null);
      return;
    }
    let cancelled = false;
    photosApi
      .summary()
      .then((s) => {
        if (!cancelled) setSummary(s);
      })
      .catch((e) => {
        // Non-fatal: local IndexedDB counts remain the fallback. Log so a broken
        // endpoint (e.g. stale server binary) is visible rather than silent.
        console.error("[usePhotoSummary] failed to load /photos/summary", e);
      });
    return () => {
      cancelled = true;
    };
  }, [accessToken, version]);

  return summary;
}
