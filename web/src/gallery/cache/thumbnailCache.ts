/**
 * Unified thumbnail cache — replaces both `thumbMemoryCache` (Map) and
 * component-level `useRef` blob URL tracking.
 *
 * Features:
 *  - LRU eviction when exceeding configurable capacity
 *  - Automatic `URL.revokeObjectURL` on eviction
 *  - Thread-safe get/set for concurrent React render cycles
 */

interface CacheEntry {
  url: string;
  mimeType: string;
  /** Monotonic counter for LRU ordering */
  lastUsed: number;
}

let accessCounter = 0;

class ThumbnailCache {
  private map = new Map<string, CacheEntry>();
  private capacity: number;

  constructor(capacity = 500) {
    this.capacity = capacity;
  }

  /** Get a cached thumbnail URL.  Returns null on miss. */
  get(blobId: string): { url: string; mimeType: string } | null {
    const entry = this.map.get(blobId);
    if (!entry) return null;
    entry.lastUsed = ++accessCounter;
    return { url: entry.url, mimeType: entry.mimeType };
  }

  /** Store a thumbnail URL in the cache.
   *  If the blobId already exists the old URL is NOT revoked (same blob).
   *  Evicts LRU entries when capacity is exceeded. */
  set(blobId: string, url: string, mimeType: string): void {
    const existing = this.map.get(blobId);
    if (existing) {
      // Same blob — update URL if it changed (e.g. re-created after data change)
      if (existing.url !== url) {
        URL.revokeObjectURL(existing.url);
      }
      existing.url = url;
      existing.mimeType = mimeType;
      existing.lastUsed = ++accessCounter;
      return;
    }
    this.map.set(blobId, { url, mimeType, lastUsed: ++accessCounter });
    this._evict();
  }

  /** Explicitly revoke and remove one entry. */
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
  }

  /** Check whether a blobId is cached. */
  has(blobId: string): boolean {
    return this.map.has(blobId);
  }

  /** Current cache size (for diagnostics). */
  get size(): number {
    return this.map.size;
  }

  /** Evict least-recently-used entries until we're at capacity. */
  private _evict(): void {
    if (this.map.size <= this.capacity) return;
    // Sort entries by lastUsed ascending (oldest first)
    const entries = [...this.map.entries()].sort(
      (a, b) => a[1].lastUsed - b[1].lastUsed
    );
    const toRemove = this.map.size - this.capacity;
    for (let i = 0; i < toRemove; i++) {
      const [key, entry] = entries[i];
      URL.revokeObjectURL(entry.url);
      this.map.delete(key);
    }
  }
}

/** Singleton thumbnail cache instance shared across all gallery components. */
export const thumbnailCache = new ThumbnailCache(500);
