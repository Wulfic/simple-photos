/**
 * Shared soft-delete helpers — move photos to the server trash (30-day
 * recovery window) and mirror them into the local IndexedDB `trash` table so
 * the Trash page can show thumbnails. Centralises the logic previously inlined
 * in Gallery.deleteSelected and useViewerActions.handleDelete so every
 * selection surface (smart albums, people/pets, memories/trips, search) trashes
 * items identically.
 */
import { db } from "../db";
import { api } from "../api/client";

/** Soft-delete a single photo by its IndexedDB blobId. */
export async function trashPhoto(blobId: string): Promise<void> {
  const cached = await db.photos.get(blobId);
  // Copies reference the original's server blob via storageBlobId.
  const storageId = cached?.storageBlobId || blobId;

  let result: { trash_id: string; expires_at: string } | null = null;
  try {
    result = await api.blobs.softDelete(storageId, {
      thumbnail_blob_id: cached?.thumbnailBlobId,
      filename: cached?.filename ?? "unknown",
      mime_type: cached?.mimeType ?? "application/octet-stream",
      media_type: cached?.mediaType,
      size_bytes: 0,
      width: cached?.width,
      height: cached?.height,
      duration_secs: cached?.duration,
      taken_at: cached?.takenAt ? new Date(cached.takenAt).toISOString() : undefined,
    });
  } catch (err) {
    // Blob may already be trashed or missing (e.g. local-only copy, or a
    // server-only synthetic row whose id isn't a real blob). Clean up IDB
    // regardless so the item leaves the grid; only re-throw real failures.
    const isNotFound = err instanceof Error && err.message === "Not found";
    if (!isNotFound) throw err;
  }

  if (cached && result) {
    await db.trash.put({
      trashId: result.trash_id,
      blobId,
      thumbnailBlobId: cached.thumbnailBlobId,
      filename: cached.filename,
      mimeType: cached.mimeType,
      mediaType: cached.mediaType,
      width: cached.width,
      height: cached.height,
      takenAt: cached.takenAt,
      deletedAt: Date.now(),
      expiresAt: result.expires_at,
      thumbnailData: cached.thumbnailData,
      duration: cached.duration,
      albumIds: cached.albumIds ?? [],
    });
  }

  await db.photos.delete(blobId);
}

/** Soft-delete many photos sequentially. Rejects on the first hard failure. */
export async function trashPhotos(blobIds: Iterable<string>): Promise<void> {
  for (const id of blobIds) {
    await trashPhoto(id);
  }
}
