import { describe, it, expect } from "vitest";
import {
  otherSecureAlbumItems,
  resolveSecureMoves,
  expandSecureSelection,
  secureMoveTargets,
  type MovableSecureItem,
  type BurstMovableItem,
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

// ── Push direction (#43) ─────────────────────────────────────────────────────

const burstItems: BurstMovableItem[] = [
  { id: "a", gallery_id: "g1", burst_id: null },
  { id: "b1", gallery_id: "g1", burst_id: "burst-1" }, // representative
  { id: "b2", gallery_id: "g1", burst_id: "burst-1" },
  { id: "b3", gallery_id: "g1", burst_id: "burst-1" },
  { id: "c", gallery_id: "g2", burst_id: null },
];

describe("expandSecureSelection", () => {
  it("pulls in every frame of a selected burst, not just the representative", () => {
    // The grid only ever exposes the representative "b1" as a tile.
    const expanded = expandSecureSelection(burstItems, ["b1"]);
    expect(expanded).toEqual(new Set(["b1", "b2", "b3"]));
  });

  it("leaves non-burst selections untouched", () => {
    expect(expandSecureSelection(burstItems, ["a", "c"])).toEqual(new Set(["a", "c"]));
  });

  it("mixes burst and non-burst selections", () => {
    expect(expandSecureSelection(burstItems, ["a", "b1"])).toEqual(
      new Set(["a", "b1", "b2", "b3"]),
    );
  });

  it("keeps an id with no matching item so downstream resolving can drop it", () => {
    expect(expandSecureSelection(burstItems, ["zzz"])).toEqual(new Set(["zzz"]));
  });
});

// `planSecureMovesToTarget`'s three tests were deleted with it (Z1f). They
// tested a MOVE plan the push stopped using when Z1 switched to adds — green,
// and proving only that dead code still worked. `planSecureAddsToTarget` below
// is the live path and carries the equivalent coverage.

describe("secureMoveTargets", () => {
  const albums = [
    { id: "g1", name: "One" },
    { id: "g2", name: "Two" },
    { id: "g3", name: "Three" },
  ];

  it("excludes the open real album", () => {
    expect(secureMoveTargets(albums, "g2").map((g) => g.id)).toEqual(["g1", "g3"]);
  });

  it("offers every album for a synthetic smart view (id matches nothing)", () => {
    expect(secureMoveTargets(albums, "secure-smart-videos").map((g) => g.id)).toEqual([
      "g1",
      "g2",
      "g3",
    ]);
  });
});
