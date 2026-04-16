/**
 * Thumbnail generation, dimension/duration extraction, fallback thumbnails,
 * and canvas-based image edit application.
 */

// ── Thumbnail generation ──────────────────────────────────────────────────────

/** Generate a JPEG thumbnail from raw image data.
 *  Preserves the original aspect ratio — the longest edge is scaled to `size`
 *  pixels so the justified grid can display portrait thumbnails without
 *  excessive cropping.
 */
function generateImageThumbnailFromBuffer(
  data: ArrayBuffer,
  mimeType: string,
  size: number
): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const blob = new Blob([data], { type: mimeType });
    const img = new Image();
    const url = URL.createObjectURL(blob);
    img.onload = () => {
      URL.revokeObjectURL(url);
      // Fit within `size` — scale so the longest edge = size
      const scale = Math.min(size / img.width, size / img.height);
      const tw = Math.round(img.width * scale);
      const th = Math.round(img.height * scale);
      const canvas = document.createElement("canvas");
      canvas.width = tw;
      canvas.height = th;
      const ctx = canvas.getContext("2d")!;
      ctx.drawImage(img, 0, 0, tw, th);
      canvas.toBlob(
        (blob) =>
          blob
            ? blob.arrayBuffer().then(resolve)
            : reject(new Error("Canvas toBlob failed")),
        "image/jpeg",
        0.8
      );
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Image load failed"));
    };
    img.src = url;
  });
}

/** Generate a JPEG thumbnail from raw video data (seek to 10%) */
function generateVideoThumbnailFromBuffer(
  data: ArrayBuffer,
  mimeType: string,
  size: number
): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const blob = new Blob([data], { type: mimeType });
    const video = document.createElement("video");
    video.muted = true;
    video.playsInline = true;
    const url = URL.createObjectURL(blob);
    video.onloadedmetadata = () => {
      video.currentTime = Math.min(
        Math.max(video.duration * 0.1, 1),
        video.duration
      );
    };
    video.onseeked = () => {
      URL.revokeObjectURL(url);
      // Fit within `size` — scale so the longest edge = size
      const scale = Math.min(size / video.videoWidth, size / video.videoHeight);
      const tw = Math.round(video.videoWidth * scale);
      const th = Math.round(video.videoHeight * scale);
      const canvas = document.createElement("canvas");
      canvas.width = tw;
      canvas.height = th;
      const ctx = canvas.getContext("2d")!;
      ctx.drawImage(video, 0, 0, tw, th);
      canvas.toBlob(
        (blob) =>
          blob
            ? blob.arrayBuffer().then(resolve)
            : reject(new Error("Canvas toBlob failed")),
        "image/jpeg",
        0.8
      );
    };
    video.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Video load failed"));
    };
    video.src = url;
  });
}

/** Generate a thumbnail from either image or video raw data */
export function generateThumbnailFromBuffer(
  data: ArrayBuffer,
  mimeType: string,
  size: number
): Promise<ArrayBuffer> {
  if (mimeType.startsWith("audio/"))
    return Promise.reject(new Error("Audio files have no visual thumbnail"));
  if (mimeType.startsWith("video/"))
    return generateVideoThumbnailFromBuffer(data, mimeType, size);
  return generateImageThumbnailFromBuffer(data, mimeType, size);
}

// ── Dimension & duration extraction ───────────────────────────────────────────

/** Get image/video dimensions from raw data */
export function getDimensionsFromBuffer(
  data: ArrayBuffer,
  mimeType: string
): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    const blob = new Blob([data], { type: mimeType });
    const url = URL.createObjectURL(blob);

    if (mimeType.startsWith("audio/")) {
      // Audio files have no visual dimensions
      resolve({ width: 0, height: 0 });
    } else if (mimeType.startsWith("video/")) {
      const video = document.createElement("video");
      video.onloadedmetadata = () => {
        URL.revokeObjectURL(url);
        resolve({ width: video.videoWidth, height: video.videoHeight });
      };
      video.onerror = () => {
        URL.revokeObjectURL(url);
        resolve({ width: 0, height: 0 });
      };
      video.src = url;
    } else {
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
    }
  });
}

/** Get video duration from raw data */
export function getVideoDurationFromBuffer(
  data: ArrayBuffer,
  mimeType: string
): Promise<number> {
  return new Promise((resolve) => {
    const blob = new Blob([data], { type: mimeType });
    const video = document.createElement("video");
    const url = URL.createObjectURL(blob);
    video.onloadedmetadata = () => {
      URL.revokeObjectURL(url);
      resolve(video.duration);
    };
    video.onerror = () => {
      URL.revokeObjectURL(url);
      resolve(0);
    };
    video.src = url;
  });
}

// ── Fallback thumbnails ──────────────────────────────────────────────────────

/** Create a gray placeholder thumbnail when generation fails */
export async function createFallbackThumbnail(): Promise<ArrayBuffer> {
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 256;
  const ctx = canvas.getContext("2d")!;
  const grad = ctx.createLinearGradient(0, 0, 256, 256);
  grad.addColorStop(0, "#555");
  grad.addColorStop(1, "#333");
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, 256, 256);
  ctx.fillStyle = "#777";
  ctx.font = "60px sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("\uD83D\uDCF7", 128, 128);
  return new Promise((resolve) => {
    canvas.toBlob(
      (blob) => blob!.arrayBuffer().then(resolve),
      "image/jpeg",
      0.5
    );
  });
}

/**
 * Apply crop/brightness/rotation edits to an existing JPEG thumbnail,
 * producing a new 256×256 JPEG ArrayBuffer.
 *
 * Used when saving an edited copy in encrypted mode so the gallery
 * thumbnail reflects the user's edits instead of the unedited original.
 */
export function applyEditsToThumbnail(
  thumbnailData: ArrayBuffer,
  crop: { x: number; y: number; width: number; height: number; rotate?: number; brightness?: number },
): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const blob = new Blob([thumbnailData], { type: "image/jpeg" });
    const img = new Image();
    const url = URL.createObjectURL(blob);
    img.onload = () => {
      URL.revokeObjectURL(url);
      const SIZE = 256;
      const canvas = document.createElement("canvas");
      canvas.width = SIZE;
      canvas.height = SIZE;
      const ctx = canvas.getContext("2d")!;

      // Source region (crop expressed as 0–1 fractions)
      const sx = crop.x * img.width;
      const sy = crop.y * img.height;
      const sw = crop.width * img.width;
      const sh = crop.height * img.height;

      // Fit the cropped region into SIZE×SIZE (cover)
      const scale = Math.max(SIZE / sw, SIZE / sh);
      const dw = sw * scale;
      const dh = sh * scale;

      ctx.save();
      // Rotation around center
      if (crop.rotate) {
        ctx.translate(SIZE / 2, SIZE / 2);
        ctx.rotate((crop.rotate * Math.PI) / 180);
        ctx.translate(-SIZE / 2, -SIZE / 2);
      }
      // Brightness via compositing: lighten (positive) or darken (negative)
      ctx.drawImage(img, sx, sy, sw, sh, (SIZE - dw) / 2, (SIZE - dh) / 2, dw, dh);
      if (crop.brightness && crop.brightness !== 0) {
        const b = Math.round(Math.abs(crop.brightness) * 2.55); // 0–100 → 0–255
        ctx.globalCompositeOperation = crop.brightness > 0 ? "lighter" : "multiply";
        ctx.fillStyle = crop.brightness > 0
          ? `rgba(255,255,255,${b / 255})`
          : `rgba(0,0,0,${b / 255})`;
        ctx.fillRect(0, 0, SIZE, SIZE);
      }
      ctx.restore();

      canvas.toBlob(
        (b) =>
          b ? b.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed")),
        "image/jpeg",
        0.85,
      );
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Failed to load thumbnail image"));
    };
    img.src = url;
  });
}

/** Create a black placeholder thumbnail with a music note for audio files */
export async function createAudioFallbackThumbnail(): Promise<ArrayBuffer> {
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 256;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, 256, 256);
  ctx.fillStyle = "#888";
  ctx.font = "80px sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("\u266B", 128, 128);
  return new Promise((resolve) => {
    canvas.toBlob(
      (blob) => blob!.arrayBuffer().then(resolve),
      "image/jpeg",
      0.5
    );
  });
}

/**
 * Bake crop/rotation/brightness edits into a full-resolution image and return
 * a Blob ready for download.  Runs entirely in the browser via Canvas 2D — no
 * server round-trip or ffmpeg required.
 *
 * Strategy:
 *  1. Draw only the cropped region (fractional 0-1 coords) at its natural pixel
 *     size onto an intermediate canvas, applying `brightness()` via ctx.filter.
 *  2. If rotation is non-zero, transfer the intermediate canvas to a second one
 *     that has its axes pre-rotated so the output is correctly upright.
 *
 * Only suitable for image types (photo / gif rendered as JPEG).
 * For video/audio the caller should skip this and download the raw file.
 */
export function applyEditsToImageDownload(
  imageUrl: string,
  crop: { x: number; y: number; width: number; height: number; rotate?: number; brightness?: number },
  outputMime: string = "image/jpeg",
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const rot = ((crop.rotate ?? 0) % 360 + 360) % 360;

      // Source region in natural pixel coordinates
      const sx = Math.round(crop.x * img.naturalWidth);
      const sy = Math.round(crop.y * img.naturalHeight);
      const sw = Math.round(crop.width * img.naturalWidth);
      const sh = Math.round(crop.height * img.naturalHeight);

      // Step 1: crop + brightness onto an sw×sh canvas
      const cropCanvas = document.createElement("canvas");
      cropCanvas.width = sw;
      cropCanvas.height = sh;
      const cropCtx = cropCanvas.getContext("2d")!;
      if (crop.brightness && crop.brightness !== 0) {
        // Canvas filter API: brightness(1.0) = no change; 100-point scale → percentage
        cropCtx.filter = `brightness(${1 + crop.brightness / 100})`;
      }
      cropCtx.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);
      cropCtx.filter = "none";

      if (!rot) {
        cropCanvas.toBlob(
          (b) => (b ? resolve(b) : reject(new Error("Canvas toBlob failed"))),
          outputMime.startsWith("image/png") ? "image/png" : "image/jpeg",
          0.93,
        );
        return;
      }

      // Step 2: rotate.  For 90° / 270°, the output canvas axes swap.
      const isSwapped = rot === 90 || rot === 270;
      const outW = isSwapped ? sh : sw;
      const outH = isSwapped ? sw : sh;
      const rotCanvas = document.createElement("canvas");
      rotCanvas.width = outW;
      rotCanvas.height = outH;
      const rotCtx = rotCanvas.getContext("2d")!;
      rotCtx.translate(outW / 2, outH / 2);
      rotCtx.rotate((rot * Math.PI) / 180);
      // After rotation the source is centred on the new origin
      rotCtx.drawImage(cropCanvas, -sw / 2, -sh / 2);

      rotCanvas.toBlob(
        (b) => (b ? resolve(b) : reject(new Error("Canvas toBlob failed"))),
        outputMime.startsWith("image/png") ? "image/png" : "image/jpeg",
        0.93,
      );
    };
    img.onerror = () => reject(new Error("applyEditsToImageDownload: failed to load image"));
    img.src = imageUrl;
  });
}
