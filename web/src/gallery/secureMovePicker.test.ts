import { describe, it, expect } from "vitest";
import {
  otherSecureAlbumItems,
  resolveSecureMoves,
  expandSecureSelection,
  expandBurstStacks,
  collapseBurstStacks,
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

describe("expandBurstStacks (TODO.md S2)", () => {
  it("removing a burst's cover tile removes every frame, not just the cover", () => {
    // The trash button sits on the collapsed tile; before this, only "b1" was
    // removed and b2/b3 stayed behind in the album.
    const out = expandBurstStacks(burstItems, [burstItems[1]]);
    expect(out.map((i) => i.id).sort()).toEqual(["b1", "b2", "b3"]);
  });

  it("leaves non-burst targets alone and never duplicates", () => {
    const out = expandBurstStacks(burstItems, [burstItems[0], burstItems[4], burstItems[0]]);
    expect(out.map((i) => i.id)).toEqual(["a", "c"]);
  });

  it("is scoped to the owning album — a smart view must not reach into another album", () => {
    const mixed: BurstMovableItem[] = [
      { id: "x1", gallery_id: "g1", burst_id: "same" },
      { id: "x2", gallery_id: "g1", burst_id: "same" },
      { id: "y1", gallery_id: "g2", burst_id: "same" },
    ];
    const out = expandBurstStacks(mixed, [mixed[0]]);
    expect(out.map((i) => i.id).sort()).toEqual(["x1", "x2"]);
  });
});

describe("the pull picker carries whole bursts (TODO.md S5)", () => {
  const pool: BurstMovableItem[] = [
    { id: "p", gallery_id: "g2", burst_id: null },
    { id: "s1", gallery_id: "g2", burst_id: "stack" },
    { id: "s2", gallery_id: "g2", burst_id: "stack" },
    { id: "s3", gallery_id: "g2", burst_id: "stack" },
    { id: "q", gallery_id: "g3", burst_id: null },
  ];

  it("draws one tile per stack, in order", () => {
    expect(collapseBurstStacks(pool).map((i) => i.id)).toEqual(["p", "s1", "q"]);
  });

  it("collapses per album, so the same burst id in two albums stays two tiles", () => {
    const twoAlbums: BurstMovableItem[] = [
      { id: "a1", gallery_id: "g2", burst_id: "b" },
      { id: "b1", gallery_id: "g3", burst_id: "b" },
    ];
    expect(collapseBurstStacks(twoAlbums).map((i) => i.id)).toEqual(["a1", "b1"]);
  });

  it("moving the stack's tile moves every frame from its source album", () => {
    // Android moved only the cover here, splitting the burst across two albums.
    const moves = resolveSecureMoves(pool, ["s1"]);
    expect(moves.map((m) => m.itemId).sort()).toEqual(["s1", "s2", "s3"]);
    expect(new Set(moves.map((m) => m.sourceGalleryId))).toEqual(new Set(["g2"]));
  });

  it("picking a frame twice, or a frame and its cover, moves each frame once", () => {
    const moves = resolveSecureMoves(pool, ["s1", "s2", "s1"]);
    expect(moves.map((m) => m.itemId).sort()).toEqual(["s1", "s2", "s3"]);
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
