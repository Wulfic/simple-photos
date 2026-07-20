// Real IndexedDB (in-memory) so the operation counts below are counts of
// *actual* Dexie work, not of a mock's method calls — same reasoning as
// syncReconcile.test.ts, and the same reason it is worth the setup cost.
import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("../../api/client", () => ({
  api: {
    blobs: { download: vi.fn(), list: vi.fn() },
    photos: { encryptedSync: vi.fn(), summary: vi.fn() },
  },
}));
vi.mock("../../crypto/crypto", () => ({
  decrypt: vi.fn(async (buf: ArrayBuffer) => buf),
}));
vi.mock("../../crypto/blobEnvelope", () => ({
  decryptBlobMetadata: vi.fn(async () => {
    throw new Error("no direct blobs in these tests");
  }),
}));
// Dimension healing needs a DOM image decoder, which this repo has no jsdom for.
// It is not what these tests are about.
vi.mock("../utils/thumbnailGenerate", () => ({
  decodeThumbnailDimensions: vi.fn(async () => {
    throw new Error("no decoder in tests");
  }),
}));

import { runSyncPass } from "./syncPass";
import { readSyncCursor, writeSyncCursor, SYNC_CURSOR_KEY } from "./syncCursor";
import { api } from "../../api/client";
import { db, clearAllUserData, type CachedPhoto } from "../../db";
import type { SyncRecord } from "./syncReconcile";

const encryptedSync = vi.mocked(api.photos.encryptedSync);
const summary = vi.mocked(api.photos.summary);
const blobList = vi.mocked(api.blobs.list);
const blobDownload = vi.mocked(api.blobs.download);

/** A server sync record. Defaults describe an ordinary encrypted photo. */
function record(id: string, over: Partial<SyncRecord> = {}): SyncRecord {
  return {
    id,
    filename: `${id}.jpg`,
    mime_type: "image/jpeg",
    media_type: "photo",
    size_bytes: 100,
    width: 40,
    height: 30,
    duration_secs: null,
    taken_at: "2026-01-01T00:00:00Z",
    created_at: "2026-01-01T00:00:00Z",
    encrypted_blob_id: `${id}-blob`,
    encrypted_thumb_blob_id: `${id}-thumb`,
    is_favorite: false,
    crop_metadata: null,
    photo_hash: null,
    source_path: null,
    photo_subtype: null,
    burst_id: null,
    motion_video_blob_id: null,
    ...over,
  } as SyncRecord;
}

/** The mirror row a previous successful sync of `record(id)` would have left. */
function cachedFor(id: string, over: Partial<CachedPhoto> = {}): CachedPhoto {
  return {
    blobId: id,
    serverPhotoId: id,
    storageBlobId: `${id}-blob`,
    thumbnailBlobId: `${id}-thumb`,
    thumbnailMimeType: "image/jpeg",
    filename: `${id}.jpg`,
    takenAt: Date.parse("2026-01-01T00:00:00Z"),
    addedAt: Date.parse("2026-01-01T00:00:00Z"),
    mimeType: "image/jpeg",
    mediaType: "photo",
    width: 40,
    height: 30,
    albumIds: [],
    isFavorite: false,
    ...over,
  };
}

function id(i: number): string {
  return `p${String(i).padStart(5, "0")}`;
}

/** Seed a fully-synced mirror of `n` photos, already current as of `seq`. */
async function seedSyncedLibrary(n: number, seq: number): Promise<void> {
  const rows: CachedPhoto[] = [];
  const thumbs = [];
  for (let i = 0; i < n; i++) {
    rows.push(cachedFor(id(i)));
    thumbs.push({
      blobId: id(i),
      data: new Uint8Array([1, 2, 3]).buffer as ArrayBuffer,
      mime: "image/jpeg",
    });
  }
  await db.photos.bulkPut(rows);
  await db.thumbs.bulkPut(thumbs);
  await writeSyncCursor(seq);
}

/** One-page delta response. */
function deltaPage(photos: SyncRecord[], deleted: string[], head: number) {
  return { photos, deleted, next_cursor: null, head_seq: head };
}

beforeEach(async () => {
  await Promise.all([db.photos.clear(), db.thumbs.clear(), db.syncState.clear()]);
  vi.restoreAllMocks();
  encryptedSync.mockReset();
  summary.mockReset();
  blobList.mockReset();
  blobDownload.mockReset();
  // No directly-uploaded blobs unless a test says otherwise.
  blobList.mockResolvedValue({ blobs: [], next_cursor: null } as never);
});

/**
 * The #38 steady-state gate.
 *
 * This is the property the whole issue is about, and it is invisible to a
 * correctness test: the pre-#38 client produced exactly the right mirror on
 * every tick — it just rebuilt the entire library to do it. Only counting the
 * work catches a silent regression back to full-walking, which is the same
 * lesson `f31b27b` learned about the reconcile.
 */
describe("runSyncPass — an unchanged library costs nothing", () => {
  it("transfers zero rows and performs zero IndexedDB writes when head_seq has not moved", async () => {
    await seedSyncedLibrary(10_000, 4242);
    summary.mockResolvedValue({ head_seq: 4242 } as never);

    const photosBulkPut = vi.spyOn(db.photos, "bulkPut");
    const photosBulkDelete = vi.spyOn(db.photos, "bulkDelete");
    const thumbsBulkPut = vi.spyOn(db.thumbs, "bulkPut");
    // The full-library deserialization the old pass did on every tick.
    const photosToArray = vi.spyOn(db.photos, "toArray");

    const stats = await runSyncPass();

    expect(stats.mode).toBe("skipped");
    expect(stats.photosReceived).toBe(0);
    expect(stats.rowsRemoved).toBe(0);

    // Zero rows transferred: the sync endpoint is never even called.
    expect(encryptedSync).not.toHaveBeenCalled();
    // Zero blob work: no library enumeration, no thumbnail downloads.
    expect(blobList).not.toHaveBeenCalled();
    expect(blobDownload).not.toHaveBeenCalled();
    // Zero IndexedDB writes, and no full-table read.
    expect(photosBulkPut).not.toHaveBeenCalled();
    expect(photosBulkDelete).not.toHaveBeenCalled();
    expect(thumbsBulkPut).not.toHaveBeenCalled();
    expect(photosToArray).not.toHaveBeenCalled();

    // And the mirror is untouched.
    expect(await db.photos.count()).toBe(10_000);
  });

  it("still reports the mirror as ready so the UI does not hang on a skipped pass", async () => {
    await seedSyncedLibrary(5, 7);
    summary.mockResolvedValue({ head_seq: 7 } as never);

    const onDataReady = vi.fn();
    await runSyncPass({ onDataReady });

    expect(onDataReady).toHaveBeenCalledTimes(1);
  });
});

describe("runSyncPass — delta mode", () => {
  it("requests only what changed and does not read the whole mirror", async () => {
    await seedSyncedLibrary(10_000, 100);
    summary.mockResolvedValue({ head_seq: 101 } as never);
    encryptedSync.mockResolvedValue(
      deltaPage([record(id(0), { is_favorite: true })], [], 101) as never,
    );

    // A delta that deserializes all 10k rows to update one of them has missed
    // the point as thoroughly as the full walk it replaced.
    const photosToArray = vi.spyOn(db.photos, "toArray");

    const stats = await runSyncPass();

    expect(stats.mode).toBe("delta");
    expect(stats.photosReceived).toBe(1);
    expect(encryptedSync).toHaveBeenCalledTimes(1);
    expect(encryptedSync).toHaveBeenCalledWith(
      expect.objectContaining({ since: 100 }),
    );
    expect(photosToArray).not.toHaveBeenCalled();
    // Zero blob-list enumeration — that is the O(library) cost delta mode drops.
    expect(blobList).not.toHaveBeenCalled();

    expect((await db.photos.get(id(0)))?.isFavorite).toBe(true);
    expect(await db.photos.count()).toBe(10_000);
    expect(await readSyncCursor()).toBe(101);
  });

  it("applies tombstones, including the row's decrypted thumbnail bytes", async () => {
    await seedSyncedLibrary(5, 10);
    summary.mockResolvedValue({ head_seq: 11 } as never);
    encryptedSync.mockResolvedValue(deltaPage([], [id(2)], 11) as never);

    const stats = await runSyncPass();

    expect(stats.mode).toBe("delta");
    expect(stats.rowsRemoved).toBe(1);
    expect(await db.photos.get(id(2))).toBeUndefined();
    // Decrypted image bytes for a deleted photo must not survive the delete.
    expect(await db.thumbs.get(id(2))).toBeUndefined();
    expect(await db.photos.count()).toBe(4);
  });

  it("tombstones a row that is keyed by its blob id rather than the photo id", async () => {
    // A locally-uploaded row: primary key is the blob id, the server photo id
    // lives in `serverPhotoId`. A tombstone names the photo id, so resolving
    // only by primary key would silently leave this row behind forever.
    await db.photos.put(cachedFor("local-blob-1", { serverPhotoId: "srv-1" }));
    await db.thumbs.put({
      blobId: "local-blob-1",
      data: new Uint8Array([9]).buffer as ArrayBuffer,
      mime: "image/jpeg",
    });
    await writeSyncCursor(20);
    summary.mockResolvedValue({ head_seq: 21 } as never);
    encryptedSync.mockResolvedValue(deltaPage([], ["srv-1"], 21) as never);

    const stats = await runSyncPass();

    expect(stats.rowsRemoved).toBe(1);
    expect(await db.photos.count()).toBe(0);
    expect(await db.thumbs.get("local-blob-1")).toBeUndefined();
  });

  it("persists the FIRST page's head, so a change committed mid-walk is not stepped over", async () => {
    // Page 2 reports a head that moved while the walk was in flight. Adopting
    // it would mark those changes as already-applied and lose them permanently;
    // keeping page 1's head re-delivers them on the next pass.
    await seedSyncedLibrary(3, 50);
    summary.mockResolvedValue({ head_seq: 51 } as never);
    encryptedSync
      .mockResolvedValueOnce({
        photos: [record(id(0))],
        deleted: [],
        next_cursor: "51|p00000",
        head_seq: 51,
      } as never)
      .mockResolvedValueOnce(deltaPage([record(id(1))], [], 99) as never);

    await runSyncPass();

    expect(encryptedSync).toHaveBeenCalledTimes(2);
    expect(await readSyncCursor()).toBe(51);
  });

  it("leaves the cursor untouched when applying the delta throws", async () => {
    await seedSyncedLibrary(3, 60);
    summary.mockResolvedValue({ head_seq: 61 } as never);
    encryptedSync.mockRejectedValue(new Error("network died"));

    await expect(runSyncPass()).rejects.toThrow("network died");

    // A cursor advanced past changes we failed to apply is the one failure this
    // design cannot recover from — the rows are never mentioned again.
    expect(await readSyncCursor()).toBe(60);
  });
});

describe("runSyncPass — falling back to the self-healing full walk", () => {
  it("full-walks on a cold start and records the resulting cursor", async () => {
    summary.mockResolvedValue({ head_seq: 500 } as never);
    encryptedSync.mockResolvedValue({
      photos: [record(id(0)), record(id(1))],
      next_cursor: null,
      head_seq: 500,
    } as never);
    blobDownload.mockRejectedValue(new Error("no thumbs in this test"));

    const stats = await runSyncPass();

    expect(stats.mode).toBe("full");
    expect(encryptedSync).toHaveBeenCalledWith(
      expect.not.objectContaining({ since: expect.anything() }),
    );
    expect(await db.photos.count()).toBe(2);
    expect(await readSyncCursor()).toBe(500);
  });

  it("treats a response with no `deleted` array as a server that cannot do deltas", async () => {
    // The protocol handshake. A server predating #38 ignores `since` and
    // answers with a FULL walk, whose `photos` look exactly like a delta's.
    // Reading that as a delta would prune nothing while believing it had, and
    // then persist a cursor that makes the mistake permanent.
    await seedSyncedLibrary(3, 70);
    summary.mockResolvedValue({ head_seq: 71 } as never);
    encryptedSync.mockResolvedValue({
      photos: [record(id(0)), record(id(1)), record(id(2))],
      next_cursor: null,
      head_seq: 71,
    } as never);
    blobDownload.mockRejectedValue(new Error("no thumbs in this test"));

    const stats = await runSyncPass();

    expect(stats.mode).toBe("full");
    // It must actually re-walk, not just relabel: the blob enumeration only
    // happens on the full path.
    expect(blobList).toHaveBeenCalled();
  });

  it("discards a cursor left over an empty mirror instead of syncing nothing", async () => {
    // The incoherent state co-location cannot rule out: `photos` wiped (storage
    // eviction, devtools, a partial clear) while the cursor survived. Trusting
    // it would ask for changes after N, get none, and show an empty gallery
    // forever.
    await writeSyncCursor(900);
    expect(await db.photos.count()).toBe(0);

    summary.mockResolvedValue({ head_seq: 900 } as never);
    encryptedSync.mockResolvedValue({
      photos: [record(id(0))],
      next_cursor: null,
      head_seq: 900,
    } as never);
    blobDownload.mockRejectedValue(new Error("no thumbs in this test"));

    const stats = await runSyncPass();

    // Note the head MATCHES the stale cursor, so the fast path would have
    // skipped outright had the guard not fired first.
    expect(stats.mode).toBe("full");
    expect(await db.photos.count()).toBe(1);
  });

  it("syncs normally when head_seq cannot be read at all", async () => {
    await seedSyncedLibrary(2, 80);
    summary.mockRejectedValue(new Error("summary endpoint down"));
    encryptedSync.mockResolvedValue(deltaPage([], [], 80) as never);

    const stats = await runSyncPass();

    // No head to compare against means no shortcut — but the pass still runs
    // rather than assuming nothing changed.
    expect(stats.mode).toBe("delta");
    expect(encryptedSync).toHaveBeenCalled();
  });

  it("stays on full walks against a server that reports no head at all", async () => {
    summary.mockResolvedValue({} as never);
    encryptedSync.mockResolvedValue({
      photos: [record(id(0))],
      next_cursor: null,
      head_seq: undefined,
    } as never);
    blobDownload.mockRejectedValue(new Error("no thumbs in this test"));

    const stats = await runSyncPass();

    expect(stats.mode).toBe("full");
    // No cursor persisted, so the next pass full-walks too: correct, just slow.
    expect(await readSyncCursor()).toBeNull();
  });
});

describe("the delta cursor's lifetime is tied to the mirror it describes", () => {
  it("is destroyed by clearAllUserData", async () => {
    await seedSyncedLibrary(3, 123);
    expect(await db.syncState.get(SYNC_CURSOR_KEY)).toBeDefined();

    await clearAllUserData();

    // Surviving a logout would tell the NEXT user's first sync that the empty
    // mirror we just created is already up to date.
    expect(await db.syncState.get(SYNC_CURSOR_KEY)).toBeUndefined();
    expect(await readSyncCursor()).toBeNull();
  });
});
