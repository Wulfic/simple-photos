/**
 * Unified thumbnail generation — single entry point for creating thumbnails
 * from both File objects (upload flow) and ArrayBuffers (import/sync flow).
 *
 * Consolidates `generateThumbnail` from gallery.ts and
 * `generateThumbnailFromBuffer` from thumbnails.ts into one module.
 */
import { createFallbackThumbnail } from "../../utils/thumbnails";

// ── Constants ─────────────────────────────────────────────────────────────────

/** GIFs at or below this size are preserved as animated thumbnails.
 *  Larger GIFs get a static JPEG first-frame instead. */
export const GIF_THUMB_MAX_BYTES = 5 * 1024 * 1024;

const DEFAULT_SIZE = 512;
const JPEG_QUALITY = 0.8;

// ── Public API ────────────────────────────────────────────────────────────────

export interface ThumbnailOptions {
  /** Maximum dimension in pixels (longest edge).  Defaults to 512. */
  size?: number;
  /** MIME type of the input — required when `input` is an `ArrayBuffer`. */
  mimeType?: string;
}

export interface ThumbnailResult {
  data: ArrayBuffer;
  mimeType: string;
}

/**
 * Generate a thumbnail from any supported media input.
 *
 * - **Images** → JPEG (longest edge ≤ `size`, never upscaled)
 * - **GIFs ≤ 5 MB** → raw GIF data (preserves animation)
 * - **GIFs > 5 MB** → static JPEG first-frame
 * - **Videos** → JPEG frame captured at 10 % of duration
 * - **Audio** → gray placeholder with 📷 icon
 *
 * @param input  A `File` (from upload) or `ArrayBuffer` (from import/sync).
 * @param options  Optional size and mimeType overrides.
 */
export async function generateThumbnail(
  input: File | ArrayBuffer,
  options: ThumbnailOptions = {},
): Promise<ThumbnailResult> {
  const size = options.size ?? DEFAULT_SIZE;
  const mime = input instanceof File ? input.type : (options.mimeType ?? "image/jpeg");
  const byteLength = input instanceof File ? input.size : input.byteLength;

  // Audio → fallback placeholder
  if (mime.startsWith("audio/")) {
    return { data: await createFallbackThumbnail(), mimeType: "image/jpeg" };
  }

  // Video → capture a frame at 10 %
  if (mime.startsWith("video/")) {
    const blob = input instanceof File ? input : new Blob([input], { type: mime });
    return { data: await captureVideoFrame(blob, size), mimeType: "image/jpeg" };
  }

  // Small GIF → preserve animated data as-is
  if (mime === "image/gif" && byteLength <= GIF_THUMB_MAX_BYTES) {
    const data = input instanceof File ? await input.arrayBuffer() : input;
    return { data, mimeType: "image/gif" };
  }

  // Image (or large GIF) → static JPEG
  const blob = input instanceof File ? input : new Blob([input], { type: mime });
  return { data: await renderImageToJpeg(blob, size), mimeType: "image/jpeg" };
}

// ── Dimension helpers (moved from gallery.ts) ─────────────────────────────────

/**
 * Decode the pixel dimensions of a thumbnail stored as an `ArrayBuffer`.
 * The browser auto-applies EXIF orientation, so `naturalWidth`/`naturalHeight`
 * reflect the true display orientation.
 */
export function decodeThumbnailDimensions(
  data: ArrayBuffer,
  mimeType?: string,
): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    const blob = new Blob([data], { type: mimeType || "image/jpeg" });
    const url = URL.createObjectURL(blob);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve({ width: img.naturalWidth, height: img.naturalHeight });
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      resolve({ width: 0, height: 0 });
    };
    img.src = url;
  });
}

/**
 * Create a blob URL to preview a thumbnail `ArrayBuffer` in an `<img>` tag.
 * Caller is responsible for revoking the URL when done.
 */
export function thumbnailSrc(data: ArrayBuffer, mimeType?: string): string {
  return URL.createObjectURL(new Blob([data], { type: mimeType || "image/jpeg" }));
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/** Render an image Blob as a JPEG thumbnail (longest edge ≤ `size`, never upscaled). */
function renderImageToJpeg(source: Blob, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(source);
    img.onload = () => {
      URL.revokeObjectURL(url);
      const scale = Math.min(size / img.naturalWidth, size / img.naturalHeight, 1);
      const w = Math.round(img.naturalWidth * scale) || 1;
      const h = Math.round(img.naturalHeight * scale) || 1;
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d")!;
      ctx.drawImage(img, 0, 0, w, h);
      canvas.toBlob(
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        JPEG_QUALITY,
      );
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Image load failed"));
    };
    img.src = url;
  });
}

/** Capture a frame at 10 % of video duration and render as JPEG (longest edge ≤ `size`). */
function captureVideoFrame(source: Blob, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const video = document.createElement("video");
    video.muted = true;
    video.playsInline = true;
    const url = URL.createObjectURL(source);

    video.onloadedmetadata = () => {
      video.currentTime = Math.min(Math.max(video.duration * 0.1, 1), video.duration);
    };

    video.onseeked = () => {
      URL.revokeObjectURL(url);
      const scale = Math.min(size / video.videoWidth, size / video.videoHeight, 1);
      const w = Math.round(video.videoWidth * scale) || 1;
      const h = Math.round(video.videoHeight * scale) || 1;
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d")!;
      ctx.drawImage(video, 0, 0, w, h);
      canvas.toBlob(
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        JPEG_QUALITY,
      );
    };

    video.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Video load failed"));
    };
    video.src = url;
  });
}
