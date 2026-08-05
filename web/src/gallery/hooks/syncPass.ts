/**
 * One server→IndexedDB sync pass (#38).
 *
 * Extracted from `usePhotoSync` for the same reason `syncReconcile` was: the
 * properties that matter here are *how much work a pass does*, and there is no
 * way to assert those through a rendered hook. See `syncPass.test.ts`.
 *
 * ## Three modes, and why the cheap one needs the expensive one
 *
 * | mode      | when                          | cost                          |
 * |-----------|-------------------------------|-------------------------------|
 * | `skipped` | `head_seq` == persisted cursor | one small JSON request. No pagination, no IDB writes, no blob downloads. |
 * | `delta`   | cursor known, head moved       | O(changed) rows and writes.   |
 * | `full`    | no cursor, or server too old   | the historical full walk.     |
 *
 * `skipped` is the steady state and the entire point of the issue: a library
 * that has not changed must cost nothing to re-sync. Before this, every
 * five-minute tick paged the whole photo table, then enumerated all four blob
 * types in full, on a library that had not moved.
 *
 * The full walk is not legacy baggage — it is the recovery path. It is
 * *self-healing*: it re-sends everything and the client set-differences, so any
 * local damage is repaired on the next pass. A delta feed has no such property
 * (it only ever names what changed after N), which is why every uncertainty in
 * this file resolves to "do a full walk" rather than "assume the cursor is
 * fine". A needless full walk costs one slow pass; a wrongly-trusted cursor
 * costs rows that no future response will ever mention again.
 *
 * ## What `delta` deliberately does NOT do
 *
 * Delta passes skip the four `fetchAllPages` blob walks that the full pass runs
 * (Phase 2/4 below), because those are O(library) per pass and would defeat the
 * whole exercise. The change-log triggers cover `photos` and
 * `encrypted_gallery_items` only, so a blob uploaded with **no `photos` row at
 * all** does not move `head_seq` and is therefore invisible to a delta pass.
 *
 * That state is a failed registration, not a normal one: the web client only
 * ever uploads `album_manifest` blobs directly, and Android always follows
 * `uploadBlob` with `registerEncryptedPhoto`, which inserts the `photos` row and
 * fires the trigger. So the window is "upload succeeded, registration did not",
 * and it is repaired by the next full pass — i.e. the next cold start, since a
 * fresh session has no cursor. The alternative, keeping four full library walks
 * on every tick forever to cover it, is not a trade worth making. Written down
 * because it is a real (if narrow) behaviour change, and an undocumented one
 * would be indistinguishable from a bug in six months.
 */
import { api } from "../../api/client";
import { decrypt } from "../../crypto/crypto";
import { decryptBlobMetadata } from "../../crypto/blobEnvelope";
import { db, type CachedPhoto, type CachedThumb, mediaTypeFromMime } from "../../db";
import { base64ToArrayBuffer } from "../../utils/media";
import { fetchAllPages } from "../../utils/gallery";
import { readSyncCursor, writeSyncCursor } from "./syncCursor";
import {
  reconcileSyncedPhotos,
  RECONCILE_CHUNK,
  type ReconcileStats,
  type SyncRecord,
} from "./syncReconcile";
import type { PhotoPayload, ThumbnailPayload } from "../../types/media";

/** Rows requested per `encrypted-sync` page. */
const PAGE_LIMIT = 500;

export type SyncMode = "skipped" | "delta" | "full";

/** What a pass actually did. Returned for tests and diagnostics — the whole
 *  regression surface of #38 is expressed in these numbers. */
export interface SyncPassStats {
  mode: SyncMode;
  /** Photo records received from the server. Zero in `skipped`. */
  photosReceived: number;
  /** Mirror rows dropped, whether by tombstone (`delta`) or set difference
   *  (`full`). */
  rowsRemoved: number;
  /** Undefined when no reconcile ran. */
  reconcile?: ReconcileStats;
}

export interface SyncPassOptions {
  /**
   * Called once the mirror is safe to present — after pruning, before the
   * (potentially long) reconcile. Drives `encryptedDataReady` in the hook.
   */
  onDataReady?: () => void;
}

/**
 * Bring the local mirror up to date with the server, by the cheapest route
 * that is provably correct.
 */
export async function runSyncPass(opts: SyncPassOptions = {}): Promise<SyncPassStats> {
  const cursor = await readSyncCursor();
  const head = await fetchHeadSeq();

  // ── Fast path ────────────────────────────────────────────────────────────
  // Nothing has changed since we last synced. Note this is only reachable with
  // a cursor that survived `readSyncCursor`'s coherence guard, and with a head
  // the server served *outside* its TTL cache — a stale head here would skip
  // real changes, which is why the server computes it fresh.
  if (cursor !== null && head !== undefined && head === cursor) {
    opts.onDataReady?.();
    return { mode: "skipped", photosReceived: 0, rowsRemoved: 0 };
  }

  if (cursor !== null) {
    const delta = await runDeltaPass(cursor, opts);
    // `null` means the server did not honour `since` — fall through and take
    // the full walk rather than misreading its answer as a delta.
    if (delta) return delta;
  }

  return runFullPass(opts);
}

/** Current change-log head, or undefined if it cannot be established. */
async function fetchHeadSeq(): Promise<number | undefined> {
  try {
    const summary = await api.photos.summary();
    return typeof summary.head_seq === "number" ? summary.head_seq : undefined;
  } catch (e) {
    // Non-fatal: we lose the shortcut, not correctness. A pass still runs.
    console.warn("[sync] could not read head_seq; syncing without the fast path", e);
    return undefined;
  }
}

// ── Delta ──────────────────────────────────────────────────────────────────

/**
 * Apply everything that changed after `since`.
 *
 * Returns `null` — meaning "caller must fall back to a full walk" — when the
 * server answers without a `deleted` array. That is the protocol handshake: a
 * server predating #38 ignores the unknown `since` parameter and replies with a
 * *full walk*, whose `photos` are indistinguishable from a delta's. Treating
 * that as a delta would mean pruning nothing while believing we had pruned
 * correctly, and then persisting a cursor that makes the mistake permanent.
 */
async function runDeltaPass(
  since: number,
  opts: SyncPassOptions,
): Promise<SyncPassStats | null> {
  const records: SyncRecord[] = [];
  const tombstones: string[] = [];
  let headAtStart: number | undefined;
  let after: string | undefined;

  do {
    const res = await api.photos.encryptedSync({ after, limit: PAGE_LIMIT, since });
    if (res.deleted === undefined) return null;

    // Keep the FIRST page's head, not the last. A change committed while this
    // multi-page walk is in flight lands at a sequence above the first page's
    // head; keeping the first head re-delivers it on the next pass, whereas
    // keeping the last would step over it and lose it permanently.
    if (headAtStart === undefined) headAtStart = res.head_seq;

    records.push(...res.photos);
    tombstones.push(...res.deleted);
    after = res.next_cursor ?? undefined;
  } while (after);

  const rowsRemoved = await applyTombstones(tombstones);

  opts.onDataReady?.();

  // Only the rows this delta could possibly touch are read — NOT the whole
  // mirror. The full pass can afford a `toArray()` because it needs every row
  // to compute a set difference; a delta pass that did the same would
  // deserialize the entire library on every change, which is most of what made
  // syncing slow in the first place.
  const cached = records.length > 0 ? await loadAffectedRows(records) : [];
  const reconcile =
    records.length > 0 ? await reconcileSyncedPhotos(records, cached) : undefined;

  // Persist only now, once the mirror actually reflects this sequence.
  if (headAtStart !== undefined) await writeSyncCursor(headAtStart);

  return { mode: "delta", photosReceived: records.length, rowsRemoved, reconcile };
}

/**
 * Load exactly the mirror rows `records` might match, using the same two
 * lookups `reconcileSyncedPhotos` performs (`serverPhotoId`, then primary key).
 *
 * `reconcileSyncedPhotos` builds its maps from whatever it is handed and never
 * assumes the array is the complete mirror, so a bounded subset is safe — it
 * just has to be a superset of the rows that could match.
 */
async function loadAffectedRows(records: SyncRecord[]): Promise<CachedPhoto[]> {
  const serverIds = records.map((r) => r.id);
  // A row inserted by a previous sync is keyed by the server photo id; a row
  // from a local upload is keyed by its blob id. Both are candidate keys.
  const primaryKeys = new Set<string>(serverIds);
  for (const r of records) if (r.encrypted_blob_id) primaryKeys.add(r.encrypted_blob_id);

  const [byServerId, byKey] = await Promise.all([
    db.photos.where("serverPhotoId").anyOf(serverIds).toArray(),
    db.photos.bulkGet([...primaryKeys]),
  ]);

  const merged = new Map<string, CachedPhoto>();
  for (const row of byServerId) merged.set(row.blobId, row);
  for (const row of byKey) if (row) merged.set(row.blobId, row);
  return [...merged.values()];
}

/**
 * Drop rows the server says have left the feed — deleted outright, or claimed
 * by a secure gallery. Both mean the same thing locally.
 *
 * A tombstone names a *photo id*, which may be the mirror row's primary key
 * (rows this sync inserted) or its `serverPhotoId` (rows bound to a local
 * upload's blob id). Both are resolved; missing ids are simply not deleted.
 */
async function applyTombstones(photoIds: string[]): Promise<number> {
  if (photoIds.length === 0) return 0;

  const [direct, viaServerId] = await Promise.all([
    db.photos.where(":id").anyOf(photoIds).primaryKeys() as Promise<string[]>,
    db.photos.where("serverPhotoId").anyOf(photoIds).primaryKeys() as Promise<string[]>,
  ]);
  const victims = [...new Set([...direct, ...viaServerId])];
  if (victims.length === 0) return 0;

  try {
    await db.transaction("rw", db.photos, db.thumbs, async () => {
      await db.photos.bulkDelete(victims);
      // Thumbnails are decrypted image bytes keyed by the same id. Leaving them
      // behind leaks the visible content of a photo the user deleted, and grows
      // without bound. The pre-#38 prune did not do this.
      await db.thumbs.bulkDelete(victims);
    });
  } catch (e) {
    // Next pass retries: the cursor is only advanced after this returns, and a
    // throw here propagates out of the pass before that happens.
    console.warn("[sync] failed to apply tombstones", e);
    throw e;
  }
  return victims.length;
}

// ── Full walk ──────────────────────────────────────────────────────────────

/**
 * The historical behaviour: page the entire eligible library, enumerate every
 * encrypted blob, set-difference the result against the mirror.
 *
 * Expensive and self-healing, in that order. This runs on cold start, after a
 * cursor is discarded, and against any server that does not speak `since`.
 */
async function runFullPass(opts: SyncPassOptions): Promise<SyncPassStats> {
  // Phase 1: Fetch metadata via encrypted-sync endpoint.
  const allSyncPhotos: SyncRecord[] = [];
  let headAtStart: number | undefined;
  let cursor: string | undefined;
  do {
    const res = await api.photos.encryptedSync({ after: cursor, limit: PAGE_LIMIT });
    if (headAtStart === undefined) headAtStart = res.head_seq;
    allSyncPhotos.push(...res.photos);
    cursor = res.next_cursor ?? undefined;
  } while (cursor);

  const serverPhotoIds = new Set<string>();
  const serverBlobIds = new Set<string>();
  for (const p of allSyncPhotos) {
    serverPhotoIds.add(p.id);
    if (p.encrypted_blob_id) serverBlobIds.add(p.encrypted_blob_id);
  }

  // Phase 2: Include directly-uploaded encrypted blobs.
  const allBlobMedia = [
    ...(await fetchAllPages("photo")),
    ...(await fetchAllPages("gif")),
    ...(await fetchAllPages("video")),
    ...(await fetchAllPages("audio")),
  ];
  for (const blob of allBlobMedia) serverBlobIds.add(blob.id);

  // Remove stale IDB entries
  const currentCached = await db.photos.toArray();
  const staleIds = [
    ...new Set(
      currentCached
        .filter((p) => {
          if (p.serverPhotoId) return !serverPhotoIds.has(p.serverPhotoId);
          const underlyingId = p.storageBlobId || p.blobId;
          return !serverBlobIds.has(underlyingId) && !serverPhotoIds.has(underlyingId);
        })
        .map((p) => p.blobId),
    ),
  ];
  if (staleIds.length > 0) {
    await db.transaction("rw", db.photos, db.thumbs, async () => {
      await db.photos.bulkDelete(staleIds);
      // See applyTombstones: same reasoning, same leak.
      await db.thumbs.bulkDelete(staleIds);
    });
  }

  opts.onDataReady?.();

  // Derived, not re-read: the previous version issued a second full
  // `toArray()` here purely to see the effect of the delete it had just
  // performed. On a large library that is a second full deserialization of
  // the mirror for information already in hand.
  const staleSet = new Set(staleIds);
  const survivingCached = currentCached.filter((p) => !staleSet.has(p.blobId));

  // Phase 3: Populate IDB from sync records.
  //
  // Batched, chunked and staged — see `syncReconcile.ts`. This was a
  // per-photo loop with an awaited IndexedDB read *and* write inside it, so
  // a 10k library meant 10k+ serialized transactions on the main thread
  // every pass. That is the bulk of "photo libraries are slow" (#38).
  const reconcile = await reconcileSyncedPhotos(allSyncPhotos, survivingCached);

  await syncDirectBlobs(allSyncPhotos, allBlobMedia);

  // A full walk brings the mirror to the head observed before it started, so
  // the next pass can be a delta. Skipped when the server did not report a
  // head (pre-#38 binary), which keeps this client on full walks — correct,
  // just not fast.
  if (headAtStart !== undefined) await writeSyncCursor(headAtStart);

  return {
    mode: "full",
    photosReceived: allSyncPhotos.length,
    rowsRemoved: staleIds.length,
    reconcile,
  };
}

/**
 * Phase 4: encrypted blobs that exist server-side with no corresponding
 * `photos` row — a client upload whose registration did not complete.
 */
async function syncDirectBlobs(
  allSyncPhotos: SyncRecord[],
  allBlobMedia: Awaited<ReturnType<typeof fetchAllPages>>,
): Promise<void> {
  const syncedBlobIds = new Set(
    allSyncPhotos.map((p) => p.encrypted_blob_id).filter((id): id is string => !!id),
  );
  // Membership is answered by ONE indexed key scan rather than a
  // `db.photos.get()` per blob as this previously did. `primaryKeys()` reads
  // the index without deserializing rows, and — unlike the pre-reconcile
  // snapshot — it includes anything Phase 3 just inserted. That matters:
  // `photos.id` and blob ids share a namespace on client-upload paths (hence
  // the `id NOT IN (SELECT blob_id ...)` guard in the sync query), so a
  // stale snapshot here could re-insert a row Phase 3 had just written.
  const cachedBlobIds = new Set((await db.photos.toCollection().primaryKeys()) as string[]);
  const unsyncedBlobs = allBlobMedia.filter(
    (b) => !syncedBlobIds.has(b.id) && !cachedBlobIds.has(b.id),
  );

  const directPhotos: CachedPhoto[] = [];
  const directThumbs: CachedThumb[] = [];

  /** Commit what has accumulated so far, then clear the buffers. */
  const flushDirect = async () => {
    if (directPhotos.length === 0 && directThumbs.length === 0) return;
    const photoRows = directPhotos.splice(0);
    const thumbRows = directThumbs.splice(0);
    try {
      await db.transaction("rw", db.photos, db.thumbs, async () => {
        if (photoRows.length > 0) await db.photos.bulkPut(photoRows);
        if (thumbRows.length > 0) await db.thumbs.bulkPut(thumbRows);
      });
    } catch (e) {
      // One bad chunk must not abandon the rest; the next pass retries it.
      console.warn("[sync] direct-blob chunk failed to commit", e);
    }
  };

  for (const blob of unsyncedBlobs) {
    try {
      const encrypted = await api.blobs.download(blob.id);
      // Only metadata is needed here — decryptBlobMetadata avoids
      // reconstructing media bytes (and, for v2, avoids decrypting every
      // chunk frame just to read fields). Handles both blob formats.
      const payload = await decryptBlobMetadata<PhotoPayload>(encrypted);

      let thumbnailData: ArrayBuffer | undefined;
      let unsyncedThumbMime: string | undefined;
      if (payload.thumbnail_blob_id) {
        try {
          const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
          const thumbDec = await decrypt(thumbEnc);
          const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
          thumbnailData = base64ToArrayBuffer(thumbPayload.data);
          unsyncedThumbMime = thumbPayload.mime_type;
        } catch {
          /* placeholder */
        }
      }

      const thumbMime =
        unsyncedThumbMime || (payload.media_type === "gif" ? "image/gif" : "image/jpeg");
      directPhotos.push({
        blobId: blob.id,
        thumbnailBlobId: payload.thumbnail_blob_id,
        filename: payload.filename,
        takenAt: new Date(payload.taken_at).getTime(),
        mimeType: payload.mime_type,
        mediaType: payload.media_type ?? mediaTypeFromMime(payload.mime_type),
        width: payload.width,
        height: payload.height,
        duration: payload.duration,
        latitude: payload.latitude,
        longitude: payload.longitude,
        albumIds: payload.album_ids ?? [],
        contentHash: blob.content_hash ?? undefined,
        ...(thumbnailData ? { thumbnailMimeType: thumbMime } : {}),
      });
      if (thumbnailData) {
        directThumbs.push({ blobId: blob.id, data: thumbnailData, mime: thumbMime });
      }
      if (directPhotos.length >= RECONCILE_CHUNK) await flushDirect();
    } catch {
      // Skip undecryptable items
    }
  }
  await flushDirect();
}
