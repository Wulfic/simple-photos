/**
 * Precomputed gallery-count summary (Issue 3).
 *
 * Fetches `GET /api/photos/summary` — a cheap, server-side, TTL-cached aggregate
 * of the smart-album counts — so badges can render **instantly on a cold cache**
 * without paginating the whole `encrypted-sync` endpoint to recount.
 *
 * This is the **authoritative** count source (#42). It previously served only as
 * a cold-start bridge, with callers preferring live counts derived from the
 * local IndexedDB mirror — but that mirror only holds rows carrying an encrypted
 * blob, so it under-reports the library by the entire pending-encryption
 * backlog. Measured on the live server: 2,494 of 14,874 rows, i.e. the badge was
 * short by ~17%. Local counts are now only a fallback for when this endpoint has
 * not answered yet.
 *
 * Responsiveness is preserved server-side: mutations that move a count
 * (favorite toggle, trash, import) call `SummaryCache::invalidate`, so the next
 * fetch recomputes rather than waiting out the TTL.
 *
 * The last-seen summary is persisted to localStorage (keyed by user) and
 * hydrated **synchronously** on mount, so reopening the Albums page paints the
 * real smart-album counts on the very first frame instead of flashing "0" for a
 * network round-trip — the wait the previous cold-fetch-every-mount caused.
 * We still revalidate in the background (stale-while-revalidate) and whenever the
 * real-time sync signal ({@link useSyncSignal}) bumps, so a change made on
 * another device is reflected without waiting out the server cache TTL.
 */
import { useEffect, useState } from "react";
import { photosApi } from "../api/photos";
import { useAuthStore } from "../store/auth";
import { useSyncSignal } from "../store/syncSignal";

export interface PhotoSummary {
  /** Raw eligible ROW count — no burst collapse. */
  total: number;
  /** Tiles the main gallery grid renders (bursts collapsed). */
  collapsed_total: number;
  photos: number;
  gifs: number;
  videos: number;
  audio: number;
  favorites: number;
  /** Tiles in the "Photos" smart album — photo + gif, burst-collapsed. */
  smart_photos: number;
  smart_gifs: number;
  smart_videos: number;
  smart_audio: number;
  smart_favorites: number;
  /** Tiles in "Recently Added" (whole library, capped at 100). */
  smart_recent: number;
  /** Head of the server's photo change log (#38).
   *
   *  If this equals the sequence the client last synced to, nothing has
   *  changed and `encrypted-sync` can be skipped entirely — no pagination, no
   *  IndexedDB writes, no blob downloads. Served fresh even when the rest of
   *  the summary comes from the server's TTL cache, so it is safe to compare
   *  on every poll. */
  head_seq: number;
}

/** localStorage key holding `{ user, summary }` for the last-seen summary. */
export const PHOTO_SUMMARY_CACHE_KEY = "sp_photo_summary_v1";

/** Read the persisted summary, but only when it belongs to the current user. */
function readCache(user: string | null): PhotoSummary | null {
  if (!user) return null;
  try {
    const raw = localStorage.getItem(PHOTO_SUMMARY_CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { user: string; summary: PhotoSummary };
    return parsed.user === user ? parsed.summary : null;
  } catch {
    return null;
  }
}

function writeCache(user: string | null, summary: PhotoSummary): void {
  if (!user) return;
  try {
    localStorage.setItem(PHOTO_SUMMARY_CACHE_KEY, JSON.stringify({ user, summary }));
  } catch {
    // Quota exceeded / storage disabled — non-fatal, the fetch result still
    // populates in-memory state for this session.
  }
}

export function usePhotoSummary(): PhotoSummary | null {
  const accessToken = useAuthStore((s) => s.accessToken);
  const username = useAuthStore((s) => s.username);
  const version = useSyncSignal((s) => s.version);
  // Hydrate synchronously from the last persisted summary so counts paint on the
  // first frame; the effect below revalidates against the server.
  const [summary, setSummary] = useState<PhotoSummary | null>(() => readCache(username));

  useEffect(() => {
    if (!accessToken) {
      setSummary(null);
      return;
    }
    // Paint the persisted counts the moment auth is ready — covers a hard reload
    // where `username` wasn't populated yet at the initial (synchronous) hydration
    // above — then revalidate against the server below.
    const cached = readCache(username);
    if (cached) setSummary((prev) => prev ?? cached);
    let cancelled = false;
    photosApi
      .summary()
      .then((s) => {
        if (cancelled) return;
        setSummary(s);
        writeCache(username, s);
      })
      .catch((e) => {
        // Non-fatal: the persisted summary + local IndexedDB counts remain the
        // fallback. Log so a broken endpoint (e.g. stale server binary) is
        // visible rather than silent.
        console.error("[usePhotoSummary] failed to load /photos/summary", e);
      });
    return () => {
      cancelled = true;
    };
  }, [accessToken, username, version]);

  return summary;
}
