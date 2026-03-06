import type { GooglePhotosMetadata, ImportItem } from "./importTypes";

// ── Binary utilities ──────────────────────────────────────────────────────────

/** Efficient base64 encoding using chunked approach to prevent O(n²) string concat */
export function arrayBufferToBase64(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  const CHUNK = 32768;
  const parts: string[] = [];
  for (let i = 0; i < bytes.byteLength; i += CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.byteLength));
    parts.push(String.fromCharCode(...slice));
  }
  return btoa(parts.join(""));
}

/** Decode a base64 string into an ArrayBuffer */
export function base64ToArrayBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

/** Decode a base64 string into a Uint8Array */
export function base64ToUint8Array(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

// ── Thumbnail generation ──────────────────────────────────────────────────────

/** Generate a JPEG thumbnail from raw image data */
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
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d")!;
      const scale = Math.max(size / img.width, size / img.height);
      const w = img.width * scale;
      const h = img.height * scale;
      ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
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
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d")!;
      const scale = Math.max(size / video.videoWidth, size / video.videoHeight);
      const w = video.videoWidth * scale;
      const h = video.videoHeight * scale;
      ctx.drawImage(video, (size - w) / 2, (size - h) / 2, w, h);
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

/**
 * Migration-safe wrapper: generates a thumbnail but returns null instead of
 * throwing on failure. Used during encryption migration and similar batch jobs.
 */
export async function generateMigrationThumbnail(
  fileData: Uint8Array | ArrayBuffer,
  mimeType: string,
  size: number
): Promise<ArrayBuffer | null> {
  try {
    return await generateThumbnailFromBuffer(
      fileData instanceof Uint8Array ? (fileData.buffer as ArrayBuffer) : fileData,
      mimeType,
      size
    );
  } catch {
    return null;
  }
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
        resolve({ width: img.width, height: img.height });
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

// ── MIME type guessing ────────────────────────────────────────────────────────

export function guessMimeFromName(name: string): string {
  const ext = name.split(".").pop()?.toLowerCase();
  const mimeMap: Record<string, string> = {
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    png: "image/png",
    gif: "image/gif",
    webp: "image/webp",
    avif: "image/avif",
    heic: "image/heic",
    heif: "image/heif",
    bmp: "image/bmp",
    tiff: "image/tiff",
    tif: "image/tiff",
    svg: "image/svg+xml",
    ico: "image/x-icon",
    cur: "image/x-icon",
    hdr: "image/vnd.radiance",
    mp4: "video/mp4",
    mov: "video/quicktime",
    mkv: "video/x-matroska",
    webm: "video/webm",
    avi: "video/x-msvideo",
    "3gp": "video/3gpp",
    m4v: "video/x-m4v",
    wmv: "video/x-ms-wmv",
    asf: "video/x-ms-asf",
        h264: "video/h264",
        mpg: "video/mpeg",
    mpeg: "video/mpeg",
    mp3: "audio/mpeg",
    aiff: "audio/aiff",
    flac: "audio/flac",
    ogg: "audio/ogg",
    wav: "audio/wav",
    wma: "audio/x-ms-wma",
  };
  return mimeMap[ext || ""] || "application/octet-stream";
}

// ── Google Photos metadata matching ───────────────────────────────────────────

/**
 * Match Google Photos JSON metadata files to their media files.
 */
export function matchMetadataToFiles(
  mediaFiles: File[],
  jsonFiles: Map<string, GooglePhotosMetadata>
): ImportItem[] {
  return mediaFiles.map((file) => {
    let meta = jsonFiles.get(file.name);
    if (!meta) {
      for (const [, m] of jsonFiles) {
        if (m.title === file.name) {
          meta = m;
          break;
        }
      }
    }
    if (!meta) {
      const baseName = file.name.replace(/\.[^.]+$/, "");
      meta = jsonFiles.get(baseName);
    }
    return {
      file,
      name: file.name,
      size: file.size,
      mimeType: file.type || guessMimeFromName(file.name),
      metadata: meta,
      metadataFile: meta ? file.name + ".json" : undefined,
      status: "pending" as const,
    };
  });
}

// ── Formatting ────────────────────────────────────────────────────────────────

/** Format bytes to human-readable string */
export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

// ── Fallback thumbnail ───────────────────────────────────────────────────────

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
