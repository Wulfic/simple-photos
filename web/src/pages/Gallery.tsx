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
import { createFallbackThumbnail, arrayBufferToBase64, base64ToArrayBuffer, generateMigrationThumbnail } from "../utils/media";
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

/** Default album filter types for organizing photos */
type AlbumFilter = "all" | "favorites" | "photos" | "gifs" | "videos";

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

  // ── Album filter state ──────────────────────────────────────────────────
  const [albumFilter, setAlbumFilter] = useState<AlbumFilter>("all");

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

  // ── Encryption migration worker ─────────────────────────────────────────
  // When the server reports an active "encrypting" migration (e.g. after
  // initial setup chose encrypted mode and photos were discovered in plain
  // mode), this worker downloads each plain photo, encrypts it client-side,
  // uploads the encrypted blob, and reports progress to the server.
  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;
    if (!hasCryptoKey()) return;

    migrationRunningRef.current = true;
    startTask("encryption");

    (async () => {
      try {
        console.log("[Gallery Migration] Starting encryption migration...");

        // Fetch ALL plain photos that need encrypting
        const allPhotos: PlainPhoto[] = [];
        let cursor: string | undefined;
        do {
          const res = await api.photos.list({ after: cursor, limit: 200 });
          allPhotos.push(...res.photos);
          cursor = res.next_cursor ?? undefined;
        } while (cursor);

        console.log(`[Gallery Migration] Found ${allPhotos.length} photos to encrypt`);
        const total = allPhotos.length;
        let completed = 0;
        let succeeded = 0;
        let failedCount = 0;
        let lastError = "";

        for (const photo of allPhotos) {
          let attempts = 0;
          let itemSuccess = false;

          while (attempts < 3 && !itemSuccess) {
            attempts++;
            try {
              // Step 1: Download the raw file
              const fileBuffer = await api.photos.downloadFile(photo.id);
              const fileData = new Uint8Array(fileBuffer);

              // Step 2: Generate thumbnail
              let thumbBlobId: string | undefined;
              try {
                const thumbData = await generateMigrationThumbnail(fileData, photo.mime_type, 256);
                if (thumbData) {
                  const thumbPayload = JSON.stringify({
                    v: 1, photo_blob_id: "", width: 256, height: 256,
                    data: arrayBufferToBase64(thumbData),
                  });
                  const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
                  const thumbHash = await sha256Hex(new Uint8Array(encThumb));
                  const thumbBlobType = photo.media_type === "video" ? "video_thumbnail" : "thumbnail";
                  const thumbUpload = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);
                  thumbBlobId = thumbUpload.blob_id;
                }
              } catch (thumbErr: any) {
                console.warn(`[Gallery Migration] Thumbnail failed for "${photo.filename}" (continuing):`, thumbErr.message);
              }

              // Step 3: Build and encrypt photo payload
              const serverBlobType = blobTypeFromMime(photo.mime_type);
              const photoPayload = JSON.stringify({
                v: 1,
                filename: photo.filename,
                taken_at: photo.taken_at || photo.created_at,
                mime_type: photo.mime_type,
                media_type: (photo.media_type || mediaTypeFromMime(photo.mime_type)) as "photo" | "gif" | "video",
                width: photo.width,
                height: photo.height,
                duration: photo.duration_secs ?? undefined,
                album_ids: [],
                thumbnail_blob_id: thumbBlobId || "",
                data: arrayBufferToBase64(fileData),
              });

              const encPhoto = await encrypt(new TextEncoder().encode(photoPayload));
              const photoHash = await sha256Hex(new Uint8Array(encPhoto));

              // Content hash: short hash of original raw bytes for cross-platform alignment
              const contentHash = (await sha256Hex(fileData)).substring(0, 12);

              // Step 4: Upload encrypted blob
              const uploadResult = await api.blobs.upload(encPhoto, serverBlobType, photoHash, contentHash);

              // Step 5: Link blob to the plain photo so it won't be re-migrated
              await api.photos.markEncrypted(photo.id, uploadResult.blob_id);

              itemSuccess = true;
              succeeded++;
            } catch (itemErr: any) {
              console.error(`[Gallery Migration] FAILED "${photo.filename}" attempt ${attempts}/3:`, itemErr.message);
              if (attempts >= 3) {
                failedCount++;
                lastError = `Failed on "${photo.filename}": ${itemErr.message}`;
              } else {
                await new Promise((r) => setTimeout(r, 500 * attempts));
              }
            }
          }

          completed++;
          setMigrationCompleted(completed);
          await api.encryption.reportProgress({
            completed_count: completed,
            ...(itemSuccess ? {} : { error: lastError }),
          });
        }

        console.log(`[Gallery Migration] Complete: ${succeeded} succeeded, ${failedCount} failed out of ${total}`);

        // Mark migration complete
        await api.encryption.reportProgress({
          completed_count: total,
          done: true,
          ...(failedCount > 0
            ? { error: `Migration finished with ${failedCount}/${total} failures. Last error: ${lastError}` }
            : {}),
        });

        // Reload encrypted photos so they appear in the gallery
        setMigrationStatus("idle");
        await loadEncryptedPhotos();
      } catch (err: any) {
        console.error("[Gallery Migration] Top-level error:", err.message);
        await api.encryption.reportProgress({
          completed_count: 0,
          error: `Migration failed: ${err.message}`,
        }).catch(() => {});
      } finally {
        migrationRunningRef.current = false;
        endTask("encryption");
      }
    })();
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
  const filteredPhotos = secureBlobIds.size > 0
    ? photos?.filter((p) => !secureBlobIds.has(p.blobId))
    : photos;

  // Apply album filter for plain mode
  const filteredPlainPhotos = (() => {
    switch (albumFilter) {
      case "favorites":
        return secureFilteredPlain.filter((p) => p.is_favorite);
      case "photos":
        return secureFilteredPlain.filter((p) => p.media_type === "photo" || p.media_type === "gif");
      case "gifs":
        return secureFilteredPlain.filter((p) => p.media_type === "gif");
      case "videos":
        return secureFilteredPlain.filter((p) => p.media_type === "video");
      default:
        return secureFilteredPlain;
    }
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

        {/* Default album filter tabs — plain mode only */}
        {mode === "plain" && (
          <div className="flex gap-1 mb-4 overflow-x-auto pb-1">
            {([
              { key: "all" as AlbumFilter, label: "All" },
              { key: "favorites" as AlbumFilter, label: "★ Favorites" },
              { key: "photos" as AlbumFilter, label: "Photos" },
              { key: "gifs" as AlbumFilter, label: "GIFs" },
              { key: "videos" as AlbumFilter, label: "Videos" },
            ]).map(({ key, label }) => (
              <button
                key={key}
                onClick={() => setAlbumFilter(key)}
                className={`px-3 py-1.5 rounded-full text-xs font-medium whitespace-nowrap transition-colors ${
                  albumFilter === key
                    ? "bg-blue-600 text-white"
                    : "bg-gray-200 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-300 dark:hover:bg-gray-600"
                }`}
              >
                {label}
                {key === "favorites" && mode === "plain" && (
                  <span className="ml-1 opacity-70">
                    ({secureFilteredPlain.filter(p => p.is_favorite).length})
                  </span>
                )}
              </button>
            ))}
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
          className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2"
        >
        {loading && !hasContent && (
          <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-12">Loading…</p>
        )}

        {!loading && !hasContent && (
          <div className="col-span-full text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
            <p className="text-gray-500 dark:text-gray-400 mb-2">No media yet</p>
            <p className="text-gray-400 text-sm">
              {mode === "plain"
                ? "Place photos in the storage directory, then click \"Scan for New Files\""
                : "Drag and drop photos, GIFs, or videos here — or click Upload"}
            </p>
          </div>
        )}

        {/* Plain mode tiles */}
        {mode === "plain" && filteredPlainPhotos.map((photo, idx) => (
          <PlainMediaTile
            key={photo.id}
            photo={photo}
            selectionMode={selectionMode}
            isSelected={selectedIds.has(photo.id)}
            onClick={() => {
              if (selectionMode) toggleSelect(photo.id);
              else navigate(`/photo/plain/${photo.id}`, {
                state: { photoIds: filteredPlainPhotos.map(p => p.id), currentIndex: idx },
              });
            }}
            onLongPress={() => {
              if (!selectionMode) enterSelectionMode(photo.id);
            }}
          />
        ))}

        {/* Encrypted mode tiles */}
        {mode === "encrypted" && filteredPhotos?.map((photo, idx) => (
          <MediaTile
            key={photo.blobId}
            photo={photo}
            selectionMode={selectionMode}
            isSelected={selectedIds.has(photo.blobId)}
            onClick={() => {
              if (selectionMode) toggleSelect(photo.blobId);
              else navigate(`/photo/${photo.blobId}`, {
                state: { photoIds: filteredPhotos!.map(p => p.blobId), currentIndex: idx },
              });
            }}
            onLongPress={() => {
              if (!selectionMode) enterSelectionMode(photo.blobId);
            }}
          />
        ))}
      </div>
      </main>
    </div>
  );
}
