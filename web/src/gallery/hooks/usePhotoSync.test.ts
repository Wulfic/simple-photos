// Real IndexedDB (in-memory) so `resolveThumb`/`putThumb` run against the
// actual Dexie schema — the whole point of the guard is what the persisted
// thumb cache does across sync passes, which a mock would not prove.
import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach, vi } from "vitest";

// The sync module reaches for the network (blob download) and the crypto layer;
// stub both so the test isolates the *decision* to download, not the transport.
vi.mock("../../api/client", () => ({
  api: { blobs: { download: vi.fn() } },
}));
vi.mock("../../crypto/crypto", () => ({
  // Pass-through: the "encrypted" bytes the mock download returns are already
  // the plaintext ThumbnailPayload JSON.
  decrypt: vi.fn(async (buf: ArrayBuffer) => buf),
}));

import { ensureThumbCached } from "./usePhotoSync";
import { api } from "../../api/client";
import { db, type CachedPhoto } from "../../db";
import { putThumb } from "../../db/thumbs";

const download = vi.mocked(api.blobs.download);

function photo(blobId: string, over: Partial<CachedPhoto> = {}): CachedPhoto {
  return {
    blobId,
    filename: `${blobId}.jpg`,
    takenAt: 0,
    mimeType: "image/jpeg",
    mediaType: "photo",
    width: 10,
    height: 10,
    albumIds: [],
    thumbnailBlobId: `${blobId}-thumb`,
    ...over,
  };
}

/** A blob-download result that decrypts to a ThumbnailPayload for `mime`. */
function encodedThumb(mime = "image/jpeg"): ArrayBuffer {
  const payload = JSON.stringify({ data: btoa("thumb-bytes"), mime_type: mime });
  return new TextEncoder().encode(payload).buffer;
}

beforeEach(async () => {
  await Promise.all([db.photos.clear(), db.thumbs.clear()]);
  download.mockReset();
});

describe("ensureThumbCached — periodic-sync anti-thrash guard", () => {
  it("does NOT download when the thumbnail already resolves from the cache", async () => {
    // The exact steady-state the 2s-sync storm violated: bytes are already in
    // the `thumbs` table, so a subsequent sync pass must not re-fetch them.
    const p = photo("p1");
    await db.photos.put(p);
    await putThumb("p1", new Uint8Array([1, 2, 3]).buffer, "image/jpeg");

    const downloaded = await ensureThumbCached(p, "p1-thumb");

    expect(downloaded).toBe(false);
    expect(download).not.toHaveBeenCalled();
  });

  it("downloads and stores the thumbnail only when it is missing", async () => {
    const p = photo("p2");
    await db.photos.put(p);
    download.mockResolvedValueOnce(encodedThumb("image/jpeg"));

    const downloaded = await ensureThumbCached(p, "p2-thumb");

    expect(downloaded).toBe(true);
    expect(download).toHaveBeenCalledTimes(1);
    expect(download).toHaveBeenCalledWith("p2-thumb"); // existing.thumbnailBlobId
    // …and it landed in the thumbs table, so the *next* pass is a no-op.
    expect((await db.thumbs.get("p2"))?.data.byteLength).toBeGreaterThan(0);
  });

  it("caches once, then never downloads again on repeated passes", async () => {
    const p = photo("p3");
    await db.photos.put(p);
    download.mockResolvedValue(encodedThumb());

    await ensureThumbCached(p, "p3-thumb"); // first pass: fetches
    await ensureThumbCached(p, "p3-thumb"); // second pass: cached
    await ensureThumbCached(p, "p3-thumb"); // third pass: cached

    expect(download).toHaveBeenCalledTimes(1);
  });

  it("falls back to the server thumb id when the row has none", async () => {
    const p = photo("p4", { thumbnailBlobId: undefined });
    await db.photos.put(p);
    download.mockResolvedValueOnce(encodedThumb());

    const downloaded = await ensureThumbCached(p, "server-thumb");

    expect(downloaded).toBe(true);
    expect(download).toHaveBeenCalledWith("server-thumb");
  });

  it("does nothing when there is no thumb id anywhere", async () => {
    const p = photo("p5", { thumbnailBlobId: undefined });
    await db.photos.put(p);

    const downloaded = await ensureThumbCached(p, undefined);

    expect(downloaded).toBe(false);
    expect(download).not.toHaveBeenCalled();
  });

  it("swallows a download failure and leaves the cache empty for a later retry", async () => {
    const p = photo("p6");
    await db.photos.put(p);
    download.mockRejectedValueOnce(new Error("network down"));

    const downloaded = await ensureThumbCached(p, "p6-thumb");

    expect(downloaded).toBe(false);
    expect(await db.thumbs.get("p6")).toBeUndefined();
  });
});
