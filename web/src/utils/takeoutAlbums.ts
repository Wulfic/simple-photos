/**
 * Google Takeout album recreation.
 *
 * Google Takeout lays photos out in per-album folders (plus non-album
 * "Photos from YYYY" date folders). Regular albums in Simple Photos are
 * end-to-end-encrypted manifests keyed by a photo's **blobId**, created
 * entirely client-side — the server can't build them. blobIds are only known
 * once a photo has synced, so album recreation runs as a post-sync pass that
 * matches each album folder's filenames against the user's already-synced
 * photos and writes the album manifests (reusing the same encrypt → upload →
 * Dexie pattern as `AddToAlbumModal`).
 *
 * Photo *edits* are already handled upstream: `dedupeGooglePhotosEdits` keeps
 * the baked-in `-edited` copy, so the edited pixels are what gets imported.
 */
import { db, type CachedAlbum } from "../db";
import { encrypt, sha256Hex } from "../crypto/crypto";
import { api } from "../api/client";
import { randomUuid } from "./uuid";

/** Google's non-album date folders, e.g. "Photos from 2023". Not real albums. */
const DATE_FOLDER_RE = /^Photos from \d{4}/i;

/** Generic Takeout container folders (any language variant is matched loosely). */
const NON_ALBUM_FOLDERS = new Set(["takeout", "google photos", "google fotos", ""]);

/**
 * Build a map of `album-name → set of media filenames` from a Takeout folder
 * selection (files carry `webkitRelativePath`). The album is each file's
 * immediate parent directory, excluding Google's date folders and the Takeout
 * container folders. Sidecar `.json` files are ignored.
 */
export function parseTakeoutFolders(files: FileList | File[]): Map<string, Set<string>> {
  const map = new Map<string, Set<string>>();
  for (const file of Array.from(files)) {
    const rel = (file as File & { webkitRelativePath?: string }).webkitRelativePath || "";
    if (!rel) continue;
    const parts = rel.split("/").filter(Boolean);
    if (parts.length < 2) continue;
    const filename = parts[parts.length - 1];
    if (filename.toLowerCase().endsWith(".json")) continue;
    const folder = parts[parts.length - 2];
    if (
      !folder ||
      DATE_FOLDER_RE.test(folder) ||
      NON_ALBUM_FOLDERS.has(folder.toLowerCase())
    ) {
      continue;
    }
    let set = map.get(folder);
    if (!set) {
      set = new Set();
      map.set(folder, set);
    }
    set.add(filename);
  }
  return map;
}

export interface RecreateResult {
  albumsCreated: number;
  albumsUpdated: number;
  photosAdded: number;
  /** Albums whose filenames matched no synced photos (e.g. not synced yet). */
  albumsUnmatched: number;
}

/**
 * Recreate (or merge into) local albums from a parsed Takeout folder map by
 * matching each album's filenames against the user's already-synced photos.
 * Idempotent: an album with the same name is merged into rather than
 * duplicated, and re-running adds nothing new.
 */
export async function recreateTakeoutAlbums(
  albumMap: Map<string, Set<string>>,
): Promise<RecreateResult> {
  const result: RecreateResult = {
    albumsCreated: 0,
    albumsUpdated: 0,
    photosAdded: 0,
    albumsUnmatched: 0,
  };

  // filename (lowercased) → blobIds. A filename can map to several blobs if the
  // library has duplicates; add them all so the album is complete.
  const photos = await db.photos.toArray();
  const byName = new Map<string, string[]>();
  for (const p of photos) {
    const key = p.filename.toLowerCase();
    const arr = byName.get(key);
    if (arr) arr.push(p.blobId);
    else byName.set(key, [p.blobId]);
  }

  const existingByName = new Map<string, CachedAlbum>();
  for (const a of await db.albums.toArray()) {
    existingByName.set(a.name.toLowerCase(), a);
  }

  for (const [name, filenames] of albumMap) {
    const blobIds = new Set<string>();
    for (const fn of filenames) {
      const matches = byName.get(fn.toLowerCase());
      if (matches) for (const b of matches) blobIds.add(b);
    }
    if (blobIds.size === 0) {
      result.albumsUnmatched++;
      continue;
    }

    const existing = existingByName.get(name.toLowerCase());
    if (existing) {
      const merged = [...new Set([...existing.photoBlobIds, ...blobIds])];
      const added = merged.length - existing.photoBlobIds.length;
      if (added === 0) continue;
      await saveAlbumManifest({
        ...existing,
        photoBlobIds: merged,
        coverPhotoBlobId: existing.coverPhotoBlobId || merged[0],
      });
      result.albumsUpdated++;
      result.photosAdded += added;
    } else {
      const ids = [...blobIds];
      await saveAlbumManifest({
        albumId: randomUuid(),
        manifestBlobId: "",
        name,
        createdAt: Date.now(),
        coverPhotoBlobId: ids[0],
        photoBlobIds: ids,
      });
      result.albumsCreated++;
      result.photosAdded += ids.length;
    }
  }
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
