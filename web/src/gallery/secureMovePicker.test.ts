import { describe, it, expect } from "vitest";
import {
  otherSecureAlbumItems,
  resolveSecureMoves,
  type MovableSecureItem,
} from "./secureMovePicker";

const items: MovableSecureItem[] = [
  { id: "a", gallery_id: "g1" },
  { id: "b", gallery_id: "g2" },
  { id: "c", gallery_id: "g2" },
  { id: "d", gallery_id: null }, // older row, no owning gallery
  { id: "e", gallery_id: "g1" },
];

describe("otherSecureAlbumItems", () => {
  it("excludes the open album's own items", () => {
    const pool = otherSecureAlbumItems(items, "g1");
    expect(pool.map((i) => i.id)).toEqual(["b", "c"]);
  });

  it("excludes items with no owning gallery id (can't route a move)", () => {
    const pool = otherSecureAlbumItems(items, "g2");
    // a + e are from g1; d has no gallery_id → dropped
    expect(pool.map((i) => i.id)).toEqual(["a", "e"]);
  });

  it("returns empty when everything is in the open album", () => {
    expect(otherSecureAlbumItems([{ id: "a", gallery_id: "g1" }], "g1")).toEqual([]);
  });
});

describe("resolveSecureMoves", () => {
  it("maps each selected id to its source gallery + item", () => {
    const pool = otherSecureAlbumItems(items, "g1"); // b, c (both g2)
    const moves = resolveSecureMoves(pool, ["b", "c"]);
    expect(moves).toEqual([
      { sourceGalleryId: "g2", itemId: "b" },
      { sourceGalleryId: "g2", itemId: "c" },
    ]);
  });

  it("drops selections not present in the pool", () => {
    const pool = otherSecureAlbumItems(items, "g1"); // b, c
    const moves = resolveSecureMoves(pool, ["b", "zzz"]);
    expect(moves).toEqual([{ sourceGalleryId: "g2", itemId: "b" }]);
  });

  it("drops selections whose item lacks a source gallery", () => {
    const moves = resolveSecureMoves([{ id: "d", gallery_id: null }], ["d"]);
    expect(moves).toEqual([]);
  });
});
