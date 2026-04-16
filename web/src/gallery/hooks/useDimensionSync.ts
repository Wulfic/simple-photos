/**
 * Dimension correction utilities and batched server update manager.
 *
 * Centralizes the orientation self-healing logic that was previously
 * scattered across useGalleryData (3 locations) and Gallery.tsx (1 location).
 *
 * The browser auto-applies EXIF orientation when decoding images, so
 * `img.naturalWidth/Height` reflect the true display orientation.  When
 * these disagree with server-stored width/height, the stored values are
 * swapped (portrait↔landscape) and pushed to the server.
 */
import { api } from "../../api/client";
import { db } from "../../db";

// ── Pure orientation utilities ────────────────────────────────────────────────

/**
 * Detect if thumbnail orientation disagrees with stored dimensions.
 * Returns true when one is portrait and the other is landscape.
 * Ignores square thumbnails (ambiguous orientation).
 */
export function detectOrientationSwap(
  thumbW: number,
  thumbH: number,
  storedW: number,
  storedH: number,
): boolean {
  if (thumbW <= 0 || thumbH <= 0 || storedW <= 0 || storedH <= 0) return false;
  if (thumbW === thumbH) return false;
  return (thumbW > thumbH) !== (storedW > storedH);
}

/**
 * Check if two dimension pairs are transposed (swapped width↔height).
 * Used to detect when the client has already corrected an EXIF mismatch
 * and the server response is stale.
 */
export function isTransposed(
  w1: number,
  h1: number,
  w2: number,
  h2: number,
): boolean {
  return w1 === h2 && h1 === w2;
}

/**
 * Correct dimensions if thumbnail orientation disagrees with stored values.
 * Returns corrected `{ width, height }` or `null` if no correction needed.
 */
export function correctDimensionsFromThumbnail(
  thumbW: number,
  thumbH: number,
  storedW: number,
  storedH: number,
): { width: number; height: number } | null {
  if (!detectOrientationSwap(thumbW, thumbH, storedW, storedH)) return null;
  return { width: storedH, height: storedW };
}

// ── Batched server dimension update ───────────────────────────────────────────

interface DimensionEntry {
  photo_id: string;
  width: number;
  height: number;
}

const pendingUpdates = new Map<string, DimensionEntry>();
let flushTimer: ReturnType<typeof setTimeout> | null = null;
const BATCH_DEBOUNCE_MS = 500;
const BATCH_MAX = 50;

function flushPendingUpdates() {
  if (pendingUpdates.size === 0) return;
  const batch = Array.from(pendingUpdates.values()).slice(0, BATCH_MAX);
  // If there are more than BATCH_MAX, they'll be flushed next cycle
  for (const entry of batch) pendingUpdates.delete(entry.photo_id);
  flushTimer = null;
  api.photos.batchUpdateDimensions(batch).catch(() => {
    /* non-fatal — dimensions will be re-corrected on next sync */
  });
}

/**
 * Queue a dimension correction for server-side push.
 * Updates are debounced and batched (500ms debounce, max 50 per batch).
 */
export function queueDimensionUpdate(
  serverPhotoId: string,
  width: number,
  height: number,
) {
  pendingUpdates.set(serverPhotoId, {
    photo_id: serverPhotoId,
    width,
    height,
  });
  if (pendingUpdates.size >= BATCH_MAX) {
    if (flushTimer) clearTimeout(flushTimer);
    flushPendingUpdates();
  } else {
    if (flushTimer) clearTimeout(flushTimer);
    flushTimer = setTimeout(flushPendingUpdates, BATCH_DEBOUNCE_MS);
  }
}

// ── Combined IDB + server correction ──────────────────────────────────────────

/**
 * Apply a dimension correction to both IDB and server.
 * Called from Gallery.tsx's onDimensionMismatch callback and
 * from useGalleryData during sync.
 */
export function applyDimensionCorrection(
  blobId: string,
  serverPhotoId: string | undefined,
  correctedW: number,
  correctedH: number,
) {
  db.photos.update(blobId, { width: correctedW, height: correctedH });
  if (serverPhotoId) {
    queueDimensionUpdate(serverPhotoId, correctedW, correctedH);
  }
}
