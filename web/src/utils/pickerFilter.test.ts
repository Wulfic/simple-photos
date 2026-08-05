import { describe, it, expect } from "vitest";
import { filterPickerPhotos } from "./pickerFilter";
import type { CachedPhoto } from "../db";

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

describe("filterPickerPhotos", () => {
  const list = [
    photo("a", { filename: "IMG_1234.jpg" }),
    photo("b", { filename: "vacation-beach.png" }),
    photo("c", { filename: "IMG_5678.HEIC" }),
  ];

  it("returns the same reference when the query is empty", () => {
    expect(filterPickerPhotos(list, "")).toBe(list);
    expect(filterPickerPhotos(list, "   ")).toBe(list);
  });

  it("matches a case-insensitive filename substring", () => {
    expect(filterPickerPhotos(list, "img").map((p) => p.blobId)).toEqual([
      "a",
      "c",
    ]);
    expect(filterPickerPhotos(list, "BEACH").map((p) => p.blobId)).toEqual([
      "b",
    ]);
  });

  it("trims surrounding whitespace from the query", () => {
    expect(filterPickerPhotos(list, "  5678  ").map((p) => p.blobId)).toEqual([
      "c",
    ]);
  });

  it("returns an empty list when nothing matches", () => {
    expect(filterPickerPhotos(list, "nope")).toEqual([]);
  });

  it("never matches a photo with no filename against a non-empty query", () => {
    const withMissing = [
      ...list,
      photo("d", { filename: undefined as unknown as string }),
    ];
    expect(filterPickerPhotos(withMissing, "img").map((p) => p.blobId)).toEqual([
      "a",
      "c",
    ]);
  });
});
