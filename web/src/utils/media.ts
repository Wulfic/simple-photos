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

// ── Google Photos "-edited" variant handling ───────────────────────────────────

/**
 * Matches Google Photos' "-edited" suffix immediately before the file
 * extension, e.g. `IMG_1234-edited.jpg`. Case-insensitive.
 *
 * NOTE: Google localises this suffix in non-English exports (e.g. `-bearbeitet`
 * in German). We match the English suffix only — non-English exports simply
 * fall back to the pre-existing behaviour (both copies imported), so we never
 * drop a file we shouldn't.
 */
const GP_EDITED_RE = /-edited(?=\.[^.]+$)/i;

/** True when `name` looks like a Google Photos baked-in edited copy. */
export function isGooglePhotosEdited(name: string): boolean {
  return GP_EDITED_RE.test(name);
}

/**
 * Recover the original filename a Google Photos "-edited" copy derives from,
 * e.g. `IMG_1234.jpg` for `IMG_1234-edited.jpg`. Returns the input unchanged
 * when it isn't an edited variant.
 */
export function originalNameForEdited(name: string): string {
  return name.replace(GP_EDITED_RE, "");
}

/**
 * Google Photos Takeout exports BOTH the unedited original (`IMG_1234.jpg`)
 * and a separate baked-in edited copy (`IMG_1234-edited.jpg`) for every photo
 * the user edited. Importing both produces visible duplicates that the
 * server's content-hash dedup cannot catch — the two files differ in bytes.
 *
 * Keep the edited version (it has the user's edits applied) and drop the
 * original whenever both are present. Files without an edited sibling are left
 * untouched. Mirrors `server/src/setup/import.rs::dedupe_google_photos_edits`.
 */
export function dedupeGooglePhotosEdits(files: File[]): File[] {
  const originalsWithEdit = new Set<string>();
  for (const f of files) {
    if (isGooglePhotosEdited(f.name)) {
      originalsWithEdit.add(originalNameForEdited(f.name).toLowerCase());
    }
  }
  if (originalsWithEdit.size === 0) return files;
  return files.filter((f) => !originalsWithEdit.has(f.name.toLowerCase()));
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
    // Google Photos names the sidecar after the ORIGINAL file, never the
    // "-edited" copy. When the edited copy is the one we keep (see
    // dedupeGooglePhotosEdits), fall back to the original's metadata so its
    // photoTakenTime / geoData still get applied.
    if (!meta && isGooglePhotosEdited(file.name)) {
      const orig = originalNameForEdited(file.name);
      meta = jsonFiles.get(orig) ?? jsonFiles.get(orig.replace(/\.[^.]+$/, ""));
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
