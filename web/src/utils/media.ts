/**
 * Media utilities — barrel re-export from focused submodules, plus MIME type
 * guessing and Google Photos metadata matching.
 */
import type { GooglePhotosMetadata, ImportItem } from "./importTypes";

// Re-export everything from submodules so existing `from "../utils/media"` imports keep working
export { arrayBufferToBase64, base64ToArrayBuffer, base64ToUint8Array } from "./encoding";
export {
  generateThumbnailFromBuffer,
  getDimensionsFromBuffer,
  getVideoDurationFromBuffer,
  createFallbackThumbnail,
  applyEditsToThumbnail,
  createAudioFallbackThumbnail,
  applyEditsToImageDownload,
} from "./thumbnails";

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
    bmp: "image/bmp",
    ico: "image/x-icon",
    mp4: "video/mp4",
    webm: "video/webm",
    mp3: "audio/mpeg",
    flac: "audio/flac",
    ogg: "audio/ogg",
    wav: "audio/wav",
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
