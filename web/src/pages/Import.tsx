import { useState, useCallback, useRef, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { api } from "../api/client";
import { db, blobTypeFromMime, mediaTypeFromMime } from "../db";
import AppHeader from "../components/AppHeader";

type EncryptionMode = "plain" | "encrypted";

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

// ── Import item types ─────────────────────────────────────────────────────────

interface ImportItem {
  /** For local files */
  file?: File;
  /** For server files — the absolute path on server */
  serverPath?: string;
  /** File name */
  name: string;
  /** File size in bytes */
  size: number;
  /** MIME type */
  mimeType: string;
  /** Google Photos metadata (optional) */
  metadata?: GooglePhotosMetadata;
  metadataFile?: string;
  status: "pending" | "uploading" | "done" | "error";
  error?: string;
}

// ── Server file listing ───────────────────────────────────────────────────────

interface ServerFile {
  name: string;
  path: string;
  size: number;
  mime_type: string;
  modified: string | null;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Efficient base64 encoding using chunked approach to prevent O(n²) string concat */
function arrayBufferToBase64(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  // Process in 32KB chunks to avoid call stack limit on String.fromCharCode
  const CHUNK = 32768;
  const parts: string[] = [];
  for (let i = 0; i < bytes.byteLength; i += CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.byteLength));
    parts.push(String.fromCharCode(...slice));
  }
  return btoa(parts.join(""));
}

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

function generateThumbnailFromBuffer(
  data: ArrayBuffer,
  mimeType: string,
  size: number
): Promise<ArrayBuffer> {
  if (mimeType.startsWith("video/"))
    return generateVideoThumbnailFromBuffer(data, mimeType, size);
  return generateImageThumbnailFromBuffer(data, mimeType, size);
}

/** Get image/video dimensions from raw data */
function getDimensionsFromBuffer(
  data: ArrayBuffer,
  mimeType: string
): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    const blob = new Blob([data], { type: mimeType });
    const url = URL.createObjectURL(blob);

    if (mimeType.startsWith("video/")) {
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
function getVideoDurationFromBuffer(
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

/**
 * Match Google Photos JSON metadata files to their media files.
 */
function matchMetadataToFiles(
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

/** Format bytes to human-readable string */
function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

/** Create a gray placeholder thumbnail when generation fails */
async function createFallbackThumbnail(): Promise<ArrayBuffer> {
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 256;
  const ctx = canvas.getContext("2d")!;
  // Gray gradient background
  const grad = ctx.createLinearGradient(0, 0, 256, 256);
  grad.addColorStop(0, "#555");
  grad.addColorStop(1, "#333");
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, 256, 256);
  // Camera icon
  ctx.fillStyle = "#777";
  ctx.font = "60px sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("📷", 128, 128);
  return new Promise((resolve) => {
    canvas.toBlob(
      (blob) => blob!.arrayBuffer().then(resolve),
      "image/jpeg",
      0.5
    );
  });
}

// ── Component ─────────────────────────────────────────────────────────────────

type ImportMode = "server" | "local";

export default function Import() {
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);
  const abortRef = useRef(false);
  const [mode, setMode] = useState<ImportMode>("server");
  const [items, setItems] = useState<ImportItem[]>([]);
  const [importing, setImporting] = useState(false);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [error, setError] = useState("");
  const [dragOver, setDragOver] = useState(false);

  // Server scan state
  const [scanning, setScanning] = useState(false);
  const [scanPath, setScanPath] = useState("");
  const [autoScanned, setAutoScanned] = useState(false);
  const autoImportPending = useRef(false);

  // Encryption mode detection
  const [encryptionMode, setEncryptionMode] = useState<EncryptionMode | null>(null);
  const [plainScanResult, setPlainScanResult] = useState<{ registered: number } | null>(null);

  // Detect encryption mode on mount
  // eslint-disable-next-line react-hooks/rules-of-hooks
  useEffect(() => {
    api.encryption.getSettings()
      .then((res) => setEncryptionMode(res.encryption_mode as EncryptionMode))
      .catch(() => setEncryptionMode("encrypted")); // fallback for pre-migration
  }, []);

  // Redirect if encrypted mode and no crypto key
  if (encryptionMode === "encrypted" && !hasCryptoKey()) {
    navigate("/setup");
    return null;
  }

  // Plain mode: just scan and register files on disk
  if (encryptionMode === "plain") {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
        <AppHeader />
        <main className="max-w-4xl mx-auto p-4">
          <div className="mb-6">
            <h2 className="text-xl font-semibold dark:text-white">Import Photos</h2>
            <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
              Scan the storage directory to register new photos and videos.
            </p>
          </div>

          {error && (
            <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">
              {error}
            </p>
          )}

          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
            <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
              In standard storage mode, photos stay as regular files in your storage directory. 
              Click the button below to scan for any new files and add them to your library.
            </p>
            <div className="flex items-center gap-4">
              <button
                onClick={async () => {
                  setError("");
                  setScanning(true);
                  setPlainScanResult(null);
                  try {
                    const res = await api.admin.scanAndRegister();
                    setPlainScanResult(res);
                  } catch (err: any) {
                    setError(err.message);
                  } finally {
                    setScanning(false);
                  }
                }}
                disabled={scanning}
                className="bg-blue-600 text-white px-5 py-2.5 rounded-md hover:bg-blue-700 text-sm font-medium disabled:opacity-50 inline-flex items-center gap-2"
              >
                {scanning && (
                  <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                )}
                {scanning ? "Scanning…" : "Scan Storage Directory"}
              </button>
              <button
                onClick={() => navigate("/gallery")}
                className="text-gray-500 hover:text-gray-700 dark:hover:text-gray-300 text-sm"
              >
                Back to Gallery
              </button>
            </div>

            {plainScanResult && (
              <div className="mt-4 p-3 bg-green-50 dark:bg-green-900/30 rounded-lg">
                <p className="text-sm text-green-700 dark:text-green-400">
                  {plainScanResult.registered > 0
                    ? `✓ ${plainScanResult.registered} new photo${plainScanResult.registered !== 1 ? "s" : ""} registered.`
                    : "No new files found. All photos in the storage directory are already registered."}
                </p>
              </div>
            )}
          </div>
        </main>
      </div>
    );
  }

  // If we haven't determined the mode yet, show a loading spinner
  if (encryptionMode === null) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  // Auto-scan the default storage directory on mount, then auto-import
  // eslint-disable-next-line react-hooks/rules-of-hooks
  useEffect(() => {
    if (!autoScanned) {
      setAutoScanned(true);
      autoImportPending.current = true;
      handleServerScan();
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // After auto-scan populates items, automatically start importing
  // eslint-disable-next-line react-hooks/rules-of-hooks
  useEffect(() => {
    if (
      autoImportPending.current &&
      !scanning &&
      !importing &&
      items.length > 0 &&
      items.some((i) => i.status === "pending")
    ) {
      autoImportPending.current = false;
      handleImport();
    }
  }, [items, scanning, importing]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Server scan ─────────────────────────────────────────────────────────

  async function handleServerScan(path?: string) {
    setScanning(true);
    setError("");
    try {
      const result = await api.admin.importScan(path || scanPath || undefined);
      setScanPath(result.directory);

      // Convert to import items
      const newItems: ImportItem[] = result.files.map((f: ServerFile) => ({
        serverPath: f.path,
        name: f.name,
        size: f.size,
        mimeType: f.mime_type,
        status: "pending" as const,
      }));

      setItems(newItems);

      if (result.files.length === 0) {
        setError("No media files found in this directory.");
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Scan failed";
      setError(msg);
    } finally {
      setScanning(false);
    }
  }

  // ── Process local files (drag & drop / file picker) ─────────────────────

  const processFiles = useCallback((fileList: FileList) => {
    const allFiles = Array.from(fileList);
    const mediaFiles: File[] = [];
    const jsonFiles = new Map<string, GooglePhotosMetadata>();
    const jsonReadPromises: Promise<void>[] = [];

    for (const file of allFiles) {
      if (file.name.endsWith(".json")) {
        const promise = file.text().then((text) => {
          try {
            const data = JSON.parse(text) as GooglePhotosMetadata;
            if (data.title || data.photoTakenTime || data.creationTime) {
              const mediaName = file.name.replace(/\.json$/, "");
              jsonFiles.set(mediaName, data);
              if (data.title) jsonFiles.set(data.title, data);
            }
          } catch {
            /* not valid metadata */
          }
        });
        jsonReadPromises.push(promise);
      } else if (
        file.type.startsWith("image/") ||
        file.type.startsWith("video/") ||
        /\.(heic|heif|avif|webp|dng|cr2|nef|arw|raw|jpg|jpeg|png|gif|mp4|mov|mkv|webm)$/i.test(
          file.name
        )
      ) {
        mediaFiles.push(file);
      }
    }

    Promise.all(jsonReadPromises).then(() => {
      const matched = matchMetadataToFiles(mediaFiles, jsonFiles);
      if (matched.length === 0 && jsonFiles.size > 0) {
        setError(
          "Only metadata JSON files found. Please also select the photo/video files."
        );
        return;
      }
      if (matched.length === 0) {
        setError("No supported media files found.");
        return;
      }
      setItems((prev) => [...prev, ...matched]);
      setError("");
    });
  }, []);

  // ── Core import logic (handles both local and server files) ─────────────

  async function handleImport() {
    const pending = items.filter(
      (i) => i.status === "pending" || i.status === "error"
    );
    if (pending.length === 0) return;

    setImporting(true);
    setError("");
    abortRef.current = false;
    setProgress({ done: 0, total: pending.length });

    let doneCount = 0;

    for (let i = 0; i < items.length; i++) {
      if (abortRef.current) break;

      const item = items[i];
      if (item.status === "done") continue;
      if (item.status !== "pending" && item.status !== "error") continue;

      setItems((prev) =>
        prev.map((it, idx) =>
          idx === i ? { ...it, status: "uploading", error: undefined } : it
        )
      );

      try {
        await importSingleItem(item);
        setItems((prev) =>
          prev.map((it, idx) => (idx === i ? { ...it, status: "done" } : it))
        );
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : "Import failed";
        console.error(`Import failed for ${item.name}:`, err);
        setItems((prev) =>
          prev.map((it, idx) =>
            idx === i ? { ...it, status: "error", error: msg } : it
          )
        );
      }

      doneCount++;
      setProgress({ done: doneCount, total: pending.length });

      // Small delay between uploads to prevent overwhelming the server/browser
      if (!abortRef.current) {
        await new Promise((r) => setTimeout(r, 100));
      }
    }

    setImporting(false);
  }

  async function importSingleItem(item: ImportItem) {
    // Step 1: Get the raw file data
    let rawData: ArrayBuffer;

    if (item.serverPath) {
      // Server-side file: download via API
      rawData = await api.admin.importFile(item.serverPath);
    } else if (item.file) {
      // Local file: read from browser File API
      rawData = await item.file.arrayBuffer();
    } else {
      throw new Error("No file data source");
    }

    const data = new Uint8Array(rawData);
    const mimeType = item.mimeType || guessMimeFromName(item.name);
    const mediaType = mediaTypeFromMime(mimeType);
    const serverBlobType = blobTypeFromMime(mimeType);
    const filename = item.metadata?.title || item.name;

    // Step 2: Extract metadata from Google Photos JSON (if available)
    let takenAt = new Date().toISOString();
    let latitude: number | undefined;
    let longitude: number | undefined;

    if (item.metadata) {
      if (item.metadata.photoTakenTime?.timestamp) {
        takenAt = new Date(
          parseInt(item.metadata.photoTakenTime.timestamp) * 1000
        ).toISOString();
      } else if (item.metadata.creationTime?.timestamp) {
        takenAt = new Date(
          parseInt(item.metadata.creationTime.timestamp) * 1000
        ).toISOString();
      }

      if (
        item.metadata.geoData &&
        (item.metadata.geoData.latitude !== 0 ||
          item.metadata.geoData.longitude !== 0)
      ) {
        latitude = item.metadata.geoData.latitude;
        longitude = item.metadata.geoData.longitude;
      } else if (
        item.metadata.geoDataExif &&
        (item.metadata.geoDataExif.latitude !== 0 ||
          item.metadata.geoDataExif.longitude !== 0)
      ) {
        latitude = item.metadata.geoDataExif.latitude;
        longitude = item.metadata.geoDataExif.longitude;
      }
    }

    // Step 3: Get dimensions
    const dims = await getDimensionsFromBuffer(rawData, mimeType);

    // Step 4: Get video duration
    let duration: number | undefined;
    if (mediaType === "video") {
      duration = await getVideoDurationFromBuffer(rawData, mimeType);
    }

    // Step 5: Generate thumbnail (with fallback on failure)
    let thumbnailData: ArrayBuffer;
    try {
      thumbnailData = await generateThumbnailFromBuffer(rawData, mimeType, 256);
    } catch {
      console.warn(
        `Thumbnail generation failed for ${item.name}, using fallback`
      );
      thumbnailData = await createFallbackThumbnail();
    }

    // Step 6: Upload thumbnail blob
    const thumbPayload = JSON.stringify({
      v: 1,
      photo_blob_id: "",
      width: 256,
      height: 256,
      data: arrayBufferToBase64(thumbnailData),
    });
    const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
    const thumbHash = await sha256Hex(new Uint8Array(encThumb));
    const thumbBlobType =
      mediaType === "video" ? "video_thumbnail" : "thumbnail";
    const thumbRes = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);

    // Step 7: Upload media blob
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

    // Step 8: Cache in IndexedDB
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

  // ── UI Actions ──────────────────────────────────────────────────────────

  function removeItem(index: number) {
    setItems((prev) => prev.filter((_, i) => i !== index));
  }

  function clearAll() {
    setItems([]);
    setProgress({ done: 0, total: 0 });
  }

  function retryFailed() {
    setItems((prev) =>
      prev.map((it) =>
        it.status === "error"
          ? { ...it, status: "pending" as const, error: undefined }
          : it
      )
    );
  }

  function stopImport() {
    abortRef.current = true;
  }

  const pendingCount = items.filter(
    (i) => i.status === "pending" || i.status === "error"
  ).length;
  const completedCount = items.filter((i) => i.status === "done").length;
  const errorCount = items.filter((i) => i.status === "error").length;
  const withMetadata = items.filter((i) => i.metadata).length;

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="max-w-4xl mx-auto p-4">
        <div className="mb-6">
          <h2 className="text-xl font-semibold dark:text-white">
            Import Photos
          </h2>
          <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
            Import from server directory or local files
          </p>
        </div>

        {/* ── Mode tabs ────────────────────────────────────────────────────── */}
        <div className="flex gap-1 bg-gray-200 dark:bg-gray-700 rounded-lg p-1 mb-6 w-fit">
          <button
            onClick={() => setMode("server")}
            className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
              mode === "server"
                ? "bg-white dark:bg-gray-800 text-gray-900 dark:text-white shadow"
                : "text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white"
            }`}
          >
            📁 Server Directory
          </button>
          <button
            onClick={() => setMode("local")}
            className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
              mode === "local"
                ? "bg-white dark:bg-gray-800 text-gray-900 dark:text-white shadow"
                : "text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white"
            }`}
          >
            💻 Local Upload
          </button>
        </div>

        {/* ── Server Directory Mode ────────────────────────────────────────── */}
        {mode === "server" && (
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-6">
            <h3 className="font-semibold text-gray-900 dark:text-gray-100 mb-3">
              Scan Server Directory
            </h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
              Scan a directory on the server for photos and videos to import.
              Files are encrypted locally before being stored.
            </p>
            <div className="flex gap-2">
              <input
                type="text"
                value={scanPath}
                onChange={(e) => setScanPath(e.target.value)}
                placeholder="Server directory path (defaults to storage root)"
                className="flex-1 border dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 text-sm"
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleServerScan();
                }}
              />
              <button
                onClick={() => handleServerScan()}
                disabled={scanning}
                className="bg-blue-600 text-white px-5 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm font-medium whitespace-nowrap"
              >
                {scanning ? (
                  <span className="flex items-center gap-2">
                    <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                    Scanning…
                  </span>
                ) : (
                  "Scan Directory"
                )}
              </button>
            </div>
          </div>
        )}

        {/* ── Local Upload Mode ────────────────────────────────────────────── */}
        {mode === "local" && (
          <>
            <div className="bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg p-4 mb-6">
              <h3 className="font-semibold text-blue-900 dark:text-blue-300 mb-2">
                📥 How to Import
              </h3>
              <ol className="text-sm text-blue-800 dark:text-blue-300 space-y-1.5 list-decimal list-inside">
                <li>
                  Select photos or videos from your computer, or drag & drop
                  below
                </li>
                <li>
                  Optionally include{" "}
                  <code className="bg-blue-100 dark:bg-blue-900/40 px-1 rounded">
                    .json
                  </code>{" "}
                  metadata files from Google Takeout
                </li>
                <li>
                  Click <strong>Import</strong> to encrypt and upload
                </li>
              </ol>
            </div>

            <div
              className={`border-2 border-dashed rounded-lg p-8 text-center transition-colors mb-6 ${
                dragOver
                  ? "border-blue-500 dark:border-blue-400 bg-blue-50 dark:bg-blue-900/30"
                  : "border-gray-300 dark:border-gray-600 hover:border-gray-400 dark:hover:border-gray-500"
              }`}
              onDragOver={(e) => {
                e.preventDefault();
                setDragOver(true);
              }}
              onDragLeave={() => setDragOver(false)}
              onDrop={(e) => {
                e.preventDefault();
                setDragOver(false);
                if (e.dataTransfer.files.length > 0)
                  processFiles(e.dataTransfer.files);
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
          </>
        )}

        {error && (
          <div className="bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-400 rounded-lg p-3 mb-4 text-sm">
            {error}
          </div>
        )}

        {/* ── Stats bar ──────────────────────────────────────────────────── */}
        {items.length > 0 && (
          <div className="flex flex-wrap items-center justify-between bg-white dark:bg-gray-800 rounded-lg shadow p-4 mb-4 gap-3">
            <div className="flex flex-wrap gap-4 text-sm">
              <span className="text-gray-700 dark:text-gray-300">
                <strong>{items.length}</strong> files
              </span>
              <span className="text-gray-500 dark:text-gray-400">
                {formatBytes(items.reduce((sum, i) => sum + i.size, 0))}
              </span>
              {withMetadata > 0 && (
                <span className="text-green-700 dark:text-green-400">
                  <strong>{withMetadata}</strong> with metadata
                </span>
              )}
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
              {importing && (
                <button
                  onClick={stopImport}
                  className="bg-red-600 text-white px-4 py-2 rounded-md hover:bg-red-700 text-sm font-medium"
                >
                  Stop
                </button>
              )}
              {!importing && pendingCount > 0 && (
                <button
                  onClick={handleImport}
                  className="bg-green-600 text-white px-5 py-2 rounded-md hover:bg-green-700 text-sm font-medium"
                >
                  Import {pendingCount} Files
                </button>
              )}
              {!importing && errorCount > 0 && (
                <button
                  onClick={retryFailed}
                  className="bg-yellow-600 text-white px-4 py-2 rounded-md hover:bg-yellow-700 text-sm font-medium"
                >
                  Retry {errorCount} Failed
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

        {/* ── Progress bar ─────────────────────────────────────────────────── */}
        {importing && (
          <div className="mb-4">
            <div className="flex items-center justify-between text-sm text-gray-600 dark:text-gray-400 mb-1">
              <span>
                Importing… {progress.done}/{progress.total}
              </span>
              <span>
                {progress.total > 0
                  ? Math.round((progress.done / progress.total) * 100)
                  : 0}
                %
              </span>
            </div>
            <div className="w-full h-2 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
              <div
                className="h-full bg-blue-600 rounded-full transition-all duration-300"
                style={{
                  width: `${
                    progress.total > 0
                      ? (progress.done / progress.total) * 100
                      : 0
                  }%`,
                }}
              />
            </div>
          </div>
        )}

        {/* ── File list ──────────────────────────────────────────────────── */}
        {items.length > 0 && (
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow overflow-hidden">
            <table className="w-full text-sm">
              <thead className="bg-gray-50 dark:bg-gray-900 border-b dark:border-gray-700">
                <tr>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
                    File
                  </th>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
                    Size
                  </th>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
                    Type
                  </th>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 dark:text-gray-400">
                    Status
                  </th>
                  <th className="px-4 py-2"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
                {items.map((item, i) => (
                  <tr
                    key={`${item.name}-${i}`}
                    className="hover:bg-gray-50 dark:hover:bg-gray-700/50"
                  >
                    <td className="px-4 py-2.5">
                      <div className="flex items-center gap-2">
                        <span className="text-base">
                          {item.mimeType?.startsWith("video/") ? "🎬" : "🖼️"}
                        </span>
                        <span
                          className="truncate max-w-[250px] dark:text-gray-200"
                          title={item.name}
                        >
                          {item.name}
                        </span>
                      </div>
                    </td>
                    <td className="px-4 py-2.5 text-gray-500 dark:text-gray-400">
                      {formatBytes(item.size)}
                    </td>
                    <td className="px-4 py-2.5 text-gray-500 dark:text-gray-400 text-xs">
                      {item.mimeType?.split("/")[1]?.toUpperCase() || "—"}
                    </td>
                    <td className="px-4 py-2.5">
                      {item.status === "pending" && (
                        <span className="text-gray-500 dark:text-gray-400 text-xs">
                          Pending
                        </span>
                      )}
                      {item.status === "uploading" && (
                        <span className="text-blue-600 text-xs flex items-center gap-1">
                          <div className="w-3 h-3 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
                          Importing
                        </span>
                      )}
                      {item.status === "done" && (
                        <span className="text-green-600 dark:text-green-400 text-xs">
                          ✓ Done
                        </span>
                      )}
                      {item.status === "error" && (
                        <span
                          className="text-red-600 dark:text-red-400 text-xs cursor-help"
                          title={item.error}
                        >
                          ✗ {item.error || "Error"}
                        </span>
                      )}
                    </td>
                    <td className="px-4 py-2.5">
                      {item.status !== "uploading" &&
                        item.status !== "done" && (
                          <button
                            onClick={() => removeItem(i)}
                            className="text-gray-400 hover:text-red-500 dark:hover:text-red-400 text-xs"
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

        {/* ── Success message ──────────────────────────────────────────────── */}
        {!importing &&
          completedCount > 0 &&
          completedCount === items.length && (
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
