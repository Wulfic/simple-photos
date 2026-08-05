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
import { sha256Hex } from "../crypto/crypto";
import { api } from "../api/client";
import { saveAlbumManifest } from "./albumManifest";

export interface ServerRecreateResult {
  albumsCreated: number;
  albumsUpdated: number;
  photosAdded: number;
  /** Albums re-titled from the Takeout folder name to their real Google name. */
  albumsRenamed: number;
  /** Albums whose photos aren't in the local mirror yet (still syncing). */
  albumsUnmatched: number;
  /** Individual photo ids not yet synced locally (skipped, re-run to fill). */
  photosUnmatched: number;
}

/** One album as the server captured it (`GET /api/photos/source-albums`). */
export interface SourceAlbum {
  /** The Takeout folder name — the album's identity, mangled by Google. */
  name: string;
  /** The album's real Google Photos title, when the export carried one. */
  title: string | null;
  source: string;
  photo_ids: string[];
}

/**
 * The deterministic local album id for a source album.
 *
 * Derived from `(source, folder name)` so a rebuild on web and on Android
 * produces the **same** id and the two converge into one album after sync,
 * instead of two identically-named ones. The server computes it too, to resolve
 * a `src-…` id back to the album it came from when tombstoning a deletion.
 * All three implementations are pinned to one shared test vector — a drift here
 * is silent and only shows up as duplicated or resurrected albums.
 *
 * Keyed on the folder name, never the title: identity must survive a retitle.
 */
export async function sourceAlbumId(
  source: string,
  folderName: string,
): Promise<string> {
  return (
    "src-" + (await sha256Hex(new TextEncoder().encode(`${source} ${folderName}`)))
  );
}

/**
 * The name to show for a source album, given the local album it maps onto
 * (`existingName` — `undefined` when we're about to create it).
 *
 * Takeout folder names are mangled ("Mum & Dad's 40th" exports as
 * "Mum _ Dad_s 40th"), so the real title from the album's `metadata.json` wins
 * for display. But the folder name stays the album's *identity* — it derives the
 * deterministic album id — so renaming is purely cosmetic and never re-keys.
 *
 * The rename only ever supersedes a name **we** wrote (i.e. still exactly the
 * raw folder name). If the local album is called anything else, the user renamed
 * it — or it's their own album we merged into — and we leave it alone rather than
 * stomping their curation on every reconstruction pass.
 */
export function resolveAlbumDisplayName(
  album: Pick<SourceAlbum, "name" | "title">,
  existingName?: string,
): string {
  const display = album.title?.trim() || album.name;
  if (existingName === undefined) return display;
  return existingName === album.name ? display : existingName;
}

/**
 * Whether a reconstruction pass proved there is nothing left to materialize, so
 * it can stop re-running.
 *
 * **Must stay identical to Android's `AlbumRepository.takeoutSettled`** — both
 * platforms run this same pass against the same server mapping, so a rule that
 * differs means one device keeps re-uploading manifests the other considers
 * finished, which is half of the churn this file's callers are trying to avoid.
 *
 * Normally settled means every source photo matched. But a photo that was
 * trashed or moved to the secure gallery never syncs into the mirror at all, so
 * `photosUnmatched` can never reach 0 and the pass would re-run forever. The
 * second clause catches that: a pass that changed nothing AND left exactly the
 * same gap as the one before it has proved the gap is permanent — more photos
 * arrived, none of them were the missing ones. Deliberately conservative;
 * latching early means silently incomplete albums, which is the bug this whole
 * path exists to fix.
 *
 * @param previousUnmatched the previous pass's gap, or -1 if there was none.
 */
export function takeoutSettled(
  result: Pick<
    ServerRecreateResult,
    "albumsCreated" | "albumsUpdated" | "photosAdded" | "photosUnmatched"
  >,
  previousUnmatched: number,
): boolean {
  if (result.photosUnmatched === 0) return true;
  const noop =
    result.albumsCreated === 0 &&
    result.albumsUpdated === 0 &&
    result.photosAdded === 0;
  return noop && result.photosUnmatched === previousUnmatched;
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
    albumsRenamed: 0,
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

    const albumId = await sourceAlbumId(album.source, album.name);

    // Prefer an album already carrying this deterministic id; otherwise merge
    // into a user's manually-created same-named album rather than duplicating.
    // The user's album is named after the real title, so try that before the raw
    // folder name (which is what earlier, title-less runs of this pass wrote).
    const display = resolveAlbumDisplayName(album);
    const existing =
      existingById.get(albumId) ??
      existingByName.get(display.toLowerCase()) ??
      existingByName.get(album.name.toLowerCase());
    if (existing) {
      const merged = [...new Set([...existing.photoBlobIds, ...blobIds])];
      const added = merged.length - existing.photoBlobIds.length;
      const name = resolveAlbumDisplayName(album, existing.name);
      const renamed = name !== existing.name;
      // No-op: nothing new to write, skip the upload. Checking the rename too is
      // what lets an album materialized under the mangled folder name by an
      // earlier run get its real title — that pass adds no photos, so a
      // photos-only check would skip it forever.
      if (added === 0 && !renamed) continue;
      jobs.push({
        ...existing,
        name,
        photoBlobIds: merged,
        coverPhotoBlobId: existing.coverPhotoBlobId || merged[0],
      });
      result.albumsUpdated++;
      result.photosAdded += added;
      if (renamed) result.albumsRenamed++;
    } else {
      const ids = [...blobIds];
      jobs.push({
        albumId,
        manifestBlobId: "",
        name: display,
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

