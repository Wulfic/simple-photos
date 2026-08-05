import { describe, it, expect } from "vitest";
import type { SecureGalleryItem } from "../api/galleries";
import {
  SECURE_SMART_ALBUM_DEFS,
  isSecureSmartAlbum,
  secureSmartAlbumDef,
  computeSecureSmartAlbums,
  filterSecureSmartAlbum,
} from "./secureSmartAlbums";

/** Build a minimal item; `added_at` order is caller-controlled (DESC). */
function item(
  id: string,
  media_type: string | null,
  extra: Partial<SecureGalleryItem> = {}
): SecureGalleryItem {
  return {
    id,
    blob_id: `blob-${id}`,
    added_at: extra.added_at ?? `2026-07-16T00:00:00Z`,
    gallery_id: extra.gallery_id ?? "g1",
    media_type,
    ...extra,
  };
}

describe("secureSmartAlbums", () => {
  it("isSecureSmartAlbum only matches the secure-smart namespace", () => {
    expect(isSecureSmartAlbum("secure-smart-all")).toBe(true);
    expect(isSecureSmartAlbum("secure-smart-videos")).toBe(true);
    expect(isSecureSmartAlbum("smart-photos")).toBe(false); // main-gallery ns
    expect(isSecureSmartAlbum("some-real-uuid")).toBe(false);
    expect(isSecureSmartAlbum(undefined)).toBe(false);
    expect(isSecureSmartAlbum(null)).toBe(false);
    expect(isSecureSmartAlbum("")).toBe(false);
  });

  it("secureSmartAlbumDef resolves ids", () => {
    expect(secureSmartAlbumDef("secure-smart-gifs")?.label).toBe("GIFs");
    expect(secureSmartAlbumDef("nope")).toBeUndefined();
  });

  it("photos filter includes photo, gif, and NULL media_type", () => {
    const def = secureSmartAlbumDef("secure-smart-photos")!;
    expect(def.filter(item("a", "photo"))).toBe(true);
    expect(def.filter(item("b", "gif"))).toBe(true);
    expect(def.filter(item("c", null))).toBe(true);
    expect(def.filter(item("d", "video"))).toBe(false);
    expect(def.filter(item("e", "audio"))).toBe(false);
  });

  it("gifs/videos/audio filters are exact media_type matches", () => {
    const gifs = secureSmartAlbumDef("secure-smart-gifs")!;
    const videos = secureSmartAlbumDef("secure-smart-videos")!;
    const audio = secureSmartAlbumDef("secure-smart-audio")!;
    expect(gifs.filter(item("a", "gif"))).toBe(true);
    expect(gifs.filter(item("b", "photo"))).toBe(false);
    expect(gifs.filter(item("c", null))).toBe(false);
    expect(videos.filter(item("d", "video"))).toBe(true);
    expect(videos.filter(item("e", null))).toBe(false);
    expect(audio.filter(item("f", "audio"))).toBe(true);
    expect(audio.filter(item("g", null))).toBe(false);
  });

  it("all filter matches everything", () => {
    const all = secureSmartAlbumDef("secure-smart-all")!;
    expect(all.filter(item("a", "video"))).toBe(true);
    expect(all.filter(item("b", null))).toBe(true);
  });

  it("computeSecureSmartAlbums only returns non-empty albums with correct counts", () => {
    const items = [
      item("v1", "video"),
      item("p1", "photo"),
      item("g1", "gif"),
      item("p2", "photo"),
    ];
    const albums = computeSecureSmartAlbums(items);
    const byId = new Map(albums.map((a) => [a.id, a]));

    // No audio in the set → no Audio tile.
    expect(byId.has("secure-smart-audio")).toBe(false);

    expect(byId.get("secure-smart-all")!.count).toBe(4);
    // Photos = photo + gif = 3 (2 photos + 1 gif)
    expect(byId.get("secure-smart-photos")!.count).toBe(3);
    expect(byId.get("secure-smart-gifs")!.count).toBe(1);
    expect(byId.get("secure-smart-videos")!.count).toBe(1);
  });

  it("NULL media_type lands in Photos + Secure Gallery only", () => {
    const items = [item("x", null)];
    const albums = computeSecureSmartAlbums(items);
    const ids = albums.map((a) => a.id);
    expect(ids).toContain("secure-smart-all");
    expect(ids).toContain("secure-smart-photos");
    expect(ids).not.toContain("secure-smart-gifs");
    expect(ids).not.toContain("secure-smart-videos");
    expect(ids).not.toContain("secure-smart-audio");
  });

  it("cover = newest matching item (first in added_at DESC input)", () => {
    // Input is DESC: newest first.
    const items = [
      item("newest-video", "video", { added_at: "2026-07-16T10:00:00Z" }),
      item("old-photo", "photo", { added_at: "2026-07-15T10:00:00Z" }),
      item("newer-photo", "photo", { added_at: "2026-07-16T09:00:00Z" }),
    ];
    const albums = computeSecureSmartAlbums(items);
    const byId = new Map(albums.map((a) => [a.id, a]));
    expect(byId.get("secure-smart-all")!.coverItem.id).toBe("newest-video");
    // First photo encountered in DESC order is old-photo (index 1), because
    // input order is the contract — not re-sorted here.
    expect(byId.get("secure-smart-photos")!.coverItem.id).toBe("old-photo");
    expect(byId.get("secure-smart-videos")!.coverItem.id).toBe("newest-video");
  });

  it("empty input yields no albums", () => {
    expect(computeSecureSmartAlbums([])).toEqual([]);
  });

  it("filterSecureSmartAlbum returns members; unknown id → empty", () => {
    const items = [item("v", "video"), item("p", "photo")];
    expect(filterSecureSmartAlbum(items, "secure-smart-videos").map((i) => i.id)).toEqual([
      "v",
    ]);
    expect(filterSecureSmartAlbum(items, "secure-smart-all").length).toBe(2);
    expect(filterSecureSmartAlbum(items, "not-a-smart-id")).toEqual([]);
  });

  it("defs are ordered all → photos → gifs → videos → audio", () => {
    expect(SECURE_SMART_ALBUM_DEFS.map((d) => d.id)).toEqual([
      "secure-smart-all",
      "secure-smart-photos",
      "secure-smart-gifs",
      "secure-smart-videos",
      "secure-smart-audio",
    ]);
  });
});
