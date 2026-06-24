import { db, type CachedPhoto } from "../../db";

/**
 * Resolve a list of server-side `PhotoSummary` records (returned by the
 * memories / trips endpoints, keyed by the server's `photos.id`) to the
 * client's `CachedPhoto` rows that the gallery components know how to
 * render.
 *
 * Uses a single `toArray()` and a Map lookup — `db.photos.where()` on the
 * `serverPhotoId` index has historically silently failed for users whose
 * photos pre-date the v8 migration (the index exists but rows are missing
 * the field), causing the trips/memories detail pages to render empty even
 * when the card claimed N photos. The Map approach also handles
 * non-encrypted galleries where rows are keyed directly by the server id.
 *
 * Server-only photos (autoscanned but not yet in encrypted-sync) that
 * still have no client-side row are returned as a synthetic display-only
 * `CachedPhoto` so the page is never silently empty — the thumbnail comes
 * from the server's `thumb_path` rather than a local decrypted blob.
 */
export async function resolveServerPhotos(summaries: { id: string; filename: string; thumb_path: string | null; taken_at: string | null }[]): Promise<CachedPhoto[]> {
  const cached = await db.photos.toArray();
  const byServerId = new Map<string, CachedPhoto>();
  const byBlobId = new Map<string, CachedPhoto>();
  for (const p of cached) {
    if (p.serverPhotoId) byServerId.set(p.serverPhotoId, p);
    byBlobId.set(p.blobId, p);
  }
  const found: CachedPhoto[] = [];
  for (const s of summaries) {
    const local = byServerId.get(s.id) ?? byBlobId.get(s.id);
    if (local) {
      found.push(local);
      continue;
    }
    // Synthetic server-side fallback. AlbumTile/JustifiedGrid only need
    // blobId/mediaType/width/height to render; serverSide=true tells
    // ThumbnailTile to fetch via `/api/photos/:id/thumbnail`.
    const synthetic: CachedPhoto = {
      blobId: s.id,
      filename: s.filename,
      takenAt: s.taken_at ? new Date(s.taken_at).getTime() : 0,
      mimeType: "image/jpeg",
      mediaType: "photo",
      width: 0,
      height: 0,
      albumIds: [],
      serverPhotoId: s.id,
      serverSide: true,
    };
    found.push(synthetic);
  }
  return found;
}

/**
 * Resolve a list of server photo ids (e.g. the `photo_id`s returned by the
 * face / pet detection endpoints) to their cached `CachedPhoto` rows, dropping
 * any that have no local row. Shared by the Pets and People detail views, which
 * both walked their detections through the same per-id index lookup.
 */
export async function resolvePhotosByServerId(ids: string[]): Promise<CachedPhoto[]> {
  const found: CachedPhoto[] = [];
  for (const id of ids) {
    const photo = await db.photos.where("serverPhotoId").equals(id).first();
    if (photo) found.push(photo);
  }
  return found;
}
