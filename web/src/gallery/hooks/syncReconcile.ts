/**
 * Reconcile `encrypted-sync` records into the IndexedDB photo mirror.
 *
 * Extracted from `usePhotoSync` so the reconcile is testable as a function
 * rather than only through a hook render — the same reason `ensureThumbCached`
 * lives at module level. The properties that matter here are *counts of IDB
 * operations*, and there is no way to assert those through a rendered hook.
 *
 * ## Why this is staged rather than a loop
 *
 * The version this replaces walked the records one at a time and, per photo,
 * performed a `resolveThumb` read (an IndexedDB `get`) and a `db.photos.update`
 * write, each `await`ed in sequence. On a 10k library that is 10k+ serialized
 * IndexedDB transactions on the main thread on **every** sync pass — the
 * "libraries are slow" report (#38). Correctness was never wrong, so only an
 * operation *count* test catches it; see `syncReconcile.test.ts`.
 *
 * The staging is forced by one hard constraint: **a blob download cannot happen
 * inside a Dexie `rw` transaction.** Awaiting a non-Dexie promise inside one
 * lets the transaction commit early, and the next write throws
 * `TransactionInactiveError`. So each chunk runs three stages in order:
 *
 *   1. **Plan** — resolve every record against in-memory maps. No IO.
 *   2. **Fetch** — one indexed key-scan to learn which thumbnails are already
 *      cached, then download only the genuinely missing ones. No transaction.
 *   3. **Commit** — a single `rw` transaction with one `bulkPut` per table.
 *
 * Chunking (rather than one giant transaction) follows `backfillThumbs`: a
 * failure costs one chunk instead of the whole pass, and the event loop gets a
 * breath between chunks so a large library does not freeze the UI.
 */
import { api } from "../../api/client";
import { decrypt } from "../../crypto/crypto";
import {
  db,
  type CachedPhoto,
  type CachedThumb,
  type MediaType,
  mediaTypeFromMime,
} from "../../db";
import { base64ToArrayBuffer } from "../../utils/media";
import { decodeThumbnailDimensions } from "../utils/thumbnailGenerate";
import {
  isTransposed as checkTransposed,
  correctDimensionsFromThumbnail,
  queueDimensionUpdate,
} from "./useDimensionSync";
import type { ThumbnailPayload } from "../../types/media";

/** One record from `GET /api/photos/encrypted-sync`. */
export type SyncRecord = Awaited<
  ReturnType<typeof api.photos.encryptedSync>
>["photos"][number];

/**
 * Photos reconciled per IndexedDB transaction. Matches `BACKFILL_CHUNK` in
 * `db/thumbs.ts` — same trade-off, same reasoning, so they move together.
 */
export const RECONCILE_CHUNK = 200;

/** What a reconcile pass actually did. Returned for tests and diagnostics. */
export interface ReconcileStats {
  inserted: number;
  updated: number;
  unchanged: number;
  thumbsDownloaded: number;
}

/** A record resolved against the mirror, before any IO. */
interface Plan {
  record: SyncRecord;
  /** The mirror row this record belongs to, or undefined for an insert. */
  existing?: CachedPhoto;
  /** Primary key to write under. */
  idbKey: string;
  /**
   * Server id this row must be bound to, when the row was matched by blob id
   * rather than by `serverPhotoId`.
   *
   * Tracked here instead of being assigned onto `existing` up front. The old
   * code mutated `existing.serverPhotoId` at match time and *then* asked
   * `existing.serverPhotoId !== photo.id` to decide whether to persist it —
   * which its own mutation had just made false. The binding was therefore
   * dropped unless some unrelated field happened to differ in the same pass,
   * leaving locally-uploaded rows with no `serverPhotoId` (and so no favourite
   * toggle, no face-cluster lookup, no duplicate).
   */
  bindServerId?: string;
  /** Thumbnail bytes must be fetched because the server's blob id changed. */
  thumbChanged: boolean;
}

function defaultThumbMime(mediaType?: string | null): string {
  return mediaType === "gif" ? "image/gif" : "image/jpeg";
}

/** Fetch and decode one thumbnail blob. Returns undefined on any failure. */
async function fetchThumb(
  blobId: string,
  mediaType?: string | null,
): Promise<{ data: ArrayBuffer; mime: string } | undefined> {
  try {
    const enc = await api.blobs.download(blobId);
    const dec = await decrypt(enc);
    const payload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(dec));
    return {
      data: base64ToArrayBuffer(payload.data),
      mime: payload.mime_type || defaultThumbMime(mediaType),
    };
  } catch {
    // Leave the cache as-is; a later pass retries. Deliberately not logged per
    // photo: on a cold start with a flaky link this would be thousands of lines.
    return undefined;
  }
}

/**
 * Merge `records` into the local mirror.
 *
 * `cached` is the current contents of `db.photos`, passed in rather than read
 * here so the caller (which already needed it for stale-entry pruning) does not
 * pay for a second full-table read.
 */
export async function reconcileSyncedPhotos(
  records: SyncRecord[],
  cached: CachedPhoto[],
  opts: { chunkSize?: number } = {},
): Promise<ReconcileStats> {
  const chunkSize = opts.chunkSize ?? RECONCILE_CHUNK;
  const stats: ReconcileStats = {
    inserted: 0,
    updated: 0,
    unchanged: 0,
    thumbsDownloaded: 0,
  };

  const idbByServerId = new Map(
    cached.filter((p) => p.serverPhotoId).map((p) => [p.serverPhotoId!, p]),
  );
  const idbByBlobId = new Map(cached.map((p) => [p.blobId, p]));

  // ── Stage 1: plan (pure, no IO) ──────────────────────────────────────────
  const plans: Plan[] = [];
  for (const record of records) {
    // No ciphertext yet — nothing displayable to mirror. The library COUNT
    // still includes these; it comes from the server summary, not from here
    // (#42). Do not "fix" this by inserting a blank row.
    if (!record.encrypted_blob_id) continue;

    let existing = idbByServerId.get(record.id);
    let bindServerId: string | undefined;
    if (!existing) {
      const boundByBlob = idbByBlobId.get(record.encrypted_blob_id);
      if (boundByBlob && !boundByBlob.serverPhotoId) {
        existing = boundByBlob;
        bindServerId = record.id;
        idbByServerId.set(record.id, existing);
      }
    }

    const serverThumbId = record.encrypted_thumb_blob_id ?? undefined;
    plans.push({
      record,
      existing,
      idbKey: existing ? existing.blobId : record.id,
      bindServerId,
      thumbChanged: !!existing && !!serverThumbId && existing.thumbnailBlobId !== serverThumbId,
    });
  }

  for (let i = 0; i < plans.length; i += chunkSize) {
    const chunk = plans.slice(i, i + chunkSize);

    // ── Stage 2a: which thumbnails are already cached? ────────────────────
    // One indexed key-scan for the whole chunk, replacing a `resolveThumb`
    // (i.e. a `db.thumbs.get`) per photo. `primaryKeys()` reads the index only,
    // so this does NOT deserialize the thumbnail bytes — a `bulkGet` here would
    // structured-clone megabytes of image data purely to test for existence.
    const needPresenceCheck = chunk
      .filter(
        (p) =>
          p.existing &&
          !p.thumbChanged &&
          // Legacy inline bytes are already in hand from the row itself.
          !(p.existing.thumbnailData && p.existing.thumbnailData.byteLength > 0),
      )
      .map((p) => p.idbKey);

    const presentThumbs = new Set<string>(
      needPresenceCheck.length > 0
        ? ((await db.thumbs
            .where("blobId")
            .anyOf(needPresenceCheck)
            .primaryKeys()) as string[])
        : [],
    );

    // ── Stage 2b: download the genuinely missing thumbnails ───────────────
    // Outside any transaction, by necessity. In the steady state this loop does
    // nothing at all, which is the property the 5-minute poll depends on.
    const fetched = new Map<string, { data: ArrayBuffer; mime: string }>();
    for (const p of chunk) {
      const { record, existing } = p;
      const serverThumbId = record.encrypted_thumb_blob_id ?? undefined;

      let sourceThumbId: string | undefined;
      if (!existing) {
        sourceThumbId = serverThumbId;
      } else if (p.thumbChanged) {
        sourceThumbId = serverThumbId;
      } else if (!presentThumbs.has(p.idbKey)) {
        sourceThumbId = existing.thumbnailBlobId ?? serverThumbId;
      }
      if (!sourceThumbId) continue;

      const thumb = await fetchThumb(sourceThumbId, record.media_type);
      if (thumb) {
        fetched.set(p.idbKey, thumb);
        stats.thumbsDownloaded++;
      }
    }

    // ── Stage 3: build the rows, then commit them in one transaction ──────
    const photoRows: CachedPhoto[] = [];
    const thumbRows: CachedThumb[] = [];
    // Dimension corrections are a network call; collected here, issued after
    // the transaction so nothing non-Dexie is awaited inside it.
    const dimensionFixes: Array<{ id: string; width: number; height: number }> = [];

    for (const p of chunk) {
      const { record, existing, idbKey } = p;
      const thumb = fetched.get(idbKey);

      if (existing) {
        const updates: Partial<CachedPhoto> = {};
        if (existing.isFavorite !== record.is_favorite) updates.isFavorite = record.is_favorite;
        if (p.bindServerId) updates.serverPhotoId = p.bindServerId;
        else if (existing.serverPhotoId !== record.id) updates.serverPhotoId = record.id;

        const serverBlobIdVal = record.encrypted_blob_id ?? undefined;
        if (serverBlobIdVal && existing.storageBlobId !== serverBlobIdVal)
          updates.storageBlobId = serverBlobIdVal;
        const serverSourcePath = record.source_path ?? undefined;
        if (existing.sourcePath !== serverSourcePath) updates.sourcePath = serverSourcePath;
        const serverSubtype = record.photo_subtype ?? undefined;
        if (existing.photoSubtype !== serverSubtype) updates.photoSubtype = serverSubtype;
        // Backfill addedAt (library import order) for entries cached before the
        // field existed. created_at never changes, so set-once is safe.
        if (existing.addedAt === undefined) {
          const added = new Date(record.created_at).getTime();
          if (!Number.isNaN(added)) updates.addedAt = added;
        }
        const serverBurstId = record.burst_id ?? undefined;
        if (existing.burstId !== serverBurstId) updates.burstId = serverBurstId;
        const serverMotionBlob = record.motion_video_blob_id ?? undefined;
        if (existing.motionVideoBlobId !== serverMotionBlob)
          updates.motionVideoBlobId = serverMotionBlob;
        const serverCrop = record.crop_metadata ?? undefined;
        if (existing.cropData !== serverCrop) updates.cropData = serverCrop;

        // Dimension sync with transpose guard: a rotated thumbnail legitimately
        // reports swapped w/h, and adopting that would fight the EXIF fix.
        if (
          record.width > 0 &&
          record.height > 0 &&
          (existing.width !== record.width || existing.height !== record.height)
        ) {
          if (!checkTransposed(existing.width, existing.height, record.width, record.height)) {
            updates.width = record.width;
            updates.height = record.height;
          }
        }

        if (p.thumbChanged && thumb) {
          updates.thumbnailBlobId = record.encrypted_thumb_blob_id ?? undefined;
        }
        if (thumb) {
          updates.thumbnailMimeType = thumb.mime;
          thumbRows.push({ blobId: idbKey, data: thumb.data, mime: thumb.mime });

          const curW = updates.width ?? existing.width;
          const curH = updates.height ?? existing.height;
          if (curW > 0 && curH > 0) {
            const correction = await correctFromThumb(thumb, curW, curH);
            if (correction) {
              updates.width = correction.width;
              updates.height = correction.height;
            }
          }
        }

        if (Object.keys(updates).length > 0) {
          // Whole-row put rather than a partial `update`: one bulk write beats
          // N patches. Spreading `existing` first preserves client-only fields
          // (notably `albumIds`, which the server record knows nothing about).
          photoRows.push({ ...existing, ...updates });
          stats.updated++;
        } else {
          stats.unchanged++;
        }
        continue;
      }

      // ── New entry ──────────────────────────────────────────────────────
      let takenAt: number;
      try {
        takenAt = record.taken_at
          ? new Date(record.taken_at).getTime()
          : new Date(record.created_at).getTime();
      } catch {
        takenAt = new Date(record.created_at).getTime();
      }

      let displayWidth = record.width;
      let displayHeight = record.height;
      if (thumb && displayWidth > 0 && displayHeight > 0) {
        const correction = await correctFromThumb(thumb, displayWidth, displayHeight);
        if (correction) {
          displayWidth = correction.width;
          displayHeight = correction.height;
          dimensionFixes.push({ id: record.id, width: displayWidth, height: displayHeight });
        }
      }

      photoRows.push({
        blobId: idbKey,
        storageBlobId: record.encrypted_blob_id ?? undefined,
        thumbnailBlobId: record.encrypted_thumb_blob_id ?? undefined,
        filename: record.filename,
        takenAt,
        mimeType: record.mime_type,
        mediaType: (record.media_type as MediaType) ?? mediaTypeFromMime(record.mime_type),
        width: displayWidth,
        height: displayHeight,
        duration: record.duration_secs ?? undefined,
        albumIds: [],
        contentHash: record.photo_hash ?? undefined,
        cropData: record.crop_metadata ?? undefined,
        isFavorite: record.is_favorite ?? false,
        serverPhotoId: record.id,
        sourcePath: record.source_path ?? undefined,
        addedAt: (() => {
          const added = new Date(record.created_at).getTime();
          return Number.isNaN(added) ? takenAt : added;
        })(),
        photoSubtype: record.photo_subtype ?? undefined,
        burstId: record.burst_id ?? undefined,
        motionVideoBlobId: record.motion_video_blob_id ?? undefined,
        ...(thumb ? { thumbnailMimeType: thumb.mime } : {}),
      });
      if (thumb) thumbRows.push({ blobId: idbKey, data: thumb.data, mime: thumb.mime });
      stats.inserted++;
    }

    if (photoRows.length > 0 || thumbRows.length > 0) {
      try {
        await db.transaction("rw", db.photos, db.thumbs, async () => {
          if (photoRows.length > 0) await db.photos.bulkPut(photoRows);
          if (thumbRows.length > 0) await db.thumbs.bulkPut(thumbRows);
        });
      } catch (e) {
        // One bad chunk must not abandon the rest of the library: these rows
        // keep their previous state and the next pass retries them. Logged
        // once per chunk, not per photo.
        console.warn("[sync] reconcile chunk failed to commit", e);
      }
    }

    for (const fix of dimensionFixes) queueDimensionUpdate(fix.id, fix.width, fix.height);

    // Yield between chunks so a large library does not lock up the UI thread.
    if (i + chunkSize < plans.length) await new Promise((r) => setTimeout(r, 0));
  }

  return stats;
}

/**
 * Heal server dimensions that disagree with the thumbnail's real aspect ratio.
 * Returns undefined when the thumbnail cannot be decoded (no DOM decoder, or
 * corrupt bytes) — in which case the server's numbers stand.
 */
async function correctFromThumb(
  thumb: { data: ArrayBuffer; mime: string },
  width: number,
  height: number,
): Promise<{ width: number; height: number } | undefined> {
  try {
    const dims = await decodeThumbnailDimensions(thumb.data, thumb.mime);
    return correctDimensionsFromThumbnail(dims.width, dims.height, width, height) ?? undefined;
  } catch {
    return undefined;
  }
}
