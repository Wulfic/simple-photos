/**
 * The `thumbs` table — every read and write of a cached thumbnail goes here.
 *
 * Thumbnails used to live inline on each `photos` row. IndexedDB deserializes
 * whole records, so that made every query over the mirror — including ones that
 * only wanted an album's member count — structured-clone every thumbnail in the
 * library: hundreds of MB of work to produce an integer. Splitting them out is
 * what makes counting and album-open cheap.
 *
 * Legacy rows keep their bytes in `photo.thumbnailData` until {@link backfillThumbs}
 * moves them, so {@link resolveThumb} looks in both places. Once the backfill has
 * drained, the legacy branch stops matching and the photo rows are lean.
 */
import { db, type CachedPhoto } from "./index";

/** Thumbnail bytes plus the MIME type needed to turn them into a Blob. */
export interface ResolvedThumb {
  data: ArrayBuffer;
  mime: string;
}

/** What a thumbnail should be typed as when nothing recorded it. */
function defaultMime(mediaType?: string): string {
  return mediaType === "gif" ? "image/gif" : "image/jpeg";
}

/**
 * Store a photo's thumbnail. The only way thumbnail bytes enter the cache.
 *
 * Also mirrors the MIME onto the photo row when that row already exists: tiles
 * read it synchronously on first paint (see `CachedPhoto.thumbnailMimeType`).
 * Callers inserting a fresh row set the field themselves and pass
 * `mirrorMime: false`.
 */
export async function putThumb(
  blobId: string,
  data: ArrayBuffer,
  mime?: string,
  mediaType?: string,
  mirrorMime = true,
): Promise<void> {
  const resolved = mime || defaultMime(mediaType);
  await db.thumbs.put({ blobId, data, mime: resolved });
  if (mirrorMime) {
    // `update` no-ops when the row is gone, which is the behaviour we want.
    await db.photos.update(blobId, { thumbnailMimeType: resolved });
  }
}

/**
 * A photo's thumbnail, wherever it currently lives: the `thumbs` table, or —
 * for a row the backfill hasn't reached yet — the legacy inline field.
 *
 * Takes the photo rather than a bare id so the legacy bytes it may already be
 * carrying can be used without a second read.
 */
export async function resolveThumb(
  photo:
    | Pick<CachedPhoto, "blobId" | "thumbnailData" | "thumbnailMimeType" | "mediaType">
    | undefined,
): Promise<ResolvedThumb | undefined> {
  if (!photo) return undefined;
  if (photo.thumbnailData && photo.thumbnailData.byteLength > 0) {
    return {
      data: photo.thumbnailData,
      mime: photo.thumbnailMimeType || defaultMime(photo.mediaType),
    };
  }
  const row = await db.thumbs.get(photo.blobId);
  if (!row) return undefined;
  return { data: row.data, mime: row.mime || defaultMime(photo.mediaType) };
}

/** Like {@link resolveThumb} but keyed only by blobId, for callers holding no row. */
export async function getThumb(blobId: string): Promise<ResolvedThumb | undefined> {
  const row = await db.thumbs.get(blobId);
  if (row) return { data: row.data, mime: row.mime || "image/jpeg" };
  // Not backfilled yet — fall back to the row's inline copy.
  const photo = await db.photos.get(blobId);
  if (photo?.thumbnailData && photo.thumbnailData.byteLength > 0) {
    return {
      data: photo.thumbnailData,
      mime: photo.thumbnailMimeType || defaultMime(photo.mediaType),
    };
  }
  return undefined;
}

/**
 * Give `toBlobId` its own copy of `photo`'s thumbnail, if it has one.
 *
 * Copies are separate rows with separate thumbs entries, so an edit or delete
 * of one never disturbs the other. Call after the copy's photo row exists, so
 * the MIME mirror lands.
 */
export async function copyThumb(
  photo:
    | Pick<CachedPhoto, "blobId" | "thumbnailData" | "thumbnailMimeType" | "mediaType">
    | undefined,
  toBlobId: string,
): Promise<void> {
  const thumb = await resolveThumb(photo);
  if (thumb) await putThumb(toBlobId, thumb.data, thumb.mime);
}

/** Drop thumbnails for photos that no longer exist (trash, delete). */
export async function deleteThumbs(blobIds: string[]): Promise<void> {
  if (blobIds.length === 0) return;
  await db.thumbs.bulkDelete(blobIds);
}

/** How many photo rows are moved per backfill transaction. */
const BACKFILL_CHUNK = 200;

let backfillRunning = false;

/**
 * Move legacy inline thumbnails into the `thumbs` table, a chunk at a time.
 *
 * Chunked and outside the Dexie upgrade on purpose: this can be hundreds of MB,
 * and an upgrade function that size risks stalling app start or — if it throws
 * part-way — leaving a database that won't open at all. Here, each chunk is its
 * own transaction, a failure costs only that chunk, and every row not yet moved
 * still resolves through the legacy branch of {@link resolveThumb}. Interrupting
 * it (closing the tab) is safe; the next run picks up whatever is left.
 *
 * Idempotent, and a no-op once drained.
 */
export async function backfillThumbs(): Promise<number> {
  if (backfillRunning) return 0;
  backfillRunning = true;
  let moved = 0;
  try {
    // Read keys from the index — this does NOT deserialize the rows, so the
    // scan doesn't pay the very cost it exists to eliminate.
    const keys = (await db.photos.toCollection().primaryKeys()) as string[];
    for (let i = 0; i < keys.length; i += BACKFILL_CHUNK) {
      const chunk = keys.slice(i, i + BACKFILL_CHUNK);
      try {
        moved += await db.transaction("rw", db.photos, db.thumbs, async () => {
          const rows = await db.photos.bulkGet(chunk);
          const thumbs: { blobId: string; data: ArrayBuffer; mime: string }[] = [];
          const leaned: CachedPhoto[] = [];
          for (const p of rows) {
            if (!p?.thumbnailData || p.thumbnailData.byteLength === 0) continue;
            const mime = p.thumbnailMimeType || defaultMime(p.mediaType);
            thumbs.push({ blobId: p.blobId, data: p.thumbnailData, mime });
            // Drop the bytes; keep `thumbnailMimeType` — it's a short string the
            // tiles need synchronously (see CachedPhoto.thumbnailMimeType).
            const lean = { ...p, thumbnailMimeType: mime };
            delete lean.thumbnailData;
            leaned.push(lean);
          }
          if (thumbs.length === 0) return 0;
          await db.thumbs.bulkPut(thumbs);
          await db.photos.bulkPut(leaned);
          return thumbs.length;
        });
      } catch (e) {
        // One bad chunk must not abandon the rest: those rows keep their legacy
        // bytes and still render, and the next run retries them.
        console.warn("[thumbs] backfill chunk failed", e);
      }
      // Yield between chunks so a large library doesn't lock up the UI thread.
      await new Promise((r) => setTimeout(r, 0));
    }
  } catch (e) {
    console.warn("[thumbs] backfill could not enumerate photos", e);
  } finally {
    backfillRunning = false;
  }
  return moved;
}

/** Kick off {@link backfillThumbs} without awaiting it. Safe to call repeatedly. */
export function startThumbBackfill(): void {
  void backfillThumbs();
}
