/**
 * Shared logic for moving photos into a secure (encrypted) album.
 *
 * Extracted from SecureGallery so the new cross-page "Add Photos" flow (browse
 * your regular/smart albums, multi-select, then add) can reuse the exact same
 * server clone + IndexedDB bookkeeping that the old inline picker used.
 */
import { api } from "../api/client";
import { db } from "../db";
import { encrypt, sha256Hex } from "../crypto/crypto";
import { expandBurstSelection } from "./burstExpand";

/**
 * Add a set of photos (by blob ID) to a secure gallery.
 *
 * For each photo: the server creates an independent encrypted clone under a new
 * blob ID; we mirror the IndexedDB cache entry to that clone (so its thumbnail
 * resolves), delete the original local entry (so it leaves the main gallery),
 * and finally strip the originals from every regular album manifest.
 *
 * Returns the number of photos actually added.
 */
export async function addPhotosToSecureGallery(
  galleryId: string,
  blobIds: string[],
): Promise<number> {
  // A selected burst representative stands in for its whole stack — pull in the
  // rest of the frames so the entire burst moves into the secure album, not
  // just the cover frame.
  const expanded = await expandBurstSelection(blobIds);
  const addedOriginalIds: string[] = [];

  for (const blobId of expanded) {
    const response = await api.secureGalleries.addItem(galleryId, blobId);
    if (response.new_blob_id) {
      const originalCached = await db.photos.get(blobId);
      if (originalCached) {
        // Clone the IDB entry under the new blob ID so the secure tile can
        // resolve a thumbnail. Server-side photos get a photos row keyed by the
        // new blob ID; clear storageBlobId so the clone owns its blob.
        await db.photos.put({
          ...originalCached,
          blobId: response.new_blob_id,
          serverPhotoId: originalCached.serverSide
            ? response.new_blob_id
            : originalCached.serverPhotoId,
          storageBlobId: undefined,
        });
      }
      addedOriginalIds.push(blobId);
    }
  }

  // Remove the originals from the local cache so they vanish from the main
  // gallery immediately (the server's secureBlobIds endpoint also hides them,
  // but that depends on polling).
  for (const origId of addedOriginalIds) {
    await db.photos.delete(origId);
  }

  // A photo moved to a secure album should no longer appear in any regular album.
  await removePhotosFromRegularAlbums(new Set(addedOriginalIds));

  return addedOriginalIds.length;
}

/**
 * Remove a set of blob IDs from every regular album and update the
 * corresponding album manifests on the server + local IndexedDB. Also clears
 * the albumIds on the photo records themselves.
 */
export async function removePhotosFromRegularAlbums(blobIds: Set<string>): Promise<void> {
  const allAlbums = await db.albums.toArray();

  for (const album of allAlbums) {
    const before = album.photoBlobIds.length;
    const updated = album.photoBlobIds.filter((id) => !blobIds.has(id));
    if (updated.length === before) continue; // nothing to change

    // Determine cover: clear it if the cover photo was removed
    const cover =
      album.coverPhotoBlobId && blobIds.has(album.coverPhotoBlobId)
        ? updated[0] || undefined
        : album.coverPhotoBlobId;

    // Delete old manifest blob from server
    if (album.manifestBlobId) {
      try {
        await api.blobs.delete(album.manifestBlobId);
      } catch {
        /* ok */
      }
    }

    // Upload a new manifest with the blob IDs removed
    const payload = JSON.stringify({
      v: 1,
      album_id: album.albumId,
      name: album.name,
      created_at: new Date(album.createdAt).toISOString(),
      cover_photo_blob_id: cover || null,
      photo_blob_ids: updated,
    });
    const encrypted = await encrypt(new TextEncoder().encode(payload));
    const hash = await sha256Hex(new Uint8Array(encrypted));
    const res = await api.blobs.upload(encrypted, "album_manifest", hash);

    // Update local cache
    await db.albums.put({
      ...album,
      photoBlobIds: updated,
      coverPhotoBlobId: cover,
      manifestBlobId: res.blob_id,
    });
  }

  // Clear albumIds on each photo so the gallery / album views stay consistent
  for (const blobId of blobIds) {
    const photo = await db.photos.get(blobId);
    if (photo && photo.albumIds.length > 0) {
      await db.photos.update(blobId, { albumIds: [] });
    }
  }
}
