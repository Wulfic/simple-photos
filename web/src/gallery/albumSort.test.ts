import { describe, it, expect, beforeEach, beforeAll } from "vitest";
import {
  compareAlbumPhotos,
  sortAlbumPhotos,
  readAlbumSort,
  writeAlbumSort,
  isAlbumSort,
  defaultDirFor,
  DEFAULT_ALBUM_SORT,
  type AlbumSort,
} from "./albumSort";
import type { PhotoWithBurstCount } from "../utils/burstCollapse";

function p(
  blobId: string,
  over: Partial<PhotoWithBurstCount> = {}
): PhotoWithBurstCount {
  return {
    blobId,
    filename: `${blobId}.jpg`,
    takenAt: 0,
    mimeType: "image/jpeg",
    mediaType: "photo",
    width: 1,
    height: 1,
    albumIds: [],
    ...over,
  };
}

const ids = (list: PhotoWithBurstCount[]) => list.map((x) => x.blobId);

// This project's test environment is node (no jsdom), so localStorage is not
// defined. A tiny in-memory stub is enough to exercise the persistence helpers.
beforeAll(() => {
  if (typeof globalThis.localStorage === "undefined") {
    const store = new Map<string, string>();
    globalThis.localStorage = {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => void store.set(k, String(v)),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: (i: number) => [...store.keys()][i] ?? null,
      get length() {
        return store.size;
      },
    } as Storage;
  }
});

describe("compareAlbumPhotos — date", () => {
  it("desc puts the newest capture first", () => {
    const list = [
      p("old", { takenAt: 100 }),
      p("new", { takenAt: 300 }),
      p("mid", { takenAt: 200 }),
    ];
    expect(ids(sortAlbumPhotos(list, { field: "date", dir: "desc" }))).toEqual([
      "new",
      "mid",
      "old",
    ]);
  });

  it("asc puts the oldest capture first", () => {
    const list = [
      p("old", { takenAt: 100 }),
      p("new", { takenAt: 300 }),
      p("mid", { takenAt: 200 }),
    ];
    expect(ids(sortAlbumPhotos(list, { field: "date", dir: "asc" }))).toEqual([
      "old",
      "mid",
      "new",
    ]);
  });

  it("treats a missing takenAt as the oldest (last in desc)", () => {
    const list = [p("has", { takenAt: 500 }), p("missing", { takenAt: 0 })];
    expect(ids(sortAlbumPhotos(list, { field: "date", dir: "desc" }))).toEqual([
      "has",
      "missing",
    ]);
    expect(ids(sortAlbumPhotos(list, { field: "date", dir: "asc" }))).toEqual([
      "missing",
      "has",
    ]);
  });
});

describe("compareAlbumPhotos — name", () => {
  it("orders numerically so IMG_2 precedes IMG_10 (asc)", () => {
    const list = [
      p("x", { filename: "IMG_10.jpg" }),
      p("y", { filename: "IMG_2.jpg" }),
      p("z", { filename: "IMG_1.jpg" }),
    ];
    expect(ids(sortAlbumPhotos(list, { field: "name", dir: "asc" }))).toEqual([
      "z",
      "y",
      "x",
    ]);
  });

  it("is case-insensitive", () => {
    const list = [p("x", { filename: "banana.jpg" }), p("y", { filename: "Apple.jpg" })];
    expect(ids(sortAlbumPhotos(list, { field: "name", dir: "asc" }))).toEqual(["y", "x"]);
  });

  it("desc reverses the name order", () => {
    const list = [
      p("a", { filename: "a.jpg" }),
      p("b", { filename: "b.jpg" }),
      p("c", { filename: "c.jpg" }),
    ];
    expect(ids(sortAlbumPhotos(list, { field: "name", dir: "desc" }))).toEqual([
      "c",
      "b",
      "a",
    ]);
  });
});

describe("compareAlbumPhotos — deterministic ties", () => {
  it("breaks a date tie on blobId so order is stable, not random", () => {
    const list = [
      p("zeta", { takenAt: 100 }),
      p("alpha", { takenAt: 100 }),
      p("mu", { takenAt: 100 }),
    ];
    // Same capture time → deterministic blobId order regardless of input order.
    const asc = ids(sortAlbumPhotos(list, { field: "date", dir: "asc" }));
    expect(asc).toEqual(["alpha", "mu", "zeta"]);
    // A shuffled input yields the identical result — this is the stability
    // JustifiedGrid relies on.
    const shuffled = [list[2], list[0], list[1]];
    expect(ids(sortAlbumPhotos(shuffled, { field: "date", dir: "asc" }))).toEqual(asc);
  });

  it("breaks a name tie on blobId", () => {
    const list = [
      p("b2", { filename: "same.jpg" }),
      p("b1", { filename: "same.jpg" }),
    ];
    expect(ids(sortAlbumPhotos(list, { field: "name", dir: "asc" }))).toEqual(["b1", "b2"]);
  });

  it("does not mutate the input array", () => {
    const list = [p("b", { takenAt: 2 }), p("a", { takenAt: 1 })];
    const before = ids(list);
    sortAlbumPhotos(list, { field: "date", dir: "asc" });
    expect(ids(list)).toEqual(before);
  });
});

describe("compareAlbumPhotos — raw comparator sign", () => {
  it("returns a negative number when a should sort before b", () => {
    const a = p("a", { takenAt: 300 });
    const b = p("b", { takenAt: 100 });
    // desc: newer (300) before older (100).
    expect(compareAlbumPhotos(a, b, { field: "date", dir: "desc" })).toBeLessThan(0);
  });
});

describe("persistence", () => {
  beforeEach(() => localStorage.clear());

  it("round-trips a chosen sort per album id", () => {
    const sort: AlbumSort = { field: "name", dir: "asc" };
    writeAlbumSort("album-7", sort);
    expect(readAlbumSort("album-7")).toEqual(sort);
    // A different album is unaffected.
    expect(readAlbumSort("album-8")).toBeNull();
  });

  it("returns null (intrinsic order) when nothing is stored", () => {
    expect(readAlbumSort("never-set")).toBeNull();
  });

  it("treats a corrupt stored value as no choice", () => {
    localStorage.setItem("albumSort:bad", "{not json");
    expect(readAlbumSort("bad")).toBeNull();
    localStorage.setItem("albumSort:wrong", JSON.stringify({ field: "size", dir: "up" }));
    expect(readAlbumSort("wrong")).toBeNull();
  });
});

describe("helpers", () => {
  it("defaults dates to desc and names to asc", () => {
    expect(defaultDirFor("date")).toBe("desc");
    expect(defaultDirFor("name")).toBe("asc");
  });

  it("DEFAULT_ALBUM_SORT is the historical date-desc order", () => {
    expect(DEFAULT_ALBUM_SORT).toEqual({ field: "date", dir: "desc" });
  });

  it("isAlbumSort rejects malformed values", () => {
    expect(isAlbumSort({ field: "date", dir: "desc" })).toBe(true);
    expect(isAlbumSort({ field: "date" })).toBe(false);
    expect(isAlbumSort(null)).toBe(false);
    expect(isAlbumSort("date")).toBe(false);
  });
});
