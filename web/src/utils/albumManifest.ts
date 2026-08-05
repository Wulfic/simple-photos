/**
 * The one place a regular album's manifest is written.
 *
 * Regular albums are end-to-end-encrypted manifest blobs: the server stores
 * opaque bytes and cannot read, build, or repair them. That makes the write
 * order here the album's only durability guarantee — and five separate call
 * sites used to each re-implement it as *delete the old blob, then encrypt and
 * upload the new one*. Anything failing in that window (offline, tab closed, a
 * 500) left the album with no manifest on the server at all: it silently
 * vanished from every other device on their next sync, and no retry could bring
 * it back because the bytes were already gone.
 *
 * The correct order — upload new, persist the new id locally, then best-effort
 * delete the old — never has zero manifests on the server. A crash mid-way
 * leaves an orphaned blob, which is cheap and harmless, instead of a lost album.
 */
import { db, type CachedAlbum } from "../db";
import { encrypt, sha256Hex } from "../crypto/crypto";
import { api } from "../api/client";

/** The manifest payload both platforms read. Keep in step with Android's
 *  `AlbumRepository.syncAlbum`. */
function buildManifest(album: CachedAlbum): string {
  return JSON.stringify({
    v: 1,
    album_id: album.albumId,
    name: album.name,
    created_at: new Date(album.createdAt).toISOString(),
    cover_photo_blob_id: album.coverPhotoBlobId || null,
    photo_blob_ids: album.photoBlobIds,
  });
}

/**
 * Encrypt + upload `album`'s manifest, persist it locally, and clean up the
 * blob it replaced. Returns the album as stored (with its new `manifestBlobId`).
 *
 * Throws if the upload fails — the caller's album is then simply unchanged, both
 * locally and on the server, which is the state a retry can recover from.
 */
export async function saveAlbumManifest(
  album: CachedAlbum,
): Promise<CachedAlbum> {
  const encrypted = await encrypt(new TextEncoder().encode(buildManifest(album)));
  const hash = await sha256Hex(new Uint8Array(encrypted));
  const res = await api.blobs.upload(encrypted, "album_manifest", hash);

  const saved = { ...album, manifestBlobId: res.blob_id };
  await db.albums.put(saved);

  // Only now is the old blob unreferenced. Best-effort: an orphan costs a few
  // hundred bytes, while failing here must not undo a successful save.
  const previous = album.manifestBlobId;
  if (previous && previous !== res.blob_id) {
    try {
      await api.blobs.delete(previous);
    } catch (e) {
      console.warn(`[albumManifest] could not delete replaced manifest blob for "${album.name}"`, e); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
    }
  }
  return saved;
}
