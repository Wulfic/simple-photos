import { describe, expect, it } from "vitest";
import {
  countsFromMirror,
  countsFromSummary,
  resolveSmartAlbumCounts,
} from "./smartAlbumCounts";
import type { CachedPhoto } from "../db";
import type { PhotoSummary } from "../hooks/usePhotoSummary";

function summary(over: Partial<PhotoSummary> = {}): PhotoSummary {
  return {
    total: 0,
    collapsed_total: 0,
    photos: 0,
    gifs: 0,
    videos: 0,
    audio: 0,
    favorites: 0,
    smart_photos: 0,
    smart_gifs: 0,
    smart_videos: 0,
    smart_audio: 0,
    smart_favorites: 0,
    smart_recent: 0,
    ...over,
  };
}

function photo(over: Partial<CachedPhoto> = {}): CachedPhoto {
  return {
    blobId: Math.random().toString(36).slice(2),
    mediaType: "photo",
    isFavorite: false,
    ...over,
  } as CachedPhoto;
}

describe("countsFromSummary", () => {
  it("reads the collapsed tile counts, never the raw row counts", () => {
    // Raw and collapsed deliberately disagree: 14874 rows -> 13350 tiles, and
    // photos+gifs raw (12322+1854) is NOT smart_photos.
    const c = countsFromSummary(
      summary({
        total: 14874,
        collapsed_total: 13350,
        photos: 12322,
        gifs: 1854,
        videos: 698,
        favorites: 37,
        smart_photos: 12800,
        smart_gifs: 1800,
        smart_videos: 698,
        smart_favorites: 30,
        smart_recent: 100,
      })
    );

    expect(c).toEqual({
      all: 13350,
      recent: 100,
      favorites: 30,
      photos: 12800,
      gifs: 1800,
      videos: 698,
      audio: 0,
    });
    // The raw numbers must not leak through anywhere.
    expect(c?.all).not.toBe(14874);
    expect(c?.photos).not.toBe(12322 + 1854);
  });

  it("returns null for a server that predates the smart_* fields", () => {
    // A pre-#42 binary, or a summary persisted to localStorage before the
    // change. Reading through would paint `undefined` into every badge.
    const stale = { total: 100, collapsed_total: 90, photos: 80, gifs: 5, videos: 5, audio: 0, favorites: 3 };
    expect(countsFromSummary(stale as PhotoSummary)).toBeNull();
  });

  it("returns null when there is no summary at all", () => {
    expect(countsFromSummary(null)).toBeNull();
  });
});

describe("countsFromMirror", () => {
  it("collapses bursts rather than counting frames", () => {
    const c = countsFromMirror([
      photo({ burstId: "b1" }),
      photo({ burstId: "b1" }),
      photo({ burstId: "b1" }),
      photo(),
    ]);
    expect(c?.all).toBe(2); // one burst tile + one loose photo
    expect(c?.photos).toBe(2);
  });

  it("filters BEFORE collapsing, so a part-favourited burst is one tile", () => {
    // The ordering trap. Collapsing first would count the burst as favourited
    // or not based on whichever frame happened to survive the collapse.
    const c = countsFromMirror([
      photo({ burstId: "b1", isFavorite: true }),
      photo({ burstId: "b1", isFavorite: true }),
      photo({ burstId: "b1", isFavorite: false }),
      photo({ isFavorite: true }),
    ]);
    expect(c?.favorites).toBe(2); // the burst (once) + the loose favourite
  });

  it("counts gifs into Photos, matching the smart-album definition", () => {
    const c = countsFromMirror([
      photo({ mediaType: "photo" }),
      photo({ mediaType: "gif" }),
      photo({ mediaType: "video" }),
    ]);
    expect(c?.photos).toBe(2);
    expect(c?.gifs).toBe(1);
    expect(c?.videos).toBe(1);
  });

  it("caps Recently Added at the album's own limit", () => {
    const many = Array.from({ length: 150 }, () => photo());
    expect(countsFromMirror(many)?.recent).toBe(100);
    expect(countsFromMirror(many)?.all).toBe(150);
  });

  it("returns null for an empty or absent mirror", () => {
    expect(countsFromMirror([])).toBeNull();
    expect(countsFromMirror(undefined)).toBeNull();
  });
});

describe("resolveSmartAlbumCounts", () => {
  it("prefers the server summary over a populated mirror", () => {
    // The regression this fixes: precedence used to be `local ?? summary`, so a
    // mirror holding even one row won and the authoritative count was never
    // seen. The mirror here is short by the pending-encryption backlog.
    const counts = resolveSmartAlbumCounts(
      summary({ collapsed_total: 13350, smart_photos: 12800, smart_recent: 100 }),
      [photo(), photo()]
    );
    expect(counts?.all).toBe(13350);
    expect(counts?.all).not.toBe(2);
  });

  it("falls back to the mirror when the summary is unavailable", () => {
    const counts = resolveSmartAlbumCounts(null, [photo(), photo()]);
    expect(counts?.all).toBe(2);
  });

  it("falls back to the mirror when the server predates smart_* fields", () => {
    const stale = { total: 9, collapsed_total: 9, photos: 9, gifs: 0, videos: 0, audio: 0, favorites: 0 };
    const counts = resolveSmartAlbumCounts(stale as PhotoSummary, [photo(), photo()]);
    expect(counts?.all).toBe(2);
  });

  it("returns null when neither source has anything", () => {
    expect(resolveSmartAlbumCounts(null, undefined)).toBeNull();
  });
});
