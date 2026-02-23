import { useState, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { api } from "../api/client";
import { db, blobTypeFromMime, mediaTypeFromMime } from "../db";
import AppHeader from "../components/AppHeader";

// ── Google Photos JSON metadata shape ─────────────────────────────────────────

interface GooglePhotosMetadata {
  title: string;
  description?: string;
  imageViews?: string;
  creationTime?: { timestamp: string; formatted: string };
  photoTakenTime?: { timestamp: string; formatted: string };
  geoData?: {
    latitude: number;
    longitude: number;
    altitude: number;
    latitudeSpan?: number;
    longitudeSpan?: number;
  };
  geoDataExif?: {
    latitude: number;
    longitude: number;
    altitude: number;
  };
  url?: string;
  googlePhotosOrigin?: Record<string, unknown>;
}

// ── A paired media file with its optional metadata ────────────────────────────

interface ImportItem {
  file: File;
  metadata?: GooglePhotosMetadata;
  metadataFile?: string;
  status: "pending" | "uploading" | "done" | "error";
  error?: string;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function arrayBufferToBase64(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

/** Generate a JPEG thumbnail from an image file */
function generateImageThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(file);
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
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        0.8
      );
    };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new Error("Image load failed")); };
    img.src = url;
  });
}

/** Generate a JPEG thumbnail from a video file (seek to 10%) */
function generateVideoThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const video = document.createElement("video");
    video.muted = true;
    video.playsInline = true;
    const url = URL.createObjectURL(file);
    video.onloadedmetadata = () => {
      video.currentTime = Math.min(Math.max(video.duration * 0.1, 1), video.duration);
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
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        0.8
      );
    };
    video.onerror = () => { URL.revokeObjectURL(url); reject(new Error("Video load failed")); };
    video.src = url;
  });
}

function generateThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  if (file.type.startsWith("video/")) return generateVideoThumbnail(file, size);
  return generateImageThumbnail(file, size);
}

function getVideoDuration(file: File): Promise<number> {
  return new Promise((resolve) => {
    const video = document.createElement("video");
    const url = URL.createObjectURL(file);
    video.onloadedmetadata = () => { URL.revokeObjectURL(url); resolve(video.duration); };
    video.onerror = () => { URL.revokeObjectURL(url); resolve(0); };
    video.src = url;
  });
}

/** Get image dimensions */
function getImageDimensions(file: File): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    if (file.type.startsWith("video/")) {
      const video = document.createElement("video");
      const url = URL.createObjectURL(file);
      video.onloadedmetadata = () => {
        URL.revokeObjectURL(url);
        resolve({ width: video.videoWidth, height: video.videoHeight });
      };
      video.onerror = () => { URL.revokeObjectURL(url); resolve({ width: 0, height: 0 }); };
      video.src = url;
    } else {
      const img = new Image();
      const url = URL.createObjectURL(file);
      img.onload = () => {
        URL.revokeObjectURL(url);
        resolve({ width: img.width, height: img.height });
      };
      img.onerror = () => { URL.revokeObjectURL(url); resolve({ width: 0, height: 0 }); };
      img.src = url;
    }
  });
}

/**
 * Match a Google Photos JSON file to its media file.
 *
 * Google Takeout naming convention:
 *   - `IMG_1234.jpg` → `IMG_1234.jpg.json`
 *   - Sometimes: `IMG_1234.json` (without extension in json name)
 *   - Edited copies: `IMG_1234-edited.jpg` → `IMG_1234-edited.jpg.json`
 */
function matchMetadataToFiles(
  mediaFiles: File[],
  jsonFiles: Map<string, GooglePhotosMetadata>
): ImportItem[] {
  return mediaFiles.map((file) => {
    // Try exact match: "photo.jpg" → "photo.jpg.json"
    let meta = jsonFiles.get(file.name);

    // Try match by title field in all metadata files
    if (!meta) {
      for (const [, m] of jsonFiles) {
        if (m.title === file.name) {
          meta = m;
          break;
        }
      }
    }

    // Try without extension: "photo.jpg" → "photo.json"
    if (!meta) {
      const baseName = file.name.replace(/\.[^.]+$/, "");
      meta = jsonFiles.get(baseName);
    }

    return {
      file,
      metadata: meta,
      metadataFile: meta ? file.name + ".json" : undefined,
      status: "pending" as const,
    };
  });
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function Import() {
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);
  const [items, setItems] = useState<ImportItem[]>([]);
  const [importing, setImporting] = useState(false);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [error, setError] = useState("");
  const [dragOver, setDragOver] = useState(false);

  // Redirect if no crypto key
  if (!hasCryptoKey()) {
    navigate("/setup");
    return null;
  }

  // ── Process dropped/selected files ──────────────────────────────────────

  const processFiles = useCallback((fileList: FileList) => {
    const allFiles = Array.from(fileList);

    // Separate media files from JSON metadata files
    const mediaFiles: File[] = [];
    const jsonFiles = new Map<string, GooglePhotosMetadata>();
    const jsonReadPromises: Promise<void>[] = [];

    for (const file of allFiles) {
      if (file.name.endsWith(".json")) {
        // Read JSON content
        const promise = file.text().then((text) => {
          try {
            const data = JSON.parse(text) as GooglePhotosMetadata;
            // Validate it looks like Google Photos metadata
            if (data.title || data.photoTakenTime || data.creationTime) {
              // Store by the filename this JSON describes (remove .json suffix)
              const mediaName = file.name.replace(/\.json$/, "");
              jsonFiles.set(mediaName, data);
              // Also store by title if available
              if (data.title) {
                jsonFiles.set(data.title, data);
              }
            }
          } catch {
            // Not valid JSON or not Google Photos metadata — ignore
          }
        });
        jsonReadPromises.push(promise);
      } else if (
        file.type.startsWith("image/") ||
        file.type.startsWith("video/") ||
        // Some browsers don't detect HEIC/HEIF MIME types
        /\.(heic|heif|avif|webp|dng|cr2|nef|arw|raw)$/i.test(file.name)
      ) {
        mediaFiles.push(file);
      }
    }

    // Wait for all JSON files to be parsed, then match them
    Promise.all(jsonReadPromises).then(() => {
      const matched = matchMetadataToFiles(mediaFiles, jsonFiles);

      if (matched.length === 0 && jsonFiles.size > 0) {
        setError(
          "Only metadata JSON files were found. Please also select the matching photo/video files."
        );
        return;
      }

      if (matched.length === 0) {
        setError("No supported media files found in the selection.");
        return;
      }

      setItems((prev) => [...prev, ...matched]);
      setError("");
    });
  }, []);

  // ── Upload all items ──────────────────────────────────────────────────────

  async function handleImport() {
    if (items.length === 0) return;

    setImporting(true);
    setError("");
    setProgress({ done: 0, total: items.length });

    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.status === "done") {
        setProgress((p) => ({ ...p, done: p.done + 1 }));
        continue;
      }

      setItems((prev) =>
        prev.map((it, idx) => (idx === i ? { ...it, status: "uploading" } : it))
      );

      try {
        await uploadWithMetadata(item);
        setItems((prev) =>
          prev.map((it, idx) => (idx === i ? { ...it, status: "done" } : it))
        );
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : "Upload failed";
        setItems((prev) =>
          prev.map((it, idx) => (idx === i ? { ...it, status: "error", error: msg } : it))
        );
      }

      setProgress((p) => ({ ...p, done: p.done + 1 }));
    }

    setImporting(false);
  }

  async function uploadWithMetadata(item: ImportItem) {
    const { file, metadata } = item;
    const arrayBuf = await file.arrayBuffer();
    const data = new Uint8Array(arrayBuf);
    const mediaType = mediaTypeFromMime(file.type || guessMimeFromName(file.name));
    const serverBlobType = blobTypeFromMime(file.type || guessMimeFromName(file.name));
    const mimeType = file.type || guessMimeFromName(file.name);

    // ── Extract metadata from Google Photos JSON ──────────────────────────
    let takenAt = new Date().toISOString();
    let latitude: number | undefined;
    let longitude: number | undefined;
    let filename = file.name;

    if (metadata) {
      // Use photoTakenTime (when camera captured it), fall back to creationTime
      if (metadata.photoTakenTime?.timestamp) {
        takenAt = new Date(parseInt(metadata.photoTakenTime.timestamp) * 1000).toISOString();
      } else if (metadata.creationTime?.timestamp) {
        takenAt = new Date(parseInt(metadata.creationTime.timestamp) * 1000).toISOString();
      }

      // Use geoData if available and non-zero
      if (metadata.geoData && (metadata.geoData.latitude !== 0 || metadata.geoData.longitude !== 0)) {
        latitude = metadata.geoData.latitude;
        longitude = metadata.geoData.longitude;
      } else if (metadata.geoDataExif && (metadata.geoDataExif.latitude !== 0 || metadata.geoDataExif.longitude !== 0)) {
        latitude = metadata.geoDataExif.latitude;
        longitude = metadata.geoDataExif.longitude;
      }

      // Use the title from metadata as the filename
      if (metadata.title) {
        filename = metadata.title;
      }
    }

    // ── Get dimensions ────────────────────────────────────────────────────
    const dims = await getImageDimensions(file);

    // ── Get video duration ────────────────────────────────────────────────
    let duration: number | undefined;
    if (mediaType === "video") {
      duration = await getVideoDuration(file);
    }

    // ── Generate thumbnail ────────────────────────────────────────────────
    const thumbnailData = await generateThumbnail(file, 256);

    // ── Upload thumbnail blob ─────────────────────────────────────────────
    const thumbPayload = JSON.stringify({
      v: 1,
      photo_blob_id: "",
      width: 256,
      height: 256,
      data: arrayBufferToBase64(thumbnailData),
    });
    const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
    const thumbHash = await sha256Hex(new Uint8Array(encThumb));
    const thumbBlobType = mediaType === "video" ? "video_thumbnail" : "thumbnail";
    const thumbRes = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);

    // ── Upload media blob ─────────────────────────────────────────────────
    const photoPayload = JSON.stringify({
      v: 1,
      filename,
      taken_at: takenAt,
      mime_type: mimeType,
      media_type: mediaType,
      width: dims.width,
      height: dims.height,
      duration,
      latitude,
      longitude,
      album_ids: [],
      thumbnail_blob_id: thumbRes.blob_id,
      data: arrayBufferToBase64(data),
    });

    const encPhoto = await encrypt(new TextEncoder().encode(photoPayload));
    const photoHash = await sha256Hex(new Uint8Array(encPhoto));
    const res = await api.blobs.upload(encPhoto, serverBlobType, photoHash);

    // ── Cache in IndexedDB ────────────────────────────────────────────────
    await db.photos.put({
      blobId: res.blob_id,
      thumbnailBlobId: thumbRes.blob_id,
      filename,
      takenAt: new Date(takenAt).getTime(),
      mimeType,
      mediaType,
      width: dims.width,
      height: dims.height,
      duration,
      latitude,
      longitude,
      albumIds: [],
      thumbnailData,
    });
  }

  // ── Helpers ─────────────────────────────────────────────────────────────

  function guessMimeFromName(name: string): string {
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
      mp4: "video/mp4",
      mov: "video/quicktime",
      mkv: "video/x-matroska",
      webm: "video/webm",
      avi: "video/x-msvideo",
      "3gp": "video/3gpp",
      m4v: "video/x-m4v",
    };
    return mimeMap[ext || ""] || "application/octet-stream";
  }

  function removeItem(index: number) {
    setItems((prev) => prev.filter((_, i) => i !== index));
  }

  function clearAll() {
    setItems([]);
    setProgress({ done: 0, total: 0 });
  }

  const completedCount = items.filter((i) => i.status === "done").length;
  const errorCount = items.filter((i) => i.status === "error").length;
  const withMetadata = items.filter((i) => i.metadata).length;

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="max-w-4xl mx-auto p-4">
        <div className="mb-6">
          <h2 className="text-xl font-semibold">Import Photos</h2>
          <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
            Import photos &amp; videos with Google Photos metadata
          </p>
        </div>

      {/* ── Instructions ───────────────────────────────────────────────────── */}
      <div className="bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg p-4 mb-6">
        <h3 className="font-semibold text-blue-900 dark:text-blue-300 mb-2">
          📥 How to Import from Google Photos
        </h3>
        <ol className="text-sm text-blue-800 dark:text-blue-300 space-y-1.5 list-decimal list-inside">
          <li>
            Go to{" "}
            <a
              href="https://takeout.google.com"
              target="_blank"
              rel="noopener noreferrer"
              className="underline font-medium"
            >
              Google Takeout
            </a>{" "}
            and export your Google Photos data
          </li>
          <li>Unzip the downloaded archive on your computer</li>
          <li>
            Select <strong>both</strong> the photo/video files <strong>and</strong> their
            matching <code className="bg-blue-100 dark:bg-blue-900/40 px-1 rounded">.json</code> metadata files
          </li>
          <li>
            The metadata (date taken, GPS location, original filename) will be
            automatically applied to each photo
          </li>
        </ol>
        <p className="text-xs text-blue-600 mt-3">
          💡 Tip: You can select entire folders. JSON files without matching
          media will be ignored, and media files without metadata will use
          today's date.
        </p>
      </div>

      {/* ── Drop zone / file picker ────────────────────────────────────────── */}
      <div
        className={`border-2 border-dashed rounded-lg p-8 text-center transition-colors mb-6 ${
          dragOver
            ? "border-blue-500 dark:border-blue-400 bg-blue-50 dark:bg-blue-900/30"
            : "border-gray-300 dark:border-gray-600 hover:border-gray-400 dark:hover:border-gray-500"
        }`}
        onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragOver(false);
          if (e.dataTransfer.files.length > 0) processFiles(e.dataTransfer.files);
        }}
      >
        <div className="text-4xl mb-3">📂</div>
        <p className="text-gray-700 dark:text-gray-300 font-medium mb-1">
          Drag & drop photos, videos, and JSON metadata files here
        </p>
        <p className="text-gray-500 dark:text-gray-400 text-sm mb-4">
          or click to browse
        </p>
        <label className="inline-block bg-blue-600 text-white px-6 py-2.5 rounded-lg hover:bg-blue-700 cursor-pointer text-sm font-medium transition-colors">
          Select Files
          <input
            ref={inputRef}
            type="file"
            multiple
            accept="image/*,video/*,.json,.heic,.heif,.avif,.dng,.cr2,.nef,.arw"
            className="hidden"
            onChange={(e) => {
              if (e.target.files && e.target.files.length > 0) {
                processFiles(e.target.files);
              }
              if (inputRef.current) inputRef.current.value = "";
            }}
          />
        </label>
      </div>

      {error && (
        <div className="bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-400 rounded-lg p-3 mb-4 text-sm">
          {error}
        </div>
      )}

      {/* ── Stats bar ──────────────────────────────────────────────────────── */}
      {items.length > 0 && (
        <div className="flex items-center justify-between bg-gray-50 dark:bg-gray-900 rounded-lg p-4 mb-4">
          <div className="flex gap-6 text-sm">
            <span className="text-gray-700 dark:text-gray-300">
              <strong>{items.length}</strong> files
            </span>
            <span className="text-green-700 dark:text-green-400">
              <strong>{withMetadata}</strong> with metadata
            </span>
            <span className="text-gray-500 dark:text-gray-400">
              <strong>{items.length - withMetadata}</strong> without
            </span>
            {completedCount > 0 && (
              <span className="text-blue-700 dark:text-blue-300">
                <strong>{completedCount}</strong> imported
              </span>
            )}
            {errorCount > 0 && (
              <span className="text-red-700 dark:text-red-400">
                <strong>{errorCount}</strong> failed
              </span>
            )}
          </div>
          <div className="flex gap-2">
            {!importing && items.some((i) => i.status === "pending") && (
              <button
                onClick={handleImport}
                className="bg-green-600 text-white px-5 py-2 rounded-md hover:bg-green-700 text-sm font-medium"
              >
                Import {items.filter((i) => i.status === "pending").length} Files
              </button>
            )}
            {!importing && (
              <button
                onClick={clearAll}
                className="bg-gray-200 dark:bg-gray-600 text-gray-700 dark:text-gray-300 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Clear
              </button>
            )}
          </div>
        </div>
      )}

      {/* ── Progress bar ───────────────────────────────────────────────────── */}
      {importing && (
        <div className="mb-4">
          <div className="flex items-center justify-between text-sm text-gray-600 dark:text-gray-400 mb-1">
            <span>Importing… {progress.done}/{progress.total}</span>
            <span>{Math.round((progress.done / progress.total) * 100)}%</span>
          </div>
          <div className="w-full h-2 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
            <div
              className="h-full bg-blue-600 rounded-full transition-all duration-300"
              style={{ width: `${(progress.done / progress.total) * 100}%` }}
            />
          </div>
        </div>
      )}

      {/* ── File list ──────────────────────────────────────────────────────── */}
      {items.length > 0 && (
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-gray-50 dark:bg-gray-900 border-b">
              <tr>
                <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">File</th>
                <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">Date</th>
                <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">Location</th>
                <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">Metadata</th>
                <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">Status</th>
                <th className="px-4 py-2"></th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {items.map((item, i) => (
                <tr key={i} className="hover:bg-gray-50 dark:hover:bg-gray-700 dark:bg-gray-900">
                  <td className="px-4 py-2.5">
                    <div className="flex items-center gap-2">
                      <span className="text-base">
                        {item.file.type?.startsWith("video/") ? "🎬" : "🖼️"}
                      </span>
                      <span className="truncate max-w-[200px]" title={item.file.name}>
                        {item.metadata?.title || item.file.name}
                      </span>
                    </div>
                  </td>
                  <td className="px-4 py-2.5 text-gray-600 dark:text-gray-400">
                    {item.metadata?.photoTakenTime?.formatted
                      ? formatGoogleDate(item.metadata.photoTakenTime.formatted)
                      : item.metadata?.creationTime?.formatted
                        ? formatGoogleDate(item.metadata.creationTime.formatted)
                        : "—"}
                  </td>
                  <td className="px-4 py-2.5 text-gray-600 dark:text-gray-400">
                    {item.metadata?.geoData &&
                    (item.metadata.geoData.latitude !== 0 || item.metadata.geoData.longitude !== 0)
                      ? `${item.metadata.geoData.latitude.toFixed(4)}, ${item.metadata.geoData.longitude.toFixed(4)}`
                      : "—"}
                  </td>
                  <td className="px-4 py-2.5">
                    {item.metadata ? (
                      <span className="inline-flex items-center gap-1 text-green-700 dark:text-green-400 bg-green-50 dark:bg-green-900/30 px-2 py-0.5 rounded-full text-xs font-medium">
                        ✓ Google
                      </span>
                    ) : (
                      <span className="text-gray-400 text-xs">None</span>
                    )}
                  </td>
                  <td className="px-4 py-2.5">
                    {item.status === "pending" && (
                      <span className="text-gray-500 dark:text-gray-400 text-xs">Pending</span>
                    )}
                    {item.status === "uploading" && (
                      <span className="text-blue-600 text-xs flex items-center gap-1">
                        <div className="w-3 h-3 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
                        Uploading
                      </span>
                    )}
                    {item.status === "done" && (
                      <span className="text-green-600 dark:text-green-400 text-xs">✓ Done</span>
                    )}
                    {item.status === "error" && (
                      <span className="text-red-600 dark:text-red-400 text-xs" title={item.error}>
                        ✗ Error
                      </span>
                    )}
                  </td>
                  <td className="px-4 py-2.5">
                    {item.status !== "uploading" && item.status !== "done" && (
                      <button
                        onClick={() => removeItem(i)}
                        className="text-gray-400 hover:text-red-500 dark:hover:text-red-400 dark:text-red-400 text-xs"
                      >
                        ✕
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* ── Success message ────────────────────────────────────────────────── */}
      {!importing && completedCount > 0 && completedCount === items.length && (
        <div className="bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-800 rounded-lg p-4 mt-4 text-center">
          <div className="text-2xl mb-2">🎉</div>
          <p className="text-green-800 dark:text-green-300 font-medium">
            All {completedCount} files imported successfully!
          </p>
          <button
            onClick={() => navigate("/gallery")}
            className="mt-3 bg-green-600 text-white px-6 py-2 rounded-md hover:bg-green-700 text-sm font-medium"
          >
            View Gallery →
          </button>
        </div>
      )}
      </main>
    </div>
  );
}

function formatGoogleDate(formatted: string): string {
  // Google format: "May 16, 2017, 7:37:54 PM UTC"
  // Simplify to: "May 16, 2017"
  const parts = formatted.split(",");
  if (parts.length >= 2) {
    return `${parts[0].trim()}, ${parts[1].trim()}`;
  }
  return formatted;
}
