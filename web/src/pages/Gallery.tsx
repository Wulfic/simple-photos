import { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import {
  db,
  type CachedPhoto,
  blobTypeFromMime,
  mediaTypeFromMime,
  ACCEPTED_MIME_TYPES,
} from "../db";
import { createFallbackThumbnail, arrayBufferToBase64, base64ToArrayBuffer } from "../utils/media";
import { useLiveQuery } from "dexie-react-hooks";
import { useAuthStore } from "../store/auth";
import { useProcessingStore } from "../store/processing";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { type PlainPhoto, generateThumbnail, getImageDimensions, fetchAllPages } from "../utils/gallery";
import MediaTile from "../components/gallery/MediaTile";
import PlainMediaTile from "../components/gallery/PlainMediaTile";

// ── Types ─────────────────────────────────────────────────────────────────────

type EncryptionMode = "plain" | "encrypted";



// ── Payload shapes ────────────────────────────────────────────────────────────

interface PhotoPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type: "photo" | "gif" | "video";
  width: number;
  height: number;
  duration?: number;
  latitude?: number;
  longitude?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64
}

interface ThumbnailPayload {
  v: number;
  photo_blob_id: string;
  width: number;
  height: number;
  data: string; // base64 JPEG
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function Gallery() {
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState("");
  const [mode, setMode] = useState<EncryptionMode | null>(null);
  const [plainPhotos, setPlainPhotos] = useState<PlainPhoto[]>([]);
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);
  const { startTask, endTask } = useProcessingStore();

  // ── Multi-select state (mobile long-press) ─────────────────────────────
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  function enterSelectionMode(id: string) {
    setSelectionMode(true);
    setSelectedIds(new Set([id]));
  }
  function toggleSelect(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      if (next.size === 0) setSelectionMode(false);
      return next;
    });
  }
  function clearSelection() {
    setSelectionMode(false);
    setSelectedIds(new Set());
  }
  async function deleteSelected() {
    if (selectedIds.size === 0) return;
    try {
      for (const id of selectedIds) {
        if (mode === "plain") await api.photos.delete(id);
        else await api.blobs.delete(id);
      }
      if (mode === "plain") await loadPlainPhotos();
    } catch { /* ignore */ }
    clearSelection();
  }

  // ── Encryption migration state ──────────────────────────────────────────
  const [migrationStatus, setMigrationStatus] = useState("idle");
  const [migrationTotal, setMigrationTotal] = useState(0);
  const [migrationCompleted, setMigrationCompleted] = useState(0);
  const migrationRunningRef = useRef(false);

  // ── Secure gallery (private) blob IDs to hide from main gallery ─────────
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());

  // Encrypted-mode: cached photos from IndexedDB (only used in encrypted mode)
  const photos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  useEffect(() => {
    async function init() {
      try {
        const settings = await api.encryption.getSettings();
        const detected = settings.encryption_mode as EncryptionMode;
        setMode(detected);

        // Track migration state so the gallery can run the migration worker
        setMigrationStatus(settings.migration_status);
        setMigrationTotal(settings.migration_total);
        setMigrationCompleted(settings.migration_completed);

        // Fetch blob IDs that are in secure galleries (to hide from main gallery)
        try {
          const secureRes = await api.secureGalleries.secureBlobIds();
          setSecureBlobIds(new Set(secureRes.blob_ids));
        } catch {
          // Secure galleries may not be available — ignore
        }

        // Trigger a background auto-scan whenever the gallery opens
        api.backup.triggerAutoScan().catch(() => {
          // Non-critical — if the user isn't admin or endpoint fails, just ignore
        });

        if (detected === "encrypted") {
          if (!hasCryptoKey()) {
            navigate("/setup");
            return;
          }
          await loadEncryptedPhotos();
        } else {
          await loadPlainPhotos();
        }
      } catch {
        // Fallback: if encryption settings endpoint doesn't exist yet, assume encrypted (legacy)
        setMode("encrypted");
        if (!hasCryptoKey()) {
          navigate("/setup");
          return;
        }
        await loadEncryptedPhotos();
      }
    }
    init();
  }, []);

  // ── Encryption migration worker (runs in a dedicated Web Worker) ──────────
  // When the server reports an active "encrypting" migration, we spawn a
  // Web Worker that handles download → encrypt → upload for each photo.
  // Web Workers are NOT subject to background-tab throttling, so encryption
  // continues at full speed even when the user switches tabs or minimizes.
  const migrationWorkerRef = useRef<Worker | null>(null);

  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;
    if (!hasCryptoKey()) return;

    migrationRunningRef.current = true;
    startTask("encryption");

    (async () => {
      try {
        console.log("[Gallery Migration] Fetching photo list for worker...");

        // Fetch ALL plain photos that need encrypting
        const allPhotos: PlainPhoto[] = [];
        let cursor: string | undefined;
        do {
          const res = await api.photos.list({ after: cursor, limit: 200 });
          allPhotos.push(...res.photos);
          cursor = res.next_cursor ?? undefined;
        } while (cursor);

        if (allPhotos.length === 0) {
          console.log("[Gallery Migration] No photos to encrypt, marking done");
          await api.encryption.reportProgress({ completed_count: 0, done: true });
          setMigrationStatus("idle");
          migrationRunningRef.current = false;
          endTask("encryption");
          return;
        }

        setMigrationTotal(allPhotos.length);
        console.log(`[Gallery Migration] Spawning worker for ${allPhotos.length} photos`);

        // Get auth tokens and encryption key for the worker
        const { accessToken, refreshToken } = useAuthStore.getState();
        const keyHex = sessionStorage.getItem("sp_key");
        if (!accessToken || !keyHex) {
          throw new Error("Missing auth token or encryption key");
        }

        // Spawn the migration Web Worker
        const worker = new Worker(
          new URL("../workers/migrationWorker.ts", import.meta.url),
          { type: "module" }
        );
        migrationWorkerRef.current = worker;

        worker.onmessage = async (e) => {
          const msg = e.data;

          if (msg.type === "progress") {
            setMigrationCompleted(msg.completed);
          } else if (msg.type === "done") {
            console.log(
              `[Gallery Migration] Worker done: ${msg.succeeded}/${msg.total} succeeded, ${msg.failed} failed`
            );
            setMigrationStatus("idle");
            migrationRunningRef.current = false;
            migrationWorkerRef.current = null;
            worker.terminate();
            endTask("encryption");
            await loadEncryptedPhotos();
          } else if (msg.type === "error") {
            console.error("[Gallery Migration] Worker error:", msg.message);
            migrationRunningRef.current = false;
            migrationWorkerRef.current = null;
            worker.terminate();
            endTask("encryption");
          } else if (msg.type === "tokenUpdate") {
            // Worker refreshed the token — update main thread stores
            useAuthStore.getState().setTokens(msg.accessToken, msg.refreshToken);
          }
        };

        worker.onerror = (err) => {
          console.error("[Gallery Migration] Worker fatal error:", err);
          migrationRunningRef.current = false;
          migrationWorkerRef.current = null;
          endTask("encryption");
        };

        // Start the worker
        worker.postMessage({
          type: "start",
          accessToken,
          refreshToken: refreshToken || "",
          keyHex,
          photos: allPhotos,
        });
      } catch (err: any) {
        console.error("[Gallery Migration] Setup error:", err.message);
        await api.encryption.reportProgress({
          completed_count: 0,
          error: `Migration failed: ${err.message}`,
        }).catch(() => {});
        migrationRunningRef.current = false;
        endTask("encryption");
      }
    })();

    return () => {
      // Cleanup worker if component unmounts during migration
      if (migrationWorkerRef.current) {
        migrationWorkerRef.current.terminate();
        migrationWorkerRef.current = null;
      }
    };
  }, [migrationStatus]);

  // ── Load plain-mode photos from server ─────────────────────────────────────

  async function loadPlainPhotos() {
    setLoading(true);
    try {
      const allPhotos: PlainPhoto[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.list({ after: cursor, limit: 200 });
        allPhotos.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      setPlainPhotos(allPhotos.sort((a, b) => {
        // Sort by taken_at descending, fallback to created_at
        const aDate = a.taken_at || a.created_at;
        const bDate = b.taken_at || b.created_at;
        return bDate.localeCompare(aDate);
      }));
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load photos");
    } finally {
      setLoading(false);
    }
  }

  // ── Load encrypted-mode photos from blobs + IndexedDB ───────────────────────

  async function loadEncryptedPhotos() {
    setLoading(true);
    try {
      // Fetch ALL blob types that represent media, with full pagination
      const allMedia = [
        ...(await fetchAllPages("photo")),
        ...(await fetchAllPages("gif")),
        ...(await fetchAllPages("video")),
      ];

      // Build set of server-side blob IDs for stale-entry cleanup
      const serverBlobIds = new Set(allMedia.map((b) => b.id));

      // Remove stale entries from IndexedDB that no longer exist on server
      // (e.g. after a server DB reset or blob deletion)
      const cachedPhotos = await db.photos.toArray();
      const staleIds = cachedPhotos
        .map((p) => p.blobId)
        .filter((id) => !serverBlobIds.has(id));
      if (staleIds.length > 0) {
        await db.photos.bulkDelete(staleIds);
      }

      for (const blob of allMedia) {
        const existing = await db.photos.get(blob.id);
        if (existing) continue;

        try {
          const encrypted = await api.blobs.download(blob.id);
          const decrypted = await decrypt(encrypted);
          const payload: PhotoPayload = JSON.parse(new TextDecoder().decode(decrypted));

          let thumbnailData: ArrayBuffer | undefined;
          if (payload.thumbnail_blob_id) {
            try {
              const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              thumbnailData = base64ToArrayBuffer(thumbPayload.data);
            } catch {
              // Thumbnail fetch failed — show placeholder
            }
          }

          await db.photos.put({
            blobId: blob.id,
            thumbnailBlobId: payload.thumbnail_blob_id,
            filename: payload.filename,
            takenAt: new Date(payload.taken_at).getTime(),
            mimeType: payload.mime_type,
            mediaType: payload.media_type ?? mediaTypeFromMime(payload.mime_type),
            width: payload.width,
            height: payload.height,
            duration: payload.duration,
            latitude: payload.latitude,
            longitude: payload.longitude,
            albumIds: payload.album_ids ?? [],
            thumbnailData,
            contentHash: blob.content_hash ?? undefined,
          });
        } catch {
          // Skip items we can't decrypt (wrong key or corrupt blob)
        }
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load");
    } finally {
      setLoading(false);
    }
  }

  // ── Upload ───────────────────────────────────────────────────────────────────

  const handleUpload = useCallback(async (files: FileList) => {
    if (mode === "plain") {
      // In plain mode, files must already be in the storage directory.
      // Trigger a server-side scan to register them.
      setUploading(true);
      startTask("upload");
      setError("");
      try {
        const res = await api.admin.scanAndRegister();
        if (res.registered > 0) {
          await loadPlainPhotos();
        } else {
          setError("No new files found. Place files in the storage directory first.");
        }
      } catch (err: unknown) {
        setError(err instanceof Error ? err.message : "Scan failed");
      } finally {
        setUploading(false);
        endTask("upload");
      }
      return;
    }

    // Encrypted mode: encrypt and upload
    setUploading(true);
    startTask("upload");
    setError("");

    const IMAGE_VIDEO_EXTENSIONS = /\.(jpe?g|png|gif|webp|heic|heif|avif|bmp|tiff?|dng|cr2|nef|arw|orf|rw2|mp4|mov|avi|mkv|webm|m4v|3gp)$/i;
    const fileArray = Array.from(files).filter(
      (f) => f.type.startsWith("image/") || f.type.startsWith("video/") || IMAGE_VIDEO_EXTENSIONS.test(f.name)
    );

    setUploadProgress({ done: 0, total: fileArray.length });

    try {
      for (let i = 0; i < fileArray.length; i++) {
        const file = fileArray[i];
        setUploadProgress({ done: i, total: fileArray.length });
        await uploadSingleFile(file);
      }
      setUploadProgress({ done: fileArray.length, total: fileArray.length });
      await loadEncryptedPhotos();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setUploading(false);
      setUploadProgress(null);
      endTask("upload");
    }
  }, []);

  async function uploadSingleFile(file: File) {
    const arrayBuf = await file.arrayBuffer();
    const data = new Uint8Array(arrayBuf);
    const mediaType = mediaTypeFromMime(file.type);
    const serverBlobType = blobTypeFromMime(file.type);

    // Generate thumbnail (JPEG frame for videos, scaled image for photos/GIFs)
    let thumbnailData: ArrayBuffer;
    try {
      thumbnailData = await generateThumbnail(file, 256);
    } catch {
      console.warn(`Thumbnail generation failed for ${file.name}, using fallback`);
      thumbnailData = await createFallbackThumbnail();
    }

    // Get actual dimensions
    const dims = await getImageDimensions(file);

    // Get video duration if applicable
    let duration: number | undefined;
    if (mediaType === "video") {
      duration = await getVideoDuration(file);
    }

    // ── Thumbnail blob ───────────────────────────────────────────────────────
    const thumbPayload = JSON.stringify({
      v: 1,
      photo_blob_id: "", // filled after photo upload
      width: 256,
      height: 256,
      data: arrayBufferToBase64(thumbnailData),
    } satisfies Partial<ThumbnailPayload>);

    const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
    const thumbHash = await sha256Hex(new Uint8Array(encThumb));
    // Use video_thumbnail type for video poster frames
    const thumbBlobType = mediaType === "video" ? "video_thumbnail" : "thumbnail";
    const thumbRes = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);

    // ── Media blob ────────────────────────────────────────────────────────────
    const photoPayload = JSON.stringify({
      v: 1,
      filename: file.name,
      taken_at: new Date().toISOString(),
      mime_type: file.type,
      media_type: mediaType,
      width: dims.width,
      height: dims.height,
      duration,
      album_ids: [],
      thumbnail_blob_id: thumbRes.blob_id,
      data: arrayBufferToBase64(data),
    } satisfies Partial<PhotoPayload>);

    const encPhoto = await encrypt(new TextEncoder().encode(photoPayload));
    const photoHash = await sha256Hex(new Uint8Array(encPhoto));
    // Content hash: short hash of original raw bytes for cross-platform alignment
    const contentHash = (await sha256Hex(new Uint8Array(data))).substring(0, 12);
    await api.blobs.upload(encPhoto, serverBlobType, photoHash, contentHash);
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

  // ── Drag & Drop ───────────────────────────────────────────────────────────────

  function handleDrop(e: React.DragEvent) {
    e.preventDefault();
    if (e.dataTransfer.files.length > 0) handleUpload(e.dataTransfer.files);
  }

  function handleFileInput(e: React.ChangeEvent<HTMLInputElement>) {
    if (e.target.files && e.target.files.length > 0) handleUpload(e.target.files);
    // Reset input so the same file can be re-selected
    if (inputRef.current) inputRef.current.value = "";
  }

  // ── Render ────────────────────────────────────────────────────────────────────

  // Filter out photos that are in secure galleries (private)
  const secureFilteredPlain = secureBlobIds.size > 0
    ? plainPhotos.filter((p) => !secureBlobIds.has(p.id))
    : plainPhotos;
  const secureFilteredEncrypted = secureBlobIds.size > 0
    ? photos?.filter((p) => !secureBlobIds.has(p.blobId))
    : photos;

  // All photos (after excluding secure gallery items)
  const filteredPlainPhotos = secureFilteredPlain;
  const filteredPhotos = secureFilteredEncrypted;

  // ── Group photos by day for date separators ─────────────────────────────
  // Matches the Android app's "EEEE, MMMM d, yyyy" format (e.g. "Friday, February 27, 2026")
  const dateFormatter = new Intl.DateTimeFormat("en-US", {
    weekday: "long",
    year: "numeric",
    month: "long",
    day: "numeric",
  });

  // Day key for grouping (YYYY-MM-DD to avoid locale issues)
  function dayKey(timestamp: number | string): string {
    const d = typeof timestamp === "number" ? new Date(timestamp) : new Date(timestamp);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  function dayLabel(timestamp: number | string): string {
    const d = typeof timestamp === "number" ? new Date(timestamp) : new Date(timestamp);
    return dateFormatter.format(d);
  }

  // Group plain photos by day
  type PlainDayGroup = { key: string; label: string; photos: PlainPhoto[] };
  const plainDayGroups: PlainDayGroup[] = (() => {
    if (mode !== "plain" || filteredPlainPhotos.length === 0) return [];
    const groups = new Map<string, PlainDayGroup>();
    // Photos are already sorted by date descending
    let globalIdx = 0;
    for (const photo of filteredPlainPhotos) {
      const ts = photo.taken_at || photo.created_at;
      const dk = dayKey(ts);
      if (!groups.has(dk)) {
        groups.set(dk, { key: dk, label: dayLabel(ts), photos: [] });
      }
      groups.get(dk)!.photos.push(photo);
      globalIdx++;
    }
    return Array.from(groups.values());
  })();

  // Group encrypted photos by day
  type EncryptedDayGroup = { key: string; label: string; photos: CachedPhoto[] };
  const encryptedDayGroups: EncryptedDayGroup[] = (() => {
    if (mode !== "encrypted" || !filteredPhotos || filteredPhotos.length === 0) return [];
    const groups = new Map<string, EncryptedDayGroup>();
    for (const photo of filteredPhotos) {
      const dk = dayKey(photo.takenAt);
      if (!groups.has(dk)) {
        groups.set(dk, { key: dk, label: dayLabel(photo.takenAt), photos: [] });
      }
      groups.get(dk)!.photos.push(photo);
    }
    return Array.from(groups.values());
  })();

  const hasContent = mode === "plain"
    ? filteredPlainPhotos.length > 0
    : (filteredPhotos && filteredPhotos.length > 0);

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="p-4">
        {/* ── Selection mode bar ──────────────────────────────────────── */}
        {selectionMode && (
          <div className="flex items-center justify-between bg-gray-200 dark:bg-gray-800 rounded-lg px-4 py-2 mb-4">
            <div className="flex items-center gap-3">
              <button onClick={clearSelection} className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-white transition-colors">
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" /></svg>
              </button>
              <span className="text-sm font-medium text-gray-700 dark:text-gray-200">{selectedIds.size} selected</span>
            </div>
            <button
              onClick={deleteSelected}
              disabled={selectedIds.size === 0}
              className="inline-flex items-center gap-1.5 bg-red-600 text-white px-3 py-1.5 rounded-md hover:bg-red-500 text-sm font-medium transition-colors disabled:opacity-50"
            >
              <AppIcon name="trash" size="w-4 h-4" themed={false} />
              Delete
            </button>
          </div>
        )}

        {error && <p className="text-red-600 dark:text-red-400 text-sm mb-4">{error}</p>}

        {/* Upload button — encrypted mode only */}
        {mode === "encrypted" && (
          <div className="flex justify-end mb-4">
            <label
              className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3.5 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors shadow-sm cursor-pointer select-none"
              title="Upload photos"
            >
              <AppIcon name="upload" size="w-4 h-4" themed={false} />
              Upload
              <input
                ref={inputRef}
                type="file"
                multiple
                accept={ACCEPTED_MIME_TYPES}
                className="hidden"
                onChange={handleFileInput}
                disabled={uploading}
              />
            </label>
          </div>
        )}

        {/* Migration progress banner */}
        {migrationStatus === "encrypting" && migrationTotal > 0 && (
          <div className="bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg p-4 mb-4">
            <div className="flex items-center gap-3 mb-2">
              <div className="w-5 h-5 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
              <p className="text-sm font-medium text-blue-800 dark:text-blue-300">
                Encrypting photos… {migrationCompleted} / {migrationTotal}
              </p>
            </div>
            <div className="w-full bg-blue-200 dark:bg-blue-800 rounded-full h-2">
              <div
                className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                style={{ width: `${migrationTotal > 0 ? (migrationCompleted / migrationTotal) * 100 : 0}%` }}
              />
            </div>
            <p className="text-xs text-blue-600 dark:text-blue-400 mt-1">
              Your existing photos are being encrypted. This happens automatically in the background.
            </p>
          </div>
        )}

        <div
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDrop}
        >
        {loading && !hasContent && (
          <p className="text-gray-500 dark:text-gray-400 text-center py-12">Loading…</p>
        )}

        {!loading && !hasContent && (
          <div className="text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
            <p className="text-gray-500 dark:text-gray-400 mb-2">No media yet</p>
            <p className="text-gray-400 text-sm">
              {mode === "plain"
                ? "Place photos in the storage directory, then click \"Scan for New Files\""
                : "Drag and drop photos, GIFs, or videos here — or click Upload"}
            </p>
          </div>
        )}

        {/* Plain mode tiles — grouped by day */}
        {mode === "plain" && plainDayGroups.map((group) => {
          // Compute global start index for this group (for photo viewer navigation)
          let groupStartIdx = 0;
          for (const g of plainDayGroups) {
            if (g.key === group.key) break;
            groupStartIdx += g.photos.length;
          }
          return (
            <div key={group.key}>
              <div className="flex items-center gap-2 py-2 mt-2 first:mt-0">
                <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300">
                  {group.label}
                </h3>
                <div className="flex-1 h-px bg-gray-200 dark:bg-gray-700" />
                <span className="text-xs text-gray-400 dark:text-gray-500">
                  {group.photos.length}
                </span>
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
                {group.photos.map((photo, localIdx) => {
                  const globalIdx = groupStartIdx + localIdx;
                  return (
                    <PlainMediaTile
                      key={photo.id}
                      photo={photo}
                      selectionMode={selectionMode}
                      isSelected={selectedIds.has(photo.id)}
                      onClick={() => {
                        if (selectionMode) toggleSelect(photo.id);
                        else navigate(`/photo/plain/${photo.id}`, {
                          state: { photoIds: filteredPlainPhotos.map(p => p.id), currentIndex: globalIdx },
                        });
                      }}
                      onLongPress={() => {
                        if (!selectionMode) enterSelectionMode(photo.id);
                      }}
                    />
                  );
                })}
              </div>
            </div>
          );
        })}

        {/* Encrypted mode tiles — grouped by day */}
        {mode === "encrypted" && encryptedDayGroups.map((group) => {
          let groupStartIdx = 0;
          for (const g of encryptedDayGroups) {
            if (g.key === group.key) break;
            groupStartIdx += g.photos.length;
          }
          return (
            <div key={group.key}>
              <div className="flex items-center gap-2 py-2 mt-2 first:mt-0">
                <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300">
                  {group.label}
                </h3>
                <div className="flex-1 h-px bg-gray-200 dark:bg-gray-700" />
                <span className="text-xs text-gray-400 dark:text-gray-500">
                  {group.photos.length}
                </span>
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
                {group.photos.map((photo, localIdx) => {
                  const globalIdx = groupStartIdx + localIdx;
                  return (
                    <MediaTile
                      key={photo.blobId}
                      photo={photo}
                      selectionMode={selectionMode}
                      isSelected={selectedIds.has(photo.blobId)}
                      onClick={() => {
                        if (selectionMode) toggleSelect(photo.blobId);
                        else navigate(`/photo/${photo.blobId}`, {
                          state: { photoIds: filteredPhotos!.map(p => p.blobId), currentIndex: globalIdx },
                        });
                      }}
                      onLongPress={() => {
                        if (!selectionMode) enterSelectionMode(photo.blobId);
                      }}
                    />
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
      </main>
    </div>
  );
}
