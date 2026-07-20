/**
 * Unified thumbnail cache — the single owner of every `thumb:` blob URL.
 *
 * Ownership is the whole point of this module. Thumbnail bytes used to be
 * turned into object URLs by `blobUrlManager` (ref-counted) while eviction here
 * called `URL.revokeObjectURL` directly. Two owners, and the one doing the
 * revoking was not the one tracking references: after an eviction the manager
 * still held a live entry pointing at a dead URL, so the next `acquire()` for
 * that blob handed the caller a revoked URL forever. The tile could never
 * recover, because a cache *miss* is what re-entered the poisoned path. This
 * cache now creates the URL it will later revoke, and nothing else may.
 *
 * Features:
 *  - O(1) LRU via `Map` insertion order (delete + re-set on access)
 *  - Pinning, so a URL is never revoked while a tile is still mounted on it
 *  - Capacity derived from the live render window rather than a magic constant
 */

interface CacheEntry {
  url: string;
  mimeType: string;
}

/**
 * Floor for the cache size. Above this, capacity tracks what is actually
 * mounted (see {@link ThumbnailCache.effectiveCapacity}) so the cache is always
 * comfortably larger than the render window it serves.
 */
const DEFAULT_BASE_CAPACITY = 500;

/**
 * Headroom multiplier over the pinned (mounted) set. Entries only become
 * evictable once roughly this many render-windows' worth of newer thumbnails
 * have been touched, which keeps scroll-back cheap.
 */
const CAPACITY_HEADROOM = 3;

class ThumbnailCache {
  /** Insertion order IS the LRU order — front is least-recently-used. */
  private map = new Map<string, CacheEntry>();
  /** blobId → number of mounted tiles currently displaying it. */
  private pins = new Map<string, number>();
  private baseCapacity: number;

  constructor(baseCapacity = DEFAULT_BASE_CAPACITY) {
    this.baseCapacity = baseCapacity;
  }

  /**
   * Capacity is a function of the render window, not a fixed number. A magic
   * 500 was unrelated to how many tiles were actually mounted, which is how
   * scrolling past 500 tiles started revoking URLs out from under live images.
   */
  private get effectiveCapacity(): number {
    return Math.max(this.baseCapacity, this.pins.size * CAPACITY_HEADROOM);
  }

  /** Move an existing key to the most-recently-used end. */
  private touch(blobId: string, entry: CacheEntry): void {
    this.map.delete(blobId);
    this.map.set(blobId, entry);
  }

  /** Get a cached thumbnail URL. Returns null on miss. */
  get(blobId: string): { url: string; mimeType: string } | null {
    const entry = this.map.get(blobId);
    if (!entry) return null;
    this.touch(blobId, entry);
    return { url: entry.url, mimeType: entry.mimeType };
  }

  /**
   * Resolve `blobId` to an object URL, creating one from `data` on a miss.
   *
   * This is the only place a `thumb:` object URL is created, which is what
   * makes {@link revoke} safe: the cache cannot hand out a URL it does not own
   * and therefore cannot hand out one it has already revoked.
   */
  getOrCreate(
    blobId: string,
    data: ArrayBuffer | Blob,
    mimeType: string,
  ): { url: string; mimeType: string } {
    const existing = this.map.get(blobId);
    if (existing) {
      this.touch(blobId, existing);
      return { url: existing.url, mimeType: existing.mimeType };
    }
    const blob = data instanceof Blob ? data : new Blob([data], { type: mimeType });
    const entry: CacheEntry = { url: URL.createObjectURL(blob), mimeType };
    this.map.set(blobId, entry);
    this.evict();
    return { url: entry.url, mimeType: entry.mimeType };
  }

  /**
   * Mark `blobId` as displayed by a mounted tile. Pinned entries are never
   * evicted. Pins are counted, because the same thumbnail can legitimately be
   * mounted by several tiles at once (a burst representative, a search hit and
   * an album tile can all reference one blob).
   */
  pin(blobId: string): void {
    this.pins.set(blobId, (this.pins.get(blobId) ?? 0) + 1);
    // Keep pinned entries at the MRU end so eviction scans do not have to walk
    // past them on every insert.
    const entry = this.map.get(blobId);
    if (entry) this.touch(blobId, entry);
  }

  /** Release one pin. The entry becomes evictable when the count reaches 0. */
  unpin(blobId: string): void {
    const count = this.pins.get(blobId);
    if (count === undefined) return;
    if (count <= 1) this.pins.delete(blobId);
    else this.pins.set(blobId, count - 1);
  }

  /** Whether a mounted tile is currently displaying this thumbnail. */
  isPinned(blobId: string): boolean {
    return this.pins.has(blobId);
  }

  /** Explicitly revoke and remove one entry, regardless of pinning. */
  revoke(blobId: string): void {
    const entry = this.map.get(blobId);
    if (entry) {
      URL.revokeObjectURL(entry.url);
      this.map.delete(blobId);
    }
  }

  /** Clear the entire cache, revoking all blob URLs. */
  clear(): void {
    for (const entry of this.map.values()) {
      URL.revokeObjectURL(entry.url);
    }
    this.map.clear();
    this.pins.clear();
  }

  /** Check whether a blobId is cached. */
  has(blobId: string): boolean {
    return this.map.has(blobId);
  }

  /** Current cache size (for diagnostics and tests). */
  get size(): number {
    return this.map.size;
  }

  /** Number of distinct pinned blobs (for diagnostics and tests). */
  get pinnedSize(): number {
    return this.pins.size;
  }

  /**
   * Evict least-recently-used entries until at capacity, skipping pinned ones.
   *
   * The old implementation sorted the whole map on every insert past capacity —
   * O(n log n) per insert for an operation an LRU does in O(1). `Map` iteration
   * yields insertion order, and {@link touch} keeps that in sync with use, so
   * the front of the iterator is exactly the eviction candidate. Deleting the
   * current entry mid-iteration is well-defined; the iterator advances to the
   * next surviving entry.
   */
  private evict(): void {
    let over = this.map.size - this.effectiveCapacity;
    if (over <= 0) return;
    for (const [key, entry] of this.map) {
      if (over <= 0) break;
      // Never pull a URL out from under a mounted <img>. If everything is
      // pinned we simply run over capacity until tiles unmount — exceeding the
      // budget is recoverable, a blanked grid is not.
      if (this.pins.has(key)) continue;
      URL.revokeObjectURL(entry.url);
      this.map.delete(key);
      over--;
    }
  }
}

export { ThumbnailCache };

/** Singleton thumbnail cache instance shared across all gallery components. */
export const thumbnailCache = new ThumbnailCache();
