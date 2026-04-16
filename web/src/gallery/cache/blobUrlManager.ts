/**
 * Centralised blob URL lifecycle manager.
 *
 * Tracks every `URL.createObjectURL` / `URL.revokeObjectURL` call so
 * multiple components can safely share a blob URL.  A URL is only
 * revoked when the last reference is released.
 */

interface UrlEntry {
  url: string;
  refCount: number;
}

class BlobUrlManager {
  private urls = new Map<string, UrlEntry>();

  /**
   * Create (or reuse) a blob URL for `data`.
   * If `key` already has a live URL the existing one is returned and
   * the reference count is incremented.
   */
  acquire(key: string, data: ArrayBuffer | Blob, mimeType: string): string {
    const existing = this.urls.get(key);
    if (existing) {
      existing.refCount++;
      return existing.url;
    }
    const blob = data instanceof Blob ? data : new Blob([data], { type: mimeType });
    const url = URL.createObjectURL(blob);
    this.urls.set(key, { url, refCount: 1 });
    return url;
  }

  /**
   * Decrement the reference count for `key`.
   * When it reaches 0 the blob URL is revoked and the entry removed.
   */
  release(key: string): void {
    const entry = this.urls.get(key);
    if (!entry) return;
    entry.refCount--;
    if (entry.refCount <= 0) {
      URL.revokeObjectURL(entry.url);
      this.urls.delete(key);
    }
  }

  /** Get the URL for `key` without changing the reference count. */
  peek(key: string): string | null {
    return this.urls.get(key)?.url ?? null;
  }

  /** Current number of tracked URLs (for diagnostics). */
  get size(): number {
    return this.urls.size;
  }
}

/** Singleton blob URL manager shared across all gallery components. */
export const blobUrlManager = new BlobUrlManager();

// ── Dev-mode leak detection ──────────────────────────────────────────────────
// Logs a warning every 60s if tracked blob URLs exceed a threshold, which
// indicates a likely leak (components not calling release()).
if (typeof window !== "undefined" && window.location.hostname === "localhost") {
  const LEAK_THRESHOLD = 100;
  setInterval(() => {
    if (blobUrlManager.size > LEAK_THRESHOLD) {
      console.warn(
        `[BlobUrlManager] Possible blob URL leak: ${blobUrlManager.size} tracked URLs (threshold: ${LEAK_THRESHOLD})`,
      );
    }
  }, 60_000);
}
