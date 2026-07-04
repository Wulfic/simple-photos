/**
 * Google Takeout album recreation (automatic).
 *
 * Google Takeout lays photos out in per-album folders. The server now captures
 * that album membership **at import time**, keyed by each photo's id
 * (`GET /api/photos/source-albums`) — see `server/src/import/takeout.rs`.
 *
 * Regular albums in Simple Photos are end-to-end-encrypted manifests keyed by a
 * photo's **blobId**, created entirely client-side (the server can't build them
 * because it can't read the encryption key). So album recreation runs client-
 * side as an automatic, idempotent pass that maps the server's authoritative
 * `photo_id → album` mapping onto locally-synced photos and writes the album
 * manifests. It runs on its own after a sync — there is no manual "rebuild"
 * step (it used to be a fragile, manual, filename-matching folder re-selection).
 */
import { db, type CachedAlbum } from "../db";
import { encrypt, sha256Hex } from "../crypto/crypto";
import { api } from "../api/client";

export interface ServerRecreateResult {
  albumsCreated: number;
  albumsUpdated: number;
  photosAdded: number;
  /** Albums whose photos aren't in the local mirror yet (still syncing). */
  albumsUnmatched: number;
  /** Individual photo ids not yet synced locally (skipped, re-run to fill). */
  photosUnmatched: number;
}

/**
 * Recreate (or merge into) local albums from the server's **authoritative**
 * source-album mapping (`GET /api/photos/source-albums`), keyed by photo id.
 *
 * Deterministic and cross-platform: the server captured each Takeout album
 * folder at import time keyed by the photo's id, so this survives `-edited`
 * dedup and `IMG_1234(1).jpg` collision renames, and needs no manual folder
 * re-selection. A photo id that isn't in the local IndexedDB mirror yet (still
 * syncing) is skipped and counted; re-running after the sync completes fills it
 * in. Idempotent — albums key on a deterministic id (and fall back to a
 * same-named existing album), so re-running merges rather than duplicating.
 */
export async function recreateAlbumsFromServer(): Promise<ServerRecreateResult> {
  const result: ServerRecreateResult = {
    albumsCreated: 0,
    albumsUpdated: 0,
    photosAdded: 0,
    albumsUnmatched: 0,
    photosUnmatched: 0,
  };

  const { albums } = await api.photos.sourceAlbums();
  if (!albums || albums.length === 0) return result;

  // serverPhotoId → blobId. Album manifests are keyed by blobId, but the server
  // mapping is keyed by photo id, so bridge via the id we stored at sync time.
  const photos = await db.photos.toArray();
  const blobByServerId = new Map<string, string>();
  for (const p of photos) {
    if (p.serverPhotoId) blobByServerId.set(p.serverPhotoId, p.blobId);
  }

  const allAlbums = await db.albums.toArray();
  const existingByName = new Map<string, CachedAlbum>();
  const existingById = new Map<string, CachedAlbum>();
  for (const a of allAlbums) {
    existingByName.set(a.name.toLowerCase(), a);
    existingById.set(a.albumId, a);
  }

  // Phase 1 (cheap, sequential): resolve each source album to the manifest we'd
  // need to write, WITHOUT touching the network. Albums that are already fully
  // materialized (every matched photo present in the existing manifest) are
  // short-circuited here so a re-run costs zero encrypt/upload round-trips — the
  // common steady state once the first pass completes.
  const jobs: CachedAlbum[] = [];
  for (const album of albums) {
    const blobIds = new Set<string>();
    for (const pid of album.photo_ids) {
      const blob = blobByServerId.get(pid);
      if (blob) blobIds.add(blob);
      else result.photosUnmatched++;
    }
    if (blobIds.size === 0) {
      result.albumsUnmatched++;
      continue;
    }

    // Deterministic album id derived from (source, name) so a rebuild run on
    // web and on Android produces the *same* id and converges into one album
    // after sync — instead of two identically-named albums. Both platforms use
    // this exact formula (see AlbumRepository.recreateAlbumsFromServer).
    const albumId = "src-" + (await sha256Hex(
      new TextEncoder().encode(`${album.source} ${album.name}`),
    ));

    // Prefer an album already carrying this deterministic id; otherwise merge
    // into a user's manually-created same-named album rather than duplicating.
    const existing = existingById.get(albumId) ?? existingByName.get(album.name.toLowerCase());
    if (existing) {
      const merged = [...new Set([...existing.photoBlobIds, ...blobIds])];
      const added = merged.length - existing.photoBlobIds.length;
      if (added === 0) continue; // no-op: nothing new to write, skip the upload
      jobs.push({
        ...existing,
        photoBlobIds: merged,
        coverPhotoBlobId: existing.coverPhotoBlobId || merged[0],
      });
      result.albumsUpdated++;
      result.photosAdded += added;
    } else {
      const ids = [...blobIds];
      jobs.push({
        albumId,
        manifestBlobId: "",
        name: album.name,
        createdAt: Date.now(),
        coverPhotoBlobId: ids[0],
        photoBlobIds: ids,
      });
      result.albumsCreated++;
      result.photosAdded += ids.length;
    }
  }

  // Phase 2 (expensive, parallel): encrypt + upload each manifest. Each job is an
  // independent network round-trip, so a bounded worker pool collapses what was a
  // sequential per-album stall (an hour on large libraries) into ~jobs/CONCURRENCY
  // waves. Bounded so we don't flood the server mid-import. Mirrors the upload
  // worker-pool pattern in pages/Import.tsx.
  const CONCURRENCY = 6;
  let next = 0;
  const worker = async () => {
    while (next < jobs.length) {
      const job = jobs[next++];
      try {
        await saveAlbumManifest(job);
      } catch (e) {
        // One album failing must not abort the rest; a later re-run retries it.
        console.error(`[takeoutAlbums] manifest upload failed for "${job.name}"`, e); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
      }
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(CONCURRENCY, jobs.length) }, () => worker()),
  );
  return result;
}

/** Encrypt + upload an album manifest and persist it locally. */
async function saveAlbumManifest(album: CachedAlbum): Promise<void> {
  // Replace the previous manifest blob (best-effort) when updating.
  if (album.manifestBlobId) {
    try {
      await api.blobs.delete(album.manifestBlobId);
    } catch {
      /* already gone */
    }
  }
  const payload = JSON.stringify({
    v: 1,
    album_id: album.albumId,
    name: album.name,
    created_at: new Date(album.createdAt).toISOString(),
    cover_photo_blob_id: album.coverPhotoBlobId || null,
    photo_blob_ids: album.photoBlobIds,
  });
  const encrypted = await encrypt(new TextEncoder().encode(payload));
  const hash = await sha256Hex(new Uint8Array(encrypted));
  const res = await api.blobs.upload(encrypted, "album_manifest", hash);
  await db.albums.put({ ...album, manifestBlobId: res.blob_id });
}
