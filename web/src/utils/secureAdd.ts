/**
 * Shared logic for moving photos into a secure (encrypted) album.
 *
 * Extracted from SecureGallery so the new cross-page "Add Photos" flow (browse
 * your regular/smart albums, multi-select, then add) can reuse the exact same
 * server clone + IndexedDB bookkeeping that the old inline picker used.
 */
import { api } from "../api/client";
import { db } from "../db";
import { saveAlbumManifest } from "./albumManifest";
import { expandBurstSelection } from "./burstExpand";

/** Outcome of a secure-add batch. `failed` photos stayed in the regular gallery. */
export interface SecureAddResult {
  /** Photos successfully moved into the secure gallery. */
  added: number;
  /** Photos that could not be secured — they remain visible in the gallery. */
  failed: number;
}

/**
 * Resilient batch runner. Applies `addOne` to every id, isolating each so one
 * failure never aborts the batch — that was the #16 leak: a single throw (or a
 * response with no clone id) stopped the loop, leaving every *later* photo
 * un-secured yet still shown in the regular gallery ("most items removed but not
 * all"). Now every id is attempted; failures are logged and partitioned out.
 *
 * `addOne` resolves the new secure blob id on success, or `null` when the server
 * accepted the request but produced no clone (the photo was not actually moved).
 * Kept dependency-injected + pure of `api`/`db` so the resilience is unit-tested.
 */
export async function runSecureAddBatch(
  ids: string[],
  addOne: (blobId: string) => Promise<string | null>,
): Promise<{ added: string[]; failed: string[] }> {
  const added: string[] = [];
  const failed: string[] = [];

  for (const blobId of ids) {
    try {
      const newBlobId = await addOne(blobId);
      if (newBlobId) {
        added.push(blobId);
      } else {
        // Server returned no clone id — treat as not-moved so it isn't stripped
        // from the gallery while lacking a secure copy.
        console.warn(`[SECURE_ADD] No secure clone returned for ${blobId}; it stays in the regular gallery`); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
        failed.push(blobId);
      }
    } catch (err) {
      console.warn(`[SECURE_ADD] Failed to secure ${blobId}:`, err); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
      failed.push(blobId);
    }
  }

  return { added, failed };
}

/**
 * Human-readable toasts for a secure-add outcome. Split success/error so a
 * partial batch reports both "added N" and "M couldn't be secured" — the user
 * must be told when some photos remain (#16), never left thinking all moved.
 */
export function secureAddResultMessage(
  result: SecureAddResult,
  galleryName: string,
): { success?: string; error?: string } {
  const out: { success?: string; error?: string } = {};
  if (result.added > 0) {
    out.success = `Added ${result.added} photo${result.added !== 1 ? "s" : ""} to ${galleryName}`;
  }
  if (result.failed > 0) {
    out.error = `${result.failed} photo${result.failed !== 1 ? "s" : ""} couldn't be secured and remain in your gallery`;
  }
  return out;
}

/**
 * Move one photo into a secure gallery: the server creates an independent
 * encrypted clone under a new blob id, and we mirror the IndexedDB cache entry
 * to that clone so its thumbnail resolves. Returns the new blob id, or `null`
 * when the server produced no clone.
 */
async function addOneToSecureGallery(
  galleryId: string,
  blobId: string,
): Promise<string | null> {
  const response = await api.secureGalleries.addItem(galleryId, blobId);
  if (!response.new_blob_id) return null;

  const originalCached = await db.photos.get(blobId);
  if (originalCached) {
    // Clone the IDB entry under the new blob ID so the secure tile can resolve a
    // thumbnail. Server-side photos get a photos row keyed by the new blob ID;
    // clear storageBlobId so the clone owns its blob.
    await db.photos.put({
      ...originalCached,
      blobId: response.new_blob_id,
      serverPhotoId: originalCached.serverSide
        ? response.new_blob_id
        : originalCached.serverPhotoId,
      storageBlobId: undefined,
    });
  }
  return response.new_blob_id;
}

/**
 * Add a set of photos (by blob ID) to a secure gallery.
 *
 * Each photo is secured independently (see {@link runSecureAddBatch}); the ones
 * that succeed then have their originals removed from the local cache and from
 * every regular album manifest. Cleanup runs on the succeeded set even when part
 * of the batch failed, so a partial move still tidies up what it did secure.
 *
 * Returns how many photos were added vs. left behind.
 */
export async function addPhotosToSecureGallery(
  galleryId: string,
  blobIds: string[],
): Promise<SecureAddResult> {
  // A selected burst representative stands in for its whole stack — pull in the
  // rest of the frames so the entire burst moves into the secure album, not
  // just the cover frame.
  const expanded = await expandBurstSelection(blobIds);

  const { added, failed } = await runSecureAddBatch(expanded, (blobId) =>
    addOneToSecureGallery(galleryId, blobId),
  );

  // Remove the successfully-secured originals from the local cache so they
  // vanish from the main gallery immediately (the server's secureBlobIds
  // endpoint also hides them, but that depends on polling).
  for (const origId of added) {
    await db.photos.delete(origId);
  }

  // A photo moved to a secure album should no longer appear in any regular album.
  await removePhotosFromRegularAlbums(new Set(added));

  return { added: added.length, failed: failed.length };
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

    // Write the manifest with the blob IDs removed (upload-then-delete, so a
    // failure can never leave the album with no manifest at all).
    await saveAlbumManifest({
      ...album,
      photoBlobIds: updated,
      coverPhotoBlobId: cover,
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
