/**
 * Thumbnail cache — LRU ordering, pinning, and blob-URL ownership (#51).
 *
 * The bug this suite exists to pin down: scrolling a long grid evicted entries
 * whose <img> was still mounted, and — because eviction revoked a URL that
 * `blobUrlManager` still believed it owned — the re-load path handed back the
 * revoked URL forever. Tiles blanked permanently rather than re-fetching.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ThumbnailCache } from "./thumbnailCache";

let created: string[] = [];
let revoked: string[] = [];
let counter = 0;

beforeEach(() => {
  created = [];
  revoked = [];
  counter = 0;
  vi.stubGlobal("URL", {
    createObjectURL: () => {
      const url = `blob:mock/${++counter}`;
      created.push(url);
      return url;
    },
    revokeObjectURL: (url: string) => {
      revoked.push(url);
    },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const bytes = () => new ArrayBuffer(8);

/** Fill a cache with `n` sequentially-named entries. */
function fill(cache: ThumbnailCache, n: number, prefix = "p"): void {
  for (let i = 0; i < n; i++) cache.getOrCreate(`${prefix}${i}`, bytes(), "image/jpeg");
}

describe("ThumbnailCache — LRU ordering", () => {
  it("evicts the least-recently-used entry once over capacity", () => {
    const cache = new ThumbnailCache(3);
    fill(cache, 3);
    expect(cache.size).toBe(3);

    cache.getOrCreate("p3", bytes(), "image/jpeg");

    expect(cache.size).toBe(3);
    expect(cache.has("p0")).toBe(false);
    expect(revoked).toEqual([created[0]]);
  });

  it("a get() promotes an entry so it survives the next eviction", () => {
    const cache = new ThumbnailCache(3);
    fill(cache, 3);

    cache.get("p0"); // p1 is now the LRU
    cache.getOrCreate("p3", bytes(), "image/jpeg");

    expect(cache.has("p0")).toBe(true);
    expect(cache.has("p1")).toBe(false);
  });

  it("returns the same URL for a repeated blobId instead of creating a second", () => {
    const cache = new ThumbnailCache(10);
    const first = cache.getOrCreate("p0", bytes(), "image/jpeg");
    const second = cache.getOrCreate("p0", bytes(), "image/jpeg");

    expect(second.url).toBe(first.url);
    expect(created).toHaveLength(1);
    expect(revoked).toHaveLength(0);
  });
});

describe("ThumbnailCache — pinning protects mounted tiles", () => {
  it("never revokes a pinned entry, even far past capacity", () => {
    const cache = new ThumbnailCache(5);
    const mounted = cache.getOrCreate("mounted", bytes(), "image/jpeg");
    cache.pin("mounted");

    // Scroll far past capacity — the old magic-500 cache revoked here.
    fill(cache, 200);

    expect(cache.has("mounted")).toBe(true);
    expect(revoked).not.toContain(mounted.url);
    expect(cache.get("mounted")?.url).toBe(mounted.url);
  });

  it("counts pins so a shared blob survives until the last tile unmounts", () => {
    const cache = new ThumbnailCache(2);
    const shared = cache.getOrCreate("shared", bytes(), "image/jpeg");
    cache.pin("shared");
    cache.pin("shared");

    cache.unpin("shared");
    fill(cache, 50);
    expect(revoked).not.toContain(shared.url);

    cache.unpin("shared");
    expect(cache.isPinned("shared")).toBe(false);
    fill(cache, 50, "q");
    expect(revoked).toContain(shared.url);
  });

  it("scales capacity with the pinned render window rather than a fixed 500", () => {
    const cache = new ThumbnailCache(5);
    for (let i = 0; i < 40; i++) {
      cache.getOrCreate(`m${i}`, bytes(), "image/jpeg");
      cache.pin(`m${i}`);
    }
    // 40 pinned × 3 headroom = 120 capacity, so nothing is dropped at 100.
    fill(cache, 60, "scroll");

    expect(cache.pinnedSize).toBe(40);
    expect(revoked).toHaveLength(0);
    expect(cache.size).toBe(100);
  });

  it("makes progress when only some entries are pinned", () => {
    const cache = new ThumbnailCache(4);
    cache.getOrCreate("pinned", bytes(), "image/jpeg");
    cache.pin("pinned");
    fill(cache, 20);

    expect(cache.has("pinned")).toBe(true);
    // Base 4 vs pinned 1×3 → capacity 4; unpinned entries are reclaimed.
    expect(cache.size).toBe(4);
  });
});

describe("ThumbnailCache — no dead URLs (the #51 poisoning)", () => {
  it("issues a fresh URL after eviction instead of resurrecting the revoked one", () => {
    const cache = new ThumbnailCache(2);
    const original = cache.getOrCreate("victim", bytes(), "image/jpeg");

    fill(cache, 10); // evicts "victim"
    expect(revoked).toContain(original.url);
    expect(cache.has("victim")).toBe(false);

    // Scrolling back re-loads the thumbnail. It MUST NOT be the revoked URL —
    // that is the permanent-blank-tile bug.
    const reloaded = cache.getOrCreate("victim", bytes(), "image/jpeg");
    expect(reloaded.url).not.toBe(original.url);
    expect(revoked).not.toContain(reloaded.url);
  });

  it("revoke() drops the entry so the next read re-creates it", () => {
    const cache = new ThumbnailCache(10);
    const first = cache.getOrCreate("p0", bytes(), "image/jpeg");
    cache.revoke("p0");

    expect(revoked).toEqual([first.url]);
    expect(cache.get("p0")).toBeNull();
    expect(cache.getOrCreate("p0", bytes(), "image/jpeg").url).not.toBe(first.url);
  });

  it("clear() revokes everything and drops pins", () => {
    const cache = new ThumbnailCache(10);
    fill(cache, 4);
    cache.pin("p0");
    cache.clear();

    expect(revoked).toHaveLength(4);
    expect(cache.size).toBe(0);
    expect(cache.pinnedSize).toBe(0);
  });
});
