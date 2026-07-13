import { describe, it, expect } from "vitest";
import { resolveAlbumPhotos, countRegularAlbum } from "./useAlbumPhotos";
import { SMART_ALBUM_DEFS } from "../gallery/smartAlbums";
import type { CachedPhoto, CachedAlbum } from "../db";

function photo(blobId: string, over: Partial<CachedPhoto> = {}): CachedPhoto {
  return {
    blobId,
    filename: `${blobId}.jpg`,
    takenAt: 0,
    mimeType: "image/jpeg",
    mediaType: "photo",
    width: 100,
    height: 100,
    albumIds: [],
    ...over,
  };
}

function album(photoBlobIds: string[]): CachedAlbum {
  return {
    albumId: "album-1",
    manifestBlobId: "manifest-1",
    name: "Trip",
    createdAt: 0,
    photoBlobIds,
  };
}

describe("resolveAlbumPhotos — regular albums", () => {
  const mirror = [
    photo("a"),
    photo("b"),
    photo("secret"),
    photo("c"),
    photo("not-in-album"),
  ];

  it("returns only manifest members present in the mirror", () => {
    const out = resolveAlbumPhotos({
      kind: "regular",
      allPhotos: mirror,
      secureBlobIds: new Set(),
      album: album(["a", "b", "c", "gone"]),
    });
    expect(out.map((p) => p.blobId)).toEqual(["a", "b", "c"]);
  });

  it("excludes secure blob ids so the count matches the rendered grid (#12/#20)", () => {
    const out = resolveAlbumPhotos({
      kind: "regular",
      allPhotos: mirror,
      secureBlobIds: new Set(["secret"]),
      album: album(["a", "b", "secret", "c"]),
    });
    // 'secret' is in the manifest but hidden — must NOT be counted or shown.
    expect(out.map((p) => p.blobId)).toEqual(["a", "b", "c"]);
    expect(out.length).toBe(3);
  });

  it("returns [] when the manifest hasn't loaded", () => {
    expect(
      resolveAlbumPhotos({
        kind: "regular",
        allPhotos: mirror,
        secureBlobIds: new Set(),
        album: undefined,
      })
    ).toEqual([]);
  });
});

describe("resolveAlbumPhotos — smart albums", () => {
  const mirror = [
    photo("v1", { mediaType: "video" }),
    photo("p1", { mediaType: "photo", isFavorite: true }),
    photo("g1", { mediaType: "gif" }),
    photo("secret", { mediaType: "video" }),
    photo("v2", { mediaType: "video" }),
  ];

  it("smart-videos filters to videos and excludes secure", () => {
    const out = resolveAlbumPhotos({
      kind: "smart",
      allPhotos: mirror,
      secureBlobIds: new Set(["secret"]),
      smartDef: SMART_ALBUM_DEFS["smart-videos"],
    });
    expect(out.map((p) => p.blobId)).toEqual(["v1", "v2"]);
  });

  it("smart-photos includes gifs", () => {
    const out = resolveAlbumPhotos({
      kind: "smart",
      allPhotos: mirror,
      secureBlobIds: new Set(),
      smartDef: SMART_ALBUM_DEFS["smart-photos"],
    });
    expect(out.map((p) => p.blobId).sort()).toEqual(["g1", "p1"]);
  });

  it("smart-favorites filters to favorites", () => {
    const out = resolveAlbumPhotos({
      kind: "smart",
      allPhotos: mirror,
      secureBlobIds: new Set(),
      smartDef: SMART_ALBUM_DEFS["smart-favorites"],
    });
    expect(out.map((p) => p.blobId)).toEqual(["p1"]);
  });

  it("smart-recent sorts by addedAt desc and caps to the limit", () => {
    const many: CachedPhoto[] = Array.from({ length: 150 }, (_, i) =>
      photo(`r${i}`, { addedAt: i })
    );
    const out = resolveAlbumPhotos({
      kind: "smart",
      allPhotos: many,
      secureBlobIds: new Set(),
      smartDef: SMART_ALBUM_DEFS["smart-recent"],
    });
    expect(out.length).toBe(100);
    // Newest addedAt first.
    expect(out[0].blobId).toBe("r149");
    expect(out[99].blobId).toBe("r50");
  });

  it("collapses bursts in smart albums (count is one per stack)", () => {
    const withBurst = [
      photo("b1", { mediaType: "video", burstId: "grp" }),
      photo("b2", { mediaType: "video", burstId: "grp" }),
      photo("solo", { mediaType: "video" }),
    ];
    const out = resolveAlbumPhotos({
      kind: "smart",
      allPhotos: withBurst,
      secureBlobIds: new Set(),
      smartDef: SMART_ALBUM_DEFS["smart-videos"],
    });
    expect(out.map((p) => p.blobId)).toEqual(["b1", "solo"]);
    expect(out.find((p) => p.blobId === "b1")?._burstCount).toBe(2);
  });
});

describe("countRegularAlbum — album-list badge source (#12)", () => {
  const mirror = [photo("a"), photo("b"), photo("secret"), photo("c")];

  it("counts only manifest members present in the mirror (drops stale ids)", () => {
    // "gone" was deleted from the library — must not be counted.
    expect(countRegularAlbum(album(["a", "b", "c", "gone"]), mirror, new Set())).toBe(3);
  });

  it("excludes secure blob ids so the badge matches the secure-filtered grid", () => {
    expect(
      countRegularAlbum(album(["a", "b", "secret", "c"]), mirror, new Set(["secret"]))
    ).toBe(3);
  });

  it("equals resolveAlbumPhotos(...).length — badge can't diverge from grid", () => {
    const members = album(["a", "b", "secret", "c", "gone"]);
    const secure = new Set(["secret"]);
    const resolved = resolveAlbumPhotos({
      kind: "regular",
      allPhotos: mirror,
      secureBlobIds: secure,
      album: members,
    });
    expect(countRegularAlbum(members, mirror, secure)).toBe(resolved.length);
  });

  it("falls back to raw manifest size while the mirror is cold (avoids flashing 0)", () => {
    expect(countRegularAlbum(album(["a", "b", "c"]), undefined, new Set())).toBe(3);
    expect(countRegularAlbum(album(["a", "b", "c"]), [], new Set())).toBe(3);
  });
});

describe("resolveAlbumPhotos — count invariant", () => {
  it("count always equals the resolved list length across kinds", () => {
    const mirror = [photo("a"), photo("b"), photo("c")];
    const regular = resolveAlbumPhotos({
      kind: "regular",
      allPhotos: mirror,
      secureBlobIds: new Set(["b"]),
      album: album(["a", "b", "c"]),
    });
    const smart = resolveAlbumPhotos({
      kind: "smart",
      allPhotos: mirror,
      secureBlobIds: new Set(["b"]),
      smartDef: SMART_ALBUM_DEFS["smart-photos"],
    });
    // The hook derives `count` as `photos.length`, so this is the guarantee
    // that the badge can never diverge from the grid.
    expect(regular.length).toBe(2);
    expect(smart.length).toBe(2);
  });

  it("unknown kind resolves to empty", () => {
    expect(
      resolveAlbumPhotos({
        kind: "unknown",
        allPhotos: [photo("a")],
        secureBlobIds: new Set(),
      })
    ).toEqual([]);
  });
});
