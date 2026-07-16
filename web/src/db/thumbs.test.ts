// Real IndexedDB (in-memory) so the Dexie schema, the v10 upgrade and the
// backfill are exercised as they actually run in a browser. These paths move a
// user's whole thumbnail cache; a mock would prove nothing about them.
import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach } from "vitest";
import { db, type CachedPhoto } from "./index";
import {
  backfillThumbs,
  copyThumb,
  deleteThumbs,
  getThumb,
  putThumb,
  resolveThumb,
} from "./thumbs";

function bytes(...values: number[]): ArrayBuffer {
  return new Uint8Array(values).buffer;
}

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
    ...over,
  };
}

/** A row as it was written before the split: bytes inline. */
function legacyPhoto(blobId: string, data: ArrayBuffer, mime?: string): CachedPhoto {
  return photo(blobId, { thumbnailData: data, thumbnailMimeType: mime });
}

async function firstByte(buf: ArrayBuffer | undefined): Promise<number | undefined> {
  return buf ? new Uint8Array(buf)[0] : undefined;
}

beforeEach(async () => {
  await Promise.all([db.photos.clear(), db.thumbs.clear(), db.albums.clear()]);
});

describe("putThumb / resolveThumb", () => {
  it("round-trips thumbnail bytes through the thumbs table", async () => {
    await db.photos.put(photo("b1"));
    await putThumb("b1", bytes(1, 2, 3), "image/jpeg");

    const resolved = await resolveThumb(await db.photos.get("b1"));
    expect(await firstByte(resolved?.data)).toBe(1);
    expect(resolved?.mime).toBe("image/jpeg");
  });

  it("keeps the bytes off the photo row", async () => {
    // The entire point of the split: a photo row must stay lean, so queries over
    // the mirror don't deserialize the whole library's thumbnails.
    await db.photos.put(photo("b1"));
    await putThumb("b1", bytes(1, 2, 3), "image/jpeg");

    expect((await db.photos.get("b1"))?.thumbnailData).toBeUndefined();
  });

  it("mirrors the mime onto the row for the tile's first paint", async () => {
    // ThumbnailTile decides GIF autoplay from this synchronously — an async read
    // would make every animated-thumb GIF fetch its full blob first.
    await db.photos.put(photo("g1", { mediaType: "gif" }));
    await putThumb("g1", bytes(9), "image/gif");

    expect((await db.photos.get("g1"))?.thumbnailMimeType).toBe("image/gif");
  });

  it("defaults a gif's thumbnail mime when none was given", async () => {
    await db.photos.put(photo("g1", { mediaType: "gif" }));
    await putThumb("g1", bytes(9), undefined, "gif");
    expect((await getThumb("g1"))?.mime).toBe("image/gif");
  });

  it("resolves nothing for a photo with no thumbnail", async () => {
    await db.photos.put(photo("b1"));
    expect(await resolveThumb(await db.photos.get("b1"))).toBeUndefined();
    expect(await getThumb("b1")).toBeUndefined();
  });

  it("resolves nothing for an absent photo", async () => {
    expect(await resolveThumb(undefined)).toBeUndefined();
  });
});

describe("resolveThumb — legacy rows", () => {
  it("still finds bytes the backfill hasn't moved yet", async () => {
    // A row written before the split. It must keep rendering, or every user's
    // gallery goes blank until the migration drains.
    await db.photos.put(legacyPhoto("old", bytes(7), "image/gif"));

    const resolved = await resolveThumb(await db.photos.get("old"));
    expect(await firstByte(resolved?.data)).toBe(7);
    expect(resolved?.mime).toBe("image/gif");
  });

  it("finds legacy bytes by id alone", async () => {
    await db.photos.put(legacyPhoto("old", bytes(7)));
    expect(await firstByte((await getThumb("old"))?.data)).toBe(7);
  });
});

describe("backfillThumbs", () => {
  it("moves legacy bytes into the thumbs table and leans the row", async () => {
    await db.photos.bulkPut([
      legacyPhoto("a", bytes(1), "image/jpeg"),
      legacyPhoto("b", bytes(2), "image/gif"),
    ]);

    expect(await backfillThumbs()).toBe(2);

    expect((await db.photos.get("a"))?.thumbnailData).toBeUndefined();
    expect((await db.photos.get("b"))?.thumbnailData).toBeUndefined();
    expect(await firstByte((await db.thumbs.get("a"))?.data)).toBe(1);
    expect((await db.thumbs.get("b"))?.mime).toBe("image/gif");
  });

  it("keeps every photo resolvable across the move", async () => {
    // The migration's real contract: nothing a user could see changes.
    await db.photos.bulkPut([
      legacyPhoto("a", bytes(1), "image/jpeg"),
      legacyPhoto("b", bytes(2), "image/gif"),
    ]);
    const before = await Promise.all(
      ["a", "b"].map(async (id) => resolveThumb(await db.photos.get(id))),
    );

    await backfillThumbs();

    const after = await Promise.all(
      ["a", "b"].map(async (id) => resolveThumb(await db.photos.get(id))),
    );
    for (let i = 0; i < before.length; i++) {
      expect(await firstByte(after[i]?.data)).toBe(await firstByte(before[i]?.data));
      expect(after[i]?.mime).toBe(before[i]?.mime);
    }
  });

  it("preserves every other field on the row", async () => {
    await db.photos.put(
      legacyPhoto("a", bytes(1), "image/jpeg"),
    );
    await db.photos.update("a", { isFavorite: true, burstId: "grp", width: 4032 });

    await backfillThumbs();

    const row = await db.photos.get("a");
    expect(row?.isFavorite).toBe(true);
    expect(row?.burstId).toBe("grp");
    expect(row?.width).toBe(4032);
    expect(row?.filename).toBe("a.jpg");
  });

  it("keeps the mime hint on the row after moving the bytes", async () => {
    await db.photos.put(legacyPhoto("g", bytes(9), "image/gif"));
    await backfillThumbs();
    expect((await db.photos.get("g"))?.thumbnailMimeType).toBe("image/gif");
  });

  it("infers a missing mime from the media type", async () => {
    await db.photos.put(photo("g", { mediaType: "gif", thumbnailData: bytes(9) }));
    await backfillThumbs();
    expect((await db.thumbs.get("g"))?.mime).toBe("image/gif");
  });

  it("is idempotent — a second run moves nothing", async () => {
    await db.photos.put(legacyPhoto("a", bytes(1)));
    expect(await backfillThumbs()).toBe(1);
    expect(await backfillThumbs()).toBe(0);
    expect(await firstByte((await db.thumbs.get("a"))?.data)).toBe(1);
  });

  it("resumes after being interrupted", async () => {
    // Closing the tab mid-migration must cost nothing: rows not yet moved still
    // render from the legacy field, and the next run finishes the job.
    await db.photos.bulkPut([
      legacyPhoto("a", bytes(1)),
      legacyPhoto("b", bytes(2)),
    ]);
    await backfillThumbs();
    // Simulate a row that never got moved (the interrupted remainder).
    await db.photos.put(legacyPhoto("c", bytes(3)));

    expect(await backfillThumbs()).toBe(1);
    expect(await firstByte((await db.thumbs.get("c"))?.data)).toBe(3);
    expect(await firstByte((await db.thumbs.get("a"))?.data)).toBe(1);
  });

  it("skips rows that never had a thumbnail", async () => {
    await db.photos.bulkPut([photo("none"), legacyPhoto("a", bytes(1))]);
    expect(await backfillThumbs()).toBe(1);
    expect(await db.thumbs.get("none")).toBeUndefined();
  });

  it("handles a library larger than one chunk", async () => {
    // The chunking is the whole reason this isn't a Dexie upgrade function.
    const many = Array.from({ length: 450 }, (_, i) =>
      legacyPhoto(`p${i}`, bytes(i % 255)),
    );
    await db.photos.bulkPut(many);

    expect(await backfillThumbs()).toBe(450);
    expect(await db.thumbs.count()).toBe(450);
    expect(
      (await db.photos.toArray()).every((p) => p.thumbnailData === undefined),
    ).toBe(true);
  });

  it("does nothing on an empty mirror", async () => {
    expect(await backfillThumbs()).toBe(0);
  });
});

describe("copyThumb / deleteThumbs", () => {
  it("gives a copy its own thumbnail", async () => {
    await db.photos.put(photo("orig"));
    await putThumb("orig", bytes(5), "image/jpeg");
    await db.photos.put(photo("copy"));

    await copyThumb(await db.photos.get("orig"), "copy");

    expect(await firstByte((await db.thumbs.get("copy"))?.data)).toBe(5);
  });

  it("leaves the original alone when the copy is deleted", async () => {
    await db.photos.put(photo("orig"));
    await putThumb("orig", bytes(5));
    await db.photos.put(photo("copy"));
    await copyThumb(await db.photos.get("orig"), "copy");

    await deleteThumbs(["copy"]);

    expect(await db.thumbs.get("copy")).toBeUndefined();
    expect(await firstByte((await db.thumbs.get("orig"))?.data)).toBe(5);
  });

  it("copies from a legacy row that hasn't been backfilled", async () => {
    await db.photos.put(legacyPhoto("orig", bytes(5), "image/gif"));
    await db.photos.put(photo("copy"));

    await copyThumb(await db.photos.get("orig"), "copy");

    expect(await firstByte((await db.thumbs.get("copy"))?.data)).toBe(5);
    expect((await db.thumbs.get("copy"))?.mime).toBe("image/gif");
  });

  it("copying a photo with no thumbnail is a no-op", async () => {
    await db.photos.put(photo("orig"));
    await copyThumb(await db.photos.get("orig"), "copy");
    expect(await db.thumbs.get("copy")).toBeUndefined();
  });

  it("deleting nothing is a no-op", async () => {
    await expect(deleteThumbs([])).resolves.toBeUndefined();
  });
});
