/**
 * Import page — bulk photo/video import from server paths or local files.
 *
 * Two modes:
 *
 * - **Server Directory**: hands a server-side path to `POST /admin/import/ingest`,
 *   which registers files in place (when under the storage root) or stream-copies
 *   them into the library server-side — NO bytes round-trip through the browser.
 *   This replaces the old download-then-reupload flow that pulled the whole
 *   library through the tab and failed / partially-imported on large folders.
 *
 * - **Local Upload**: genuine browser-picked files are uploaded to
 *   `/api/photos/upload` through a bounded-concurrency worker pool with retry +
 *   exponential backoff, so transient errors don't abandon files mid-import.
 *
 * Google Photos Takeout sidecars are still parsed locally (local mode) so their
 * `photoTakenTime` / `geoData` are forwarded as override headers when the file's
 * EXIF is missing those fields.
 */
import { useState, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { hasCryptoKey } from "../crypto/crypto";
import { api } from "../api/client";
import AppHeader from "../components/AppHeader";
import { useProcessingStore } from "../store/processing";

import type { ImportItem, GooglePhotosMetadata } from "../utils/importTypes";
import {
  guessMimeFromName,
  matchMetadataToFiles,
  dedupeGooglePhotosEdits,
} from "../utils/media";
import { formatBytes } from "../utils/formatters";
import ImportFileList from "./import/ImportFileList";

type ImportMode = "server" | "local";

/** How many local uploads run at once. Bounded so a huge folder can't open
 *  tens of thousands of simultaneous requests. */
const UPLOAD_CONCURRENCY = 3;
/** Per-file upload attempts before giving up (1 try + 2 retries). */
const MAX_UPLOAD_ATTEMPTS = 3;
/** Throttle interval (ms) for flushing per-file status into React state. Keeps
 *  the import O(n) overall instead of O(n²) re-renders on large lists. */
const STATUS_FLUSH_MS = 250;

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
  const [notice, setNotice] = useState("");
  const [dragOver, setDragOver] = useState(false);

  // Server-directory ingest state
  const [scanPath, setScanPath] = useState("");
  const [serverBusy, setServerBusy] = useState(false);
  const [serverResult, setServerResult] = useState<{
    queued: number;
    in_place: boolean;
    directory: string;
  } | null>(null);

  // Redirect if no crypto key (after hooks, none above are conditional).
  if (!hasCryptoKey()) {
    navigate("/setup");
    return null;
  }

  // ── Server directory ingest (no browser round-trip) ─────────────────────

  async function handleServerIngest() {
    setServerBusy(true);
    setError("");
    setNotice("");
    setServerResult(null);
    try {
      const res = await api.admin.importIngest(scanPath || undefined, "copy");
      setServerResult(res);
      if (res.queued === 0) {
        setNotice("No new media files found to import in that directory.");
      } else {
        setNotice(
          `Queued ${res.queued.toLocaleString()} file${res.queued === 1 ? "" : "s"} from ` +
            `${res.directory}. ${
              res.in_place
                ? "Registering them in place"
                : "Copying them into your library"
            } — conversion and encryption continue in the background. You can ` +
            "leave this page; progress shows in the banners.",
        );
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Import failed";
      setError(msg);
    } finally {
      setServerBusy(false);
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
      // Collapse Google Photos original/"-edited" pairs to the edited copy
      // BEFORE matching metadata, so the surviving file inherits the
      // original's sidecar (Google names the sidecar after the original).
      const dedupedMedia = dedupeGooglePhotosEdits(mediaFiles);
      const skippedDupes = mediaFiles.length - dedupedMedia.length;
      const matched = matchMetadataToFiles(dedupedMedia, jsonFiles);
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
      setNotice(
        skippedDupes > 0
          ? `Skipped ${skippedDupes} unedited Google Photos original${skippedDupes === 1 ? "" : "s"} — keeping the edited copy with its metadata.`
          : ""
      );
    });
  }, []);

  // ── Core local-upload logic (bounded concurrency + retry/backoff) ───────

  async function handleImport() {
    // Snapshot the indices that still need work. We read file data from this
    // snapshot during the run; per-file status lives in `statusRef` (not React
    // state) so completing a file is O(1), not an O(n) array map.
    const snapshot = items;
    const pendingIdx: number[] = [];
    snapshot.forEach((it, i) => {
      if (it.status === "pending" || it.status === "error") pendingIdx.push(i);
    });
    if (pendingIdx.length === 0) return;

    setImporting(true);
    startTask("import");
    setError("");
    abortRef.current = false;
    setProgress({ done: 0, total: pendingIdx.length });

    const statusRef = new Map<
      number,
      { status: ImportItem["status"]; error?: string }
    >();
    let done = 0;

    // Flush accumulated statuses into React state at most every STATUS_FLUSH_MS,
    // so a 50k-file import doesn't trigger 50k full-list re-renders.
    let flushTimer: ReturnType<typeof setTimeout> | null = null;
    const flush = () => {
      flushTimer = null;
      setItems((prev) =>
        prev.map((it, i) => {
          const s = statusRef.get(i);
          return s ? { ...it, status: s.status, error: s.error } : it;
        })
      );
    };
    const scheduleFlush = () => {
      if (flushTimer === null) flushTimer = setTimeout(flush, STATUS_FLUSH_MS);
    };
    const setStatus = (
      i: number,
      status: ImportItem["status"],
      errMsg?: string
    ) => {
      statusRef.set(i, { status, error: errMsg });
      scheduleFlush();
    };

    let cursor = 0;
    const worker = async () => {
      while (!abortRef.current) {
        const pos = cursor++;
        if (pos >= pendingIdx.length) break;
        const i = pendingIdx[pos];
        setStatus(i, "uploading");

        let lastErr = "";
        for (let attempt = 1; attempt <= MAX_UPLOAD_ATTEMPTS; attempt++) {
          if (abortRef.current) break;
          try {
            await importSingleItem(snapshot[i]);
            lastErr = "";
            break;
          } catch (err: unknown) {
            lastErr = err instanceof Error ? err.message : "Import failed";
            if (attempt < MAX_UPLOAD_ATTEMPTS) {
              // Exponential backoff with jitter: ~0.5s, ~1s.
              const delay = 500 * 2 ** (attempt - 1) + Math.random() * 250;
              await new Promise((r) => setTimeout(r, delay));
            }
          }
        }

        if (lastErr) {
          console.error(`Import failed for ${snapshot[i].name}: ${lastErr}`); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
          setStatus(i, "error", lastErr);
        } else {
          setStatus(i, "done");
        }
        done++;
        setProgress({ done, total: pendingIdx.length });
      }
    };

    const workerCount = Math.min(UPLOAD_CONCURRENCY, pendingIdx.length);
    await Promise.all(Array.from({ length: workerCount }, () => worker()));

    // Any file still marked "uploading" was interrupted by Stop — return it to
    // pending so a later Import/Retry picks it up cleanly.
    for (const i of pendingIdx) {
      if (statusRef.get(i)?.status === "uploading") {
        statusRef.set(i, { status: "pending" });
      }
    }
    if (flushTimer !== null) clearTimeout(flushTimer);
    flush();

    setImporting(false);
    endTask("import");
  }

  async function importSingleItem(item: ImportItem) {
    if (!item.file) {
      throw new Error("No local file data — use Server Directory import for server-side files");
    }
    const rawData = await item.file.arrayBuffer();

    const mimeType = item.mimeType || guessMimeFromName(item.name);
    const filename = item.metadata?.title || item.name;

    // Pull Google Photos Takeout sidecar values, when present, as fallbacks
    // for the server-side EXIF extractor. EXIF still wins on the server so
    // on-device camera metadata isn't overridden by stale sidecar data.
    let takenAt: string | undefined;
    let latitude: number | undefined;
    let longitude: number | undefined;

    if (item.metadata) {
      if (item.metadata.photoTakenTime?.timestamp) {
        takenAt = new Date(
          parseInt(item.metadata.photoTakenTime.timestamp) * 1000,
        ).toISOString();
      } else if (item.metadata.creationTime?.timestamp) {
        takenAt = new Date(
          parseInt(item.metadata.creationTime.timestamp) * 1000,
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

    // Defer conversion to the server's background pass so a slow per-file
    // FFmpeg run can't freeze the import. The server only acts on this for
    // convertible (non-native) files with no metadata overrides.
    await api.photos.upload(rawData, filename, mimeType, {
      takenAt,
      latitude,
      longitude,
      fileModifiedAt: item.file.lastModified,
      deferConversion: true,
    });
  }

  // ── UI Actions ──────────────────────────────────────────────────────────

  function removeItem(index: number) {
    setItems((prev) => prev.filter((_, i) => i !== index));
  }

  function clearAll() {
    setItems([]);
    setProgress({ done: 0, total: 0 });
    setNotice("");
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
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      <main className="max-w-4xl mx-auto p-4">
        <div className="mb-6">
          <h2 className="text-xl font-semibold dark:text-white">Import Photos</h2>
          <p className="text-fg-muted text-sm mt-1">
            Import from a server directory or upload local files
          </p>
        </div>

        {/* Mode tabs */}
        <div className="flex gap-1 bg-edge rounded-lg p-1 mb-6 w-fit">
          <button
            onClick={() => setMode("server")}
            className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
              mode === "server"
                ? "bg-surface text-fg shadow"
                : "text-fg-muted hover:text-fg"
            }`}
          >
            📁 Server Directory
          </button>
          <button
            onClick={() => setMode("local")}
            className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
              mode === "local"
                ? "bg-surface text-fg shadow"
                : "text-fg-muted hover:text-fg"
            }`}
          >
            💻 Local Upload
          </button>
        </div>

        {/* Server Directory Mode */}
        {mode === "server" && (
          <div className="card p-6 mb-6">
            <h3 className="font-semibold text-fg mb-3">Import a Server Directory</h3>
            <p className="text-sm text-fg-muted mb-4">
              Import every photo and video under a directory on the server. Files
              already inside your storage folder are registered in place; files
              elsewhere are copied into your library. Nothing is downloaded to
              this browser, so very large folders import reliably. Everything is
              encrypted on the server.
            </p>
            <div className="flex gap-2">
              <input
                type="text"
                value={scanPath}
                onChange={(e) => setScanPath(e.target.value)}
                placeholder="Server directory path (defaults to storage root)"
                className="input flex-1"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !serverBusy) handleServerIngest();
                }}
              />
              <button
                onClick={() => handleServerIngest()}
                disabled={serverBusy}
                className="btn btn-primary btn-md whitespace-nowrap"
              >
                {serverBusy ? (
                  <span className="flex items-center gap-2">
                    <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                    Starting…
                  </span>
                ) : (
                  "Import Directory"
                )}
              </button>
            </div>

            {serverResult && serverResult.queued > 0 && (
              <div className="mt-4 bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-800 rounded-lg p-4">
                <p className="text-green-800 dark:text-green-300 font-medium text-sm">
                  🚀 Importing {serverResult.queued.toLocaleString()} file
                  {serverResult.queued === 1 ? "" : "s"} in the background
                </p>
                <button
                  onClick={() => navigate("/gallery")}
                  className="mt-3 btn btn-success btn-md"
                >
                  View Gallery →
                </button>
              </div>
            )}
          </div>
        )}

        {/* Local Upload Mode */}
        {mode === "local" && (
          <>
            <div className="bg-accent-50 dark:bg-accent-900/30 border border-accent-200 dark:border-accent-800 rounded-lg p-4 mb-6">
              <h3 className="font-semibold text-accent-900 dark:text-accent-300 mb-2">📥 How to Import</h3>
              <ol className="text-sm text-accent-800 dark:text-accent-300 space-y-1.5 list-decimal list-inside">
                <li>Select photos or videos from your computer, or drag & drop below</li>
                <li>
                  Optionally include{" "}
                  <code className="bg-accent-100 dark:bg-accent-900/40 px-1 rounded">.json</code>{" "}
                  metadata files from Google Takeout
                </li>
                <li>
                  Edited Google Photos are de-duplicated automatically — the
                  edited copy is kept and the unedited original is skipped
                </li>
                <li>Click <strong>Import</strong> to encrypt and upload</li>
              </ol>
            </div>

            <div
              className={`border-2 border-dashed rounded-lg p-8 text-center transition-colors mb-6 ${
                dragOver
                  ? "border-accent-500 dark:border-accent-400 bg-accent-50 dark:bg-accent-900/30"
                  : "border-edge-strong hover:border-edge-strong"
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
              <p className="text-fg-muted font-medium mb-1">
                Drag & drop photos, videos, and JSON metadata files here
              </p>
              <p className="text-fg-muted text-sm mb-4">or click to browse</p>
              <label className="btn btn-primary btn-md inline-block cursor-pointer">
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

        {notice && (
          <div className="bg-accent-50 dark:bg-accent-900/30 border border-accent-200 dark:border-accent-800 text-accent-800 dark:text-accent-300 rounded-lg p-3 mb-4 text-sm">
            {notice}
          </div>
        )}

        {/* Stats bar (local mode) */}
        {items.length > 0 && (
          <div className="card flex flex-wrap items-center justify-between p-4 mb-4 gap-3">
            <div className="flex flex-wrap gap-4 text-sm">
              <span className="text-fg-muted"><strong>{items.length}</strong> files</span>
              <span className="text-fg-muted">{formatBytes(items.reduce((sum, i) => sum + i.size, 0))}</span>
              {withMetadata > 0 && (
                <span className="text-green-700 dark:text-green-400"><strong>{withMetadata}</strong> with metadata</span>
              )}
              {completedCount > 0 && (
                <span className="text-accent-700 dark:text-accent-300"><strong>{completedCount}</strong> imported</span>
              )}
              {errorCount > 0 && (
                <span className="text-red-700 dark:text-red-400"><strong>{errorCount}</strong> failed</span>
              )}
            </div>
            <div className="flex gap-2">
              {importing && (
                <button onClick={stopImport} className="btn btn-danger btn-md">
                  Stop
                </button>
              )}
              {!importing && pendingCount > 0 && (
                <button onClick={handleImport} className="btn btn-success btn-md">
                  Import {pendingCount} Files
                </button>
              )}
              {!importing && errorCount > 0 && (
                <button onClick={retryFailed} className="bg-yellow-600 text-white px-4 py-2 rounded-md hover:bg-yellow-700 text-sm font-medium">
                  Retry {errorCount} Failed
                </button>
              )}
              {!importing && (
                <button onClick={clearAll} className="btn btn-secondary btn-md">
                  Clear
                </button>
              )}
            </div>
          </div>
        )}

        {/* Progress bar */}
        {importing && (
          <div className="mb-4">
            <div className="flex items-center justify-between text-sm text-fg-muted mb-1">
              <span>Importing… {progress.done}/{progress.total}</span>
              <span>{progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0}%</span>
            </div>
            <div className="w-full h-2 bg-edge-strong rounded-full overflow-hidden">
              <div
                className="h-full bg-accent-600 rounded-full transition-all duration-300"
                style={{ width: `${progress.total > 0 ? (progress.done / progress.total) * 100 : 0}%` }}
              />
            </div>
          </div>
        )}

        {/* File list (local mode) */}
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
              className="mt-3 btn btn-success btn-md"
            >
              View Gallery →
            </button>
          </div>
        )}
      </main>
    </div>
  );
}
