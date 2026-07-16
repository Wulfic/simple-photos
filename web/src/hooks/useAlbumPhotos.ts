/**
 * useAlbumPhotos — the single client-side resolver for "what photos are in this
 * album, and how many".
 *
 * Regular albums are E2E-encrypted manifests (the server only stores ciphertext)
 * and smart albums are live filters over the local mirror, so there is no
 * server-side `album_members` endpoint that could unify them — the unification
 * has to live on the client. Before this hook, RegularAlbumView and
 * SmartAlbumView each resolved membership and counts their own way, which is why
 * a regular album's header count (raw `photoBlobIds.length`, including
 * secure-hidden and stale ids) drifted from the grid it actually rendered
 * (secure-filtered) — see #12 (missing/wrong counts) and #20 (count flicker
 * 7000→5645). Here `count === photos.length`, always, from one source.
 *
 * Contract (same shape for every album kind):
 *   - `photos`  — resolved membership, secure-excluded, in display order.
 *   - `count`   — always `photos.length` (never a divergent raw manifest size).
 *   - `album`   — the manifest, for regular albums only (CRUD operations).
 *   - `kind`    — "smart" | "regular" | "unknown".
 */
import { useEffect, useMemo, useRef } from "react";
import { useLiveQuery } from "dexie-react-hooks";
import { db, type CachedPhoto, type CachedAlbum } from "../db";
import { useSecureBlobFilter } from "../gallery/hooks/useSecureBlobFilter";
import { SMART_ALBUM_DEFS, type SmartAlbumDef } from "../gallery/smartAlbums";
import { collapseBursts, type PhotoWithBurstCount } from "../utils/burstCollapse";

export type AlbumKind = "smart" | "regular" | "unknown";

export interface UseAlbumPhotosResult {
  /** Resolved album membership, secure-excluded, in display order. */
  photos: PhotoWithBurstCount[];
  /** Always equals `photos.length` — the single source of truth for the badge. */
  count: number;
  /** True until the underlying live queries have first resolved. */
  loading: boolean;
  /** Present for regular (manifest-backed) albums; used for CRUD. */
  album?: CachedAlbum;
  kind: AlbumKind;
  /** Full local photo mirror (takenAt desc). Exposed so callers can build the
   *  complement (e.g. the regular-album "add photos" picker) without a second
   *  live query. */
  allPhotos: CachedPhoto[];
  /** Blob IDs currently inside a secure gallery (live-polled). */
  secureBlobIds: Set<string>;
}

/**
 * Pure resolution core — no React, no Dexie. Given the raw inputs, produce the
 * album's photo list. Kept separate so membership + count behaviour is unit
 * testable in isolation.
 *
 * Smart albums: filter the mirror by the album's predicate, apply the album's
 * ordering (addedAt for "Recently Added"), collapse bursts, then cap to
 * `limit`. Regular albums: intersect the mirror with the manifest's blob ids,
 * preserving the mirror's takenAt-desc order (NOT manifest order, matching the
 * historical view). Both exclude secure blob ids.
 *
 * Note: bursts are collapsed only for smart albums. Regular albums keep every
 * frame a user explicitly added, so removal/secure-add over the rendered list
 * stays faithful to the manifest.
 */
export function resolveAlbumPhotos(params: {
  kind: AlbumKind;
  allPhotos: CachedPhoto[];
  secureBlobIds: Set<string>;
  smartDef?: SmartAlbumDef;
  album?: CachedAlbum;
}): PhotoWithBurstCount[] {
  const { kind, allPhotos, secureBlobIds, smartDef, album } = params;

  if (kind === "smart" && smartDef) {
    let next = allPhotos
      .filter((p) => !secureBlobIds.has(p.blobId))
      .filter(smartDef.filterEncrypted);
    if (smartDef.sortBy === "addedAt") {
      next = [...next].sort(
        (a, b) => (b.addedAt ?? b.takenAt ?? 0) - (a.addedAt ?? a.takenAt ?? 0)
      );
    }
    let collapsed = collapseBursts(next);
    if (smartDef.limit !== undefined) {
      collapsed = collapsed.slice(0, smartDef.limit);
    }
    return collapsed;
  }

  if (kind === "regular" && album) {
    const members = new Set(album.photoBlobIds);
    return allPhotos.filter(
      (p) => members.has(p.blobId) && !secureBlobIds.has(p.blobId)
    );
  }

  return [];
}

/**
 * Count a regular album's *visible* members — the manifest intersected with the
 * local mirror, secure-excluded. This is the single source for the album-list
 * badges (Albums.tsx tiles, AddToAlbumModal) so a tile's count can never diverge
 * from the grid the detail view renders via {@link resolveAlbumPhotos} (#12
 * missing/wrong counts, #20 flicker). Raw `photoBlobIds.length` must never be
 * shown as the count — it includes secure-hidden and stale (deleted) ids.
 *
 * When the mirror hasn't loaded yet (cold cache) this falls back to the count we
 * last persisted, so a tile opens on its previous stable number. Only if there
 * has never been one does it fall back to the raw manifest size (still better
 * than flashing 0). Either way the accurate intersection takes over the moment
 * the mirror holds photos.
 */
export function countRegularAlbum(
  album: Pick<CachedAlbum, "photoBlobIds" | "cachedCount">,
  allPhotos: CachedPhoto[] | undefined,
  secureBlobIds: Set<string>
): number {
  if (!allPhotos || allPhotos.length === 0) {
    return album.cachedCount ?? album.photoBlobIds.length;
  }
  const mirror = new Set(allPhotos.map((p) => p.blobId));
  let n = 0;
  for (const id of album.photoBlobIds) {
    if (mirror.has(id) && !secureBlobIds.has(id)) n++;
  }
  return n;
}

/**
 * Persist an album's freshly-resolved count so the next mount can render it
 * before the mirror has loaded.
 *
 * The `cachedCount === count` guard is load-bearing, not an optimisation: every
 * album tile is driven by a Dexie live query, so an unconditional write would
 * re-emit the album list, re-run this reconciliation, and write again — forever.
 */
export async function reconcileAlbumCount(
  albumId: string,
  count: number
): Promise<void> {
  try {
    const album = await db.albums.get(albumId);
    if (!album || album.cachedCount === count) return;
    await db.albums.update(albumId, { cachedCount: count });
  } catch (e) {
    // A render hint failing to persist must never break the view.
    console.warn(`[useAlbumPhotos] could not cache count for album ${albumId}`, e); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
  }
}

export function useAlbumPhotos(
  albumId: string | undefined
): UseAlbumPhotosResult {
  const { secureBlobIds, refreshSecureBlobIds, startPolling } =
    useSecureBlobFilter();

  // One-shot fetch + live polling so photos moved into/out of a secure gallery
  // on another device are reflected here (relevant to #16 leak and #20 flicker).
  useEffect(() => {
    void refreshSecureBlobIds();
    startPolling();
    // refresh/startPolling are stable for the hook's lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const smartDef = albumId ? SMART_ALBUM_DEFS[albumId] : undefined;
  const kind: AlbumKind = smartDef ? "smart" : albumId ? "regular" : "unknown";

  const allPhotos = useLiveQuery(
    () => db.photos.orderBy("takenAt").reverse().toArray(),
    []
  );

  const album = useLiveQuery(
    () => (kind === "regular" && albumId ? db.albums.get(albumId) : undefined),
    [kind, albumId]
  );

  const loading =
    allPhotos === undefined || (kind === "regular" && album === undefined);

  // Stabilise the resolved list: an identical membership must not produce a new
  // array reference, or JustifiedGrid re-mounts and the scroll position jumps.
  const prevKeyRef = useRef<string>("");
  const prevListRef = useRef<PhotoWithBurstCount[]>([]);

  const photos = useMemo(() => {
    if (!allPhotos) return prevListRef.current;
    // Regular album whose manifest hasn't loaded yet: hold the previous list
    // rather than flashing empty (contributes to #20's count flicker).
    if (kind === "regular" && !album) return prevListRef.current;

    const next = resolveAlbumPhotos({
      kind,
      allPhotos,
      secureBlobIds,
      smartDef,
      album: album ?? undefined,
    });

    const key = next.map((p) => p.blobId).join(",");
    if (key === prevKeyRef.current) return prevListRef.current;
    prevKeyRef.current = key;
    prevListRef.current = next;
    return next;
  }, [allPhotos, album, kind, smartDef, secureBlobIds]);

  // Feed the resolved count back into the cache: this hook *is* the album's
  // authoritative count, so opening an album is what teaches its tile the right
  // number to show next time.
  const albumIdForCache = album?.albumId;
  useEffect(() => {
    if (loading || kind !== "regular" || !albumIdForCache) return;
    void reconcileAlbumCount(albumIdForCache, photos.length);
  }, [loading, kind, albumIdForCache, photos.length]);

  return {
    photos,
    count: photos.length,
    loading,
    album: album ?? undefined,
    kind,
    allPhotos: allPhotos ?? [],
    secureBlobIds,
  };
}
