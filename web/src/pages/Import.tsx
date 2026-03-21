/**
 * Import page — bulk photo/video import from server paths or local files.
 *
 * Reads files client-side, encrypts with AES-256-GCM, and uploads blobs.
 * Supports Google Photos Takeout metadata matching and deduplication via
 * content hashes.
 */
import { useState, useCallback, useRef, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { api } from "../api/client";
import { db, blobTypeFromMime, mediaTypeFromMime } from "../db";
import AppHeader from "../components/AppHeader";
import { useProcessingStore } from "../store/processing";

import type { ImportItem, ServerFile, GooglePhotosMetadata } from "../utils/importTypes";
import {
  arrayBufferToBase64,
  generateThumbnailFromBuffer,
  getDimensionsFromBuffer,
  getVideoDurationFromBuffer,
  guessMimeFromName,
  matchMetadataToFiles,
  createFallbackThumbnail,
  createAudioFallbackThumbnail,
} from "../utils/media";
import { formatBytes } from "../utils/formatters";
import ImportFileList from "./import/ImportFileList";

type ImportMode = "server" | "local";

export default function Import() {
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);
  const abortRef = useRef(false);
  const { startTask, endTask } = useProcessingStore();
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

  // ── Hooks (unconditional — must run in the same order every render) ────

  // Auto-scan default storage on first mount
  useEffect(() => {
    if (!hasCryptoKey() || autoScanned) return;
    setAutoScanned(true);
    autoImportPending.current = true;
    handleServerScan();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-import once scan populates items
  useEffect(() => {
    if (!hasCryptoKey()) return;
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

  // ── Conditional early returns (AFTER all hooks) ─────────────────────────

  // Redirect if no crypto key
  if (!hasCryptoKey()) {
    navigate("/setup");
    return null;
  }

  // ── Server scan ─────────────────────────────────────────────────────────

  async function handleServerScan(path?: string) {
    setScanning(true);
    setError("");
    try {
      const result = await api.admin.importScan(path || scanPath || undefined);
      setScanPath(result.directory);

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
        file.type.startsWith("audio/") ||
        /\.(jpe?g|png|gif|webp|avif|bmp|ico|svg|mp4|webm|mp3|flac|ogg|wav)$/i.test(
          file.name
        )
      ) {
        mediaFiles.push(file);
      }
    }

    Promise.all(jsonReadPromises).then(() => {
      const matched = matchMetadataToFiles(mediaFiles, jsonFiles);
      if (matched.length === 0 && jsonFiles.size > 0) {
        setError("Only metadata JSON files found. Please also select the photo/video files.");
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

  // ── Core import logic ───────────────────────────────────────────────────

  async function handleImport() {
    const pending = items.filter(
      (i) => i.status === "pending" || i.status === "error"
    );
    if (pending.length === 0) return;

    setImporting(true);
    startTask("import");
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

      if (!abortRef.current) {
        await new Promise((r) => setTimeout(r, 100));
      }
    }

    setImporting(false);
    endTask("import");
  }

  async function importSingleItem(item: ImportItem) {
    let rawData: ArrayBuffer;

    if (item.serverPath) {
      rawData = await api.admin.importFile(item.serverPath);
    } else if (item.file) {
      rawData = await item.file.arrayBuffer();
    } else {
      throw new Error("No file data source");
    }

    const data = new Uint8Array(rawData);
    const mimeType = item.mimeType || guessMimeFromName(item.name);
    const mediaType = mediaTypeFromMime(mimeType);
    const serverBlobType = blobTypeFromMime(mimeType);
    const filename = item.metadata?.title || item.name;

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

    const dims = await getDimensionsFromBuffer(rawData, mimeType);

    let duration: number | undefined;
    if (mediaType === "video") {
      duration = await getVideoDurationFromBuffer(rawData, mimeType);
    }

    let thumbnailData: ArrayBuffer;
    if (mediaType === "audio") {
      thumbnailData = await createAudioFallbackThumbnail();
    } else {
      try {
        thumbnailData = await generateThumbnailFromBuffer(rawData, mimeType, 256);
      } catch {
        console.warn(`Thumbnail generation failed for ${item.name}, using fallback`);
        thumbnailData = await createFallbackThumbnail();
      }
    }

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
    // Content hash: short hash of original raw bytes for cross-platform alignment
    const contentHash = (await sha256Hex(new Uint8Array(data))).substring(0, 12);
    const res = await api.blobs.upload(encPhoto, serverBlobType, photoHash, contentHash);

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
      contentHash,
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
          <h2 className="text-xl font-semibold dark:text-white">Import Photos</h2>
          <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
            Import from server directory or local files
          </p>
        </div>

        {/* Mode tabs */}
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

        {/* Server Directory Mode */}
        {mode === "server" && (
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-6">
            <h3 className="font-semibold text-gray-900 dark:text-gray-100 mb-3">Scan Server Directory</h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
              Scan a directory on the server for photos and videos to import. Files are encrypted locally before being stored.
            </p>
            <div className="flex gap-2">
              <input
                type="text"
                value={scanPath}
                onChange={(e) => setScanPath(e.target.value)}
                placeholder="Server directory path (defaults to storage root)"
                className="flex-1 border dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-white rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 text-sm"
                onKeyDown={(e) => { if (e.key === "Enter") handleServerScan(); }}
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

        {/* Local Upload Mode */}
        {mode === "local" && (
          <>
            <div className="bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg p-4 mb-6">
              <h3 className="font-semibold text-blue-900 dark:text-blue-300 mb-2">📥 How to Import</h3>
              <ol className="text-sm text-blue-800 dark:text-blue-300 space-y-1.5 list-decimal list-inside">
                <li>Select photos or videos from your computer, or drag & drop below</li>
                <li>
                  Optionally include{" "}
                  <code className="bg-blue-100 dark:bg-blue-900/40 px-1 rounded">.json</code>{" "}
                  metadata files from Google Takeout
                </li>
                <li>Click <strong>Import</strong> to encrypt and upload</li>
              </ol>
            </div>

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
              <p className="text-gray-500 dark:text-gray-400 text-sm mb-4">or click to browse</p>
              <label className="inline-block bg-blue-600 text-white px-6 py-2.5 rounded-lg hover:bg-blue-700 cursor-pointer text-sm font-medium transition-colors">
                Select Files
                <input
                  ref={inputRef}
                  type="file"
                  multiple
                  accept="image/jpeg,image/png,image/gif,image/webp,image/avif,image/bmp,image/x-icon,video/mp4,video/webm,video/quicktime,audio/mpeg,audio/flac,audio/ogg,audio/wav,.json"
                  className="hidden"
                  onChange={(e) => {
                    if (e.target.files && e.target.files.length > 0) processFiles(e.target.files);
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

        {/* Stats bar */}
        {items.length > 0 && (
          <div className="flex flex-wrap items-center justify-between bg-white dark:bg-gray-800 rounded-lg shadow p-4 mb-4 gap-3">
            <div className="flex flex-wrap gap-4 text-sm">
              <span className="text-gray-700 dark:text-gray-300"><strong>{items.length}</strong> files</span>
              <span className="text-gray-500 dark:text-gray-400">{formatBytes(items.reduce((sum, i) => sum + i.size, 0))}</span>
              {withMetadata > 0 && (
                <span className="text-green-700 dark:text-green-400"><strong>{withMetadata}</strong> with metadata</span>
              )}
              {completedCount > 0 && (
                <span className="text-blue-700 dark:text-blue-300"><strong>{completedCount}</strong> imported</span>
              )}
              {errorCount > 0 && (
                <span className="text-red-700 dark:text-red-400"><strong>{errorCount}</strong> failed</span>
              )}
            </div>
            <div className="flex gap-2">
              {importing && (
                <button onClick={stopImport} className="bg-red-600 text-white px-4 py-2 rounded-md hover:bg-red-700 text-sm font-medium">
                  Stop
                </button>
              )}
              {!importing && pendingCount > 0 && (
                <button onClick={handleImport} className="bg-green-600 text-white px-5 py-2 rounded-md hover:bg-green-700 text-sm font-medium">
                  Import {pendingCount} Files
                </button>
              )}
              {!importing && errorCount > 0 && (
                <button onClick={retryFailed} className="bg-yellow-600 text-white px-4 py-2 rounded-md hover:bg-yellow-700 text-sm font-medium">
                  Retry {errorCount} Failed
                </button>
              )}
              {!importing && (
                <button onClick={clearAll} className="bg-gray-200 dark:bg-gray-600 text-gray-700 dark:text-gray-300 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm">
                  Clear
                </button>
              )}
            </div>
          </div>
        )}

        {/* Progress bar */}
        {importing && (
          <div className="mb-4">
            <div className="flex items-center justify-between text-sm text-gray-600 dark:text-gray-400 mb-1">
              <span>Importing… {progress.done}/{progress.total}</span>
              <span>{progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0}%</span>
            </div>
            <div className="w-full h-2 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
              <div
                className="h-full bg-blue-600 rounded-full transition-all duration-300"
                style={{ width: `${progress.total > 0 ? (progress.done / progress.total) * 100 : 0}%` }}
              />
            </div>
          </div>
        )}

        {/* File list */}
        <ImportFileList items={items} removeItem={removeItem} />

        {/* Success message */}
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
