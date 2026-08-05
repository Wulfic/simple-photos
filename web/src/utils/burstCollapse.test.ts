import { describe, it, expect } from "vitest";
import { collapseBursts } from "./burstCollapse";
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

describe("collapseBursts", () => {
  it("passes non-burst photos through untouched", () => {
    const input = [photo("a"), photo("b"), photo("c")];
    const out = collapseBursts(input);
    expect(out.map((p) => p.blobId)).toEqual(["a", "b", "c"]);
    expect(out.every((p) => p._burstCount === undefined)).toBe(true);
  });

  it("collapses a burst group to its first frame and stamps _burstCount", () => {
    const input = [
      photo("rep", { burstId: "g1" }),
      photo("f2", { burstId: "g1" }),
      photo("f3", { burstId: "g1" }),
    ];
    const out = collapseBursts(input);
    expect(out).toHaveLength(1);
    expect(out[0].blobId).toBe("rep");
    expect(out[0]._burstCount).toBe(3);
  });

  it("preserves input order using the representative's position", () => {
    const input = [
      photo("a"),
      photo("rep", { burstId: "g1" }),
      photo("f2", { burstId: "g1" }),
      photo("b"),
    ];
    const out = collapseBursts(input);
    expect(out.map((p) => p.blobId)).toEqual(["a", "rep", "b"]);
  });

  it("handles multiple independent burst groups", () => {
    const input = [
      photo("g1a", { burstId: "g1" }),
      photo("g2a", { burstId: "g2" }),
      photo("g1b", { burstId: "g1" }),
      photo("solo"),
      photo("g2b", { burstId: "g2" }),
    ];
    const out = collapseBursts(input);
    expect(out.map((p) => p.blobId)).toEqual(["g1a", "g2a", "solo"]);
    expect(out.find((p) => p.blobId === "g1a")?._burstCount).toBe(2);
    expect(out.find((p) => p.blobId === "g2a")?._burstCount).toBe(2);
  });

  it("does not mutate the input objects (no stale badge)", () => {
    const rep = photo("rep", { burstId: "g1" });
    collapseBursts([rep, photo("f2", { burstId: "g1" })]);
    expect((rep as { _burstCount?: number })._burstCount).toBeUndefined();
  });
});
