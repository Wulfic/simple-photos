// Real IndexedDB (in-memory) so the operation counts below are counts of *actual*
// Dexie work, not of a mock's method calls. The entire point of this suite is
// how many round-trips the reconcile makes, which a stubbed db would not prove.
import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("../../api/client", () => ({
  api: { blobs: { download: vi.fn() }, photos: {} },
}));
vi.mock("../../crypto/crypto", () => ({
  decrypt: vi.fn(async (buf: ArrayBuffer) => buf),
}));
// Thumbnail decoding needs a DOM image decoder; the reconcile only uses it to
// heal transposed dimensions, which is not what these tests are about.
vi.mock("../utils/thumbnailGenerate", () => ({
  decodeThumbnailDimensions: vi.fn(async () => {
    throw new Error("no decoder in tests");
  }),
}));

import { reconcileSyncedPhotos, type SyncRecord } from "./syncReconcile";
import { api } from "../../api/client";
import { db, type CachedPhoto } from "../../db";

const download = vi.mocked(api.blobs.download);

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

function encodedThumb(mime = "image/jpeg"): ArrayBuffer {
  const payload = JSON.stringify({ data: btoa("thumb-bytes"), mime_type: mime });
  return new TextEncoder().encode(payload).buffer as ArrayBuffer;
}

/** Seed `n` fully-synced photos: mirror row + cached thumbnail bytes. */
async function seedSteadyState(n: number): Promise<{ records: SyncRecord[] }> {
  const rows: CachedPhoto[] = [];
  const thumbs = [];
  const records: SyncRecord[] = [];
  for (let i = 0; i < n; i++) {
    const id = `p${String(i).padStart(4, "0")}`;
    rows.push(cachedFor(id));
    thumbs.push({ blobId: id, data: new Uint8Array([1, 2, 3]).buffer as ArrayBuffer, mime: "image/jpeg" });
    records.push(record(id));
  }
  await db.photos.bulkPut(rows);
  await db.thumbs.bulkPut(thumbs);
  return { records };
}

beforeEach(async () => {
  await Promise.all([db.photos.clear(), db.thumbs.clear()]);
  download.mockReset();
  vi.restoreAllMocks();
});

/**
 * The #38 regression suite.
 *
 * Every assertion here is an *operation count*, because the defect was never
 * about correctness — the old reconcile produced the right mirror. It did so
 * with one serialized IndexedDB round-trip per photo, so a 10k library meant
 * 10k+ sequential transactions on the main thread every sync pass. Correctness
 * tests cannot catch that; only counting can.
 */
describe("reconcileSyncedPhotos — IndexedDB round-trips are bounded, not per-photo", () => {
  it("performs NO per-row reads on an unchanged, fully-cached library", async () => {
    const { records } = await seedSteadyState(300);
    const cached = await db.photos.toArray();

    const thumbGet = vi.spyOn(db.thumbs, "get");
    const photoGet = vi.spyOn(db.photos, "get");

    const stats = await reconcileSyncedPhotos(records, cached);

    expect(stats.unchanged).toBe(300);
    // The steady state is the common case: a 5-minute poll over a library that
    // has not changed must not touch the mirror once per photo.
    expect(thumbGet).not.toHaveBeenCalled();
    expect(photoGet).not.toHaveBeenCalled();
    expect(download).not.toHaveBeenCalled();
  });

  it("performs NO writes at all on an unchanged library", async () => {
    const { records } = await seedSteadyState(300);
    const cached = await db.photos.toArray();

    const update = vi.spyOn(db.photos, "update");
    const put = vi.spyOn(db.photos, "put");
    const bulkPut = vi.spyOn(db.photos, "bulkPut");

    await reconcileSyncedPhotos(records, cached);

    expect(update).not.toHaveBeenCalled();
    expect(put).not.toHaveBeenCalled();
    expect(bulkPut).not.toHaveBeenCalled();
  });

  it("inserts a page of new photos in bulk, not one put per photo", async () => {
    const records = Array.from({ length: 300 }, (_, i) =>
      record(`n${String(i).padStart(4, "0")}`),
    );
    download.mockResolvedValue(encodedThumb());

    const put = vi.spyOn(db.photos, "put");
    const bulkPut = vi.spyOn(db.photos, "bulkPut");

    const stats = await reconcileSyncedPhotos(records, []);

    expect(stats.inserted).toBe(300);
    expect(await db.photos.count()).toBe(300);
    expect(put).not.toHaveBeenCalled();
    // Chunked, so the bound is "a handful", not "one per photo".
    expect(bulkPut.mock.calls.length).toBeLessThanOrEqual(4);
  });

  it("applies updates to existing rows in bulk, not one update per row", async () => {
    const { records } = await seedSteadyState(300);
    const cached = await db.photos.toArray();
    // Every row changed: the server flipped the favourite flag.
    const flipped = records.map((r) => ({ ...r, is_favorite: true }));

    const update = vi.spyOn(db.photos, "update");
    const bulkPut = vi.spyOn(db.photos, "bulkPut");

    const stats = await reconcileSyncedPhotos(flipped, cached);

    expect(stats.updated).toBe(300);
    expect(update).not.toHaveBeenCalled();
    expect(bulkPut.mock.calls.length).toBeLessThanOrEqual(4);
    expect((await db.photos.get("p0000"))?.isFavorite).toBe(true);
  });
});

/**
 * The idle-disk-thrash guard, ported from the `ensureThumbCached` suite this
 * replaced (see repo todo.md "Idle Disk-Thrash Fix", and the
 * `idle-disk-thrash-investigation` memory).
 *
 * The original defect: a full-library sync every 2 s with no re-entrancy guard,
 * re-downloading thumbnails a racing pass had not yet persisted — ~28 blob
 * downloads/second against the server, at idle. The interval and the guard live
 * in `usePhotoSync`; the property they rely on lives HERE — a pass over an
 * already-cached library must issue no downloads at all. If that regresses, the
 * poll becomes a storm again no matter how long the interval is.
 */
describe("reconcileSyncedPhotos — repeated passes stay quiet", () => {
  it("caches once, then never downloads again across repeated passes", async () => {
    const records = [record("once")];
    download.mockResolvedValue(encodedThumb());

    // Cold: fetches the thumbnail.
    await reconcileSyncedPhotos(records, []);
    expect(download).toHaveBeenCalledTimes(1);

    // Every subsequent pass re-reads the mirror the way the hook does.
    for (let pass = 0; pass < 3; pass++) {
      const cached = await db.photos.toArray();
      const stats = await reconcileSyncedPhotos(records, cached);
      expect(stats.unchanged).toBe(1);
    }

    expect(download).toHaveBeenCalledTimes(1);
  });

  it("issues no downloads on a steady-state pass over a large library", async () => {
    const { records } = await seedSteadyState(300);
    const cached = await db.photos.toArray();

    await reconcileSyncedPhotos(records, cached);

    expect(download).not.toHaveBeenCalled();
  });
});

/**
 * Batching must not change what ends up in the mirror. These pin the semantics
 * the per-row loop had, so the optimization cannot quietly drop a field.
 */
describe("reconcileSyncedPhotos — mirror contents are unchanged by batching", () => {
  it("inserts a new photo with its metadata and thumbnail", async () => {
    download.mockResolvedValue(encodedThumb("image/jpeg"));

    await reconcileSyncedPhotos(
      [record("fresh", { is_favorite: true, photo_subtype: "motion", burst_id: "b1" })],
      [],
    );

    const row = await db.photos.get("fresh");
    expect(row).toMatchObject({
      blobId: "fresh",
      serverPhotoId: "fresh",
      storageBlobId: "fresh-blob",
      thumbnailBlobId: "fresh-thumb",
      filename: "fresh.jpg",
      mediaType: "photo",
      isFavorite: true,
      photoSubtype: "motion",
      burstId: "b1",
    });
    expect((await db.thumbs.get("fresh"))?.data.byteLength).toBeGreaterThan(0);
  });

  it("skips records that have no encrypted blob yet", async () => {
    // These rows exist server-side but have no ciphertext to show, so the
    // mirror cannot hold them. (#42 documents why the COUNT still includes
    // them — the badge is server-authoritative, the grid is not.)
    const stats = await reconcileSyncedPhotos(
      [record("pending", { encrypted_blob_id: null, encrypted_thumb_blob_id: null })],
      [],
    );

    expect(stats.inserted).toBe(0);
    expect(await db.photos.count()).toBe(0);
    expect(download).not.toHaveBeenCalled();
  });

  it("preserves local-only fields when updating a row", async () => {
    // albumIds is client-side membership; the server record knows nothing about
    // it. A whole-row bulkPut must not clobber it back to [].
    await db.photos.put(cachedFor("keep", { albumIds: ["album-1", "album-2"] }));
    await db.thumbs.put({
      blobId: "keep",
      data: new Uint8Array([9]).buffer as ArrayBuffer,
      mime: "image/jpeg",
    });
    const cached = await db.photos.toArray();

    await reconcileSyncedPhotos([record("keep", { is_favorite: true })], cached);

    const row = await db.photos.get("keep");
    expect(row?.isFavorite).toBe(true);
    expect(row?.albumIds).toEqual(["album-1", "album-2"]);
  });

  // ── The resolution ladder (#49) ────────────────────────────────────────
  //
  // The sync feed is the ONLY delivery path for renditions: the server's
  // change-log triggers nominate a photo when its playable rung set changes,
  // and this reconcile is what turns that nomination into something the viewer
  // can read. If these writes are missing the gear icon never appears, however
  // correct the server is — and nothing else in the client would notice.
  describe("the resolution ladder rides the mirror", () => {
    const ladder = [
      {
        short_edge: 2160,
        width: 3840,
        height: 2160,
        is_source: true,
        blob_id: "vid-blob",
        codec: "h264",
        size_bytes: 900,
      },
      {
        short_edge: 1080,
        width: 1920,
        height: 1080,
        is_source: false,
        blob_id: "rung-1080",
        codec: "h264",
        size_bytes: 300,
      },
    ];

    it("persists a ladder that arrives on an existing video", async () => {
      // The normal case by a mile: a rung is encoded minutes-to-hours after the
      // photo was registered, so the ladder always arrives as an UPDATE.
      await db.photos.put(cachedFor("vid", { mediaType: "video" }));
      await db.thumbs.put({
        blobId: "vid",
        data: new Uint8Array([1]).buffer as ArrayBuffer,
        mime: "image/jpeg",
      });
      const cached = await db.photos.toArray();

      await reconcileSyncedPhotos(
        [record("vid", { media_type: "video", renditions: ladder })],
        cached,
      );

      expect((await db.photos.get("vid"))?.renditions).toEqual(ladder);
    });

    it("persists a ladder that arrives with a brand-new video", async () => {
      download.mockResolvedValue(encodedThumb());

      await reconcileSyncedPhotos(
        [record("vid", { media_type: "video", renditions: ladder })],
        [],
      );

      expect((await db.photos.get("vid"))?.renditions).toEqual(ladder);
    });

    it("does not rewrite the library when a #49 server reports no rungs", async () => {
      // `undefined` (cached before this field existed) and `[]` (a #49 server
      // saying "this video needs no rung" — ~600 of the live library) are the
      // same state. Treating them as different makes the first pass after a
      // server upgrade rewrite every video, and every pass after it do so
      // again — reintroducing exactly the O(library) write amplification #38
      // removed.
      await db.photos.put(cachedFor("vid", { mediaType: "video" }));
      await db.thumbs.put({
        blobId: "vid",
        data: new Uint8Array([1]).buffer as ArrayBuffer,
        mime: "image/jpeg",
      });
      const cached = await db.photos.toArray();

      const stats = await reconcileSyncedPhotos(
        [record("vid", { media_type: "video", renditions: [] })],
        cached,
      );

      expect(stats.updated).toBe(0);
      expect(stats.unchanged).toBe(1);
    });

    it("replaces a rung whose blob was re-encoded under the same short edge", async () => {
      // `upsert_rendition` refreshes a rung in place, so a re-encode changes
      // the blob id without changing the rung's identity. A comparison on
      // length or short_edge alone leaves the viewer fetching bytes the server
      // has already replaced.
      await db.photos.put(
        cachedFor("vid", { mediaType: "video", renditions: ladder }),
      );
      await db.thumbs.put({
        blobId: "vid",
        data: new Uint8Array([1]).buffer as ArrayBuffer,
        mime: "image/jpeg",
      });
      const cached = await db.photos.toArray();
      const reEncoded = [ladder[0], { ...ladder[1], blob_id: "rung-1080-v2" }];

      await reconcileSyncedPhotos(
        [record("vid", { media_type: "video", renditions: reEncoded })],
        cached,
      );

      expect((await db.photos.get("vid"))?.renditions?.[1].blob_id).toBe("rung-1080-v2");
    });

    it("clears the ladder when the server withdraws it", async () => {
      // A withdrawn rung changes the picker as much as an added one. Leaving a
      // stale entry offers a quality whose blob is gone — a menu item that 404s.
      await db.photos.put(
        cachedFor("vid", { mediaType: "video", renditions: ladder }),
      );
      await db.thumbs.put({
        blobId: "vid",
        data: new Uint8Array([1]).buffer as ArrayBuffer,
        mime: "image/jpeg",
      });
      const cached = await db.photos.toArray();

      await reconcileSyncedPhotos(
        [record("vid", { media_type: "video", renditions: [] })],
        cached,
      );

      const row = await db.photos.get("vid");
      expect(row?.renditions ?? []).toEqual([]);
    });
  });

  it("re-downloads a thumbnail when the server's thumb blob id changed", async () => {
    await db.photos.put(cachedFor("rot"));
    await db.thumbs.put({
      blobId: "rot",
      data: new Uint8Array([1]).buffer as ArrayBuffer,
      mime: "image/jpeg",
    });
    const cached = await db.photos.toArray();
    download.mockResolvedValue(encodedThumb("image/gif"));

    await reconcileSyncedPhotos(
      [record("rot", { encrypted_thumb_blob_id: "rot-thumb-v2" })],
      cached,
    );

    expect(download).toHaveBeenCalledWith("rot-thumb-v2");
    expect((await db.photos.get("rot"))?.thumbnailBlobId).toBe("rot-thumb-v2");
    expect((await db.thumbs.get("rot"))?.mime).toBe("image/gif");
  });

  it("downloads a missing thumbnail for a row whose thumb id did not change", async () => {
    // Mirror row present, thumbnail bytes absent (interrupted earlier pass).
    await db.photos.put(cachedFor("nothumb"));
    const cached = await db.photos.toArray();
    download.mockResolvedValue(encodedThumb());

    await reconcileSyncedPhotos([record("nothumb")], cached);

    expect(download).toHaveBeenCalledWith("nothumb-thumb");
    expect((await db.thumbs.get("nothumb"))?.data.byteLength).toBeGreaterThan(0);
  });

  it("survives a failed thumbnail download and still writes the row", async () => {
    download.mockRejectedValue(new Error("network down"));

    const stats = await reconcileSyncedPhotos([record("broken")], []);

    expect(stats.inserted).toBe(1);
    expect(await db.photos.get("broken")).toBeDefined();
    expect(await db.thumbs.get("broken")).toBeUndefined();
  });

  it("does not download when the record carries no thumb id anywhere", async () => {
    await db.photos.put(cachedFor("bare", { thumbnailBlobId: undefined }));
    const cached = await db.photos.toArray();

    await reconcileSyncedPhotos(
      [record("bare", { encrypted_thumb_blob_id: null })],
      cached,
    );

    expect(download).not.toHaveBeenCalled();
  });

  it("falls back to the server thumb id when the row has none", async () => {
    await db.photos.put(cachedFor("fallback", { thumbnailBlobId: undefined }));
    const cached = await db.photos.toArray();
    download.mockResolvedValue(encodedThumb());

    await reconcileSyncedPhotos(
      [record("fallback", { encrypted_thumb_blob_id: "server-thumb" })],
      cached,
    );

    expect(download).toHaveBeenCalledWith("server-thumb");
  });

  it("binds an unbound local row to its server id by blob id", async () => {
    // A row uploaded by this client before the server record came back: keyed
    // by the encrypted blob id, with no serverPhotoId yet.
    await db.photos.put(cachedFor("up-blob", { blobId: "up-blob", serverPhotoId: undefined }));
    await db.thumbs.put({
      blobId: "up-blob",
      data: new Uint8Array([1]).buffer as ArrayBuffer,
      mime: "image/jpeg",
    });
    const cached = await db.photos.toArray();

    await reconcileSyncedPhotos(
      [record("srv-1", { encrypted_blob_id: "up-blob", encrypted_thumb_blob_id: "up-blob-thumb" })],
      cached,
    );

    // Bound in place — NOT duplicated under the server id.
    expect(await db.photos.count()).toBe(1);
    expect((await db.photos.get("up-blob"))?.serverPhotoId).toBe("srv-1");
  });
});
