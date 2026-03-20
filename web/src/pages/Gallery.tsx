/**
 * Main gallery page — displays the user's photo/video grid.
 *
 * All photos are encrypted (client-side AES-256-GCM). Delegates data loading
 * to useGalleryData and file upload to useGalleryUpload.
 */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { type CachedPhoto, ACCEPTED_MIME_TYPES, db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import MediaTile from "../components/gallery/MediaTile";
import { useGalleryData } from "../hooks/useGalleryData";
import { useGalleryUpload } from "../hooks/useGalleryUpload";
import { useThumbnailSizeStore } from "../store/thumbnailSize";

// ── Component ─────────────────────────────────────────────────────────────────

export default function Gallery() {
  const navigate = useNavigate();

  // ── Core data hook ──────────────────────────────────────────────────────
  const {
    loading, error, setError, encryptedPhotos,
    secureBlobIds,
    loadEncryptedPhotos,
  } = useGalleryData();

  // ── Upload ──────────────────────────────────────────────────────────────
  const {
    uploading, uploadProgress, inputRef, handleDrop, handleFileInput,
  } = useGalleryUpload({ loadEncryptedPhotos, setError });

  // ── Read global activity store (banners rendered by GlobalProgressBanners) ──
  const gridClasses = useThumbnailSizeStore((s) => s.gridClasses)();

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
  /** Delete all selected photos/blobs. Uses encrypted soft-delete (trash with
   *  30-day recovery window). */
  async function deleteSelected() {
    if (selectedIds.size === 0) return;
    try {
      for (const id of selectedIds) {
        // Encrypted mode: soft-delete to trash with client metadata
        const cached = await db.photos.get(id);
        const result = await api.blobs.softDelete(id, {
          thumbnail_blob_id: cached?.thumbnailBlobId,
          filename: cached?.filename ?? "unknown",
          mime_type: cached?.mimeType ?? "application/octet-stream",
          media_type: cached?.mediaType,
          size_bytes: 0,
          width: cached?.width,
          height: cached?.height,
          duration_secs: cached?.duration,
          taken_at: cached?.takenAt
            ? new Date(cached.takenAt).toISOString()
            : undefined,
        });
        // Cache in local trash table for the Trash page thumbnail grid
        if (cached) {
          await db.trash.put({
            trashId: result.trash_id,
            blobId: id,
            thumbnailBlobId: cached.thumbnailBlobId,
            filename: cached.filename,
            mimeType: cached.mimeType,
            mediaType: cached.mediaType,
            width: cached.width,
            height: cached.height,
            takenAt: cached.takenAt,
            deletedAt: Date.now(),
            expiresAt: result.expires_at,
            thumbnailData: cached.thumbnailData,
            duration: cached.duration,
            albumIds: cached.albumIds ?? [],
          });
        }
        await db.photos.delete(id);
      }
      await loadEncryptedPhotos();
    } catch { /* ignore */ }
    clearSelection();
  }

  // ── Filter out photos in secure galleries (private) ─────────────────────
  const filteredPhotos = secureBlobIds.size > 0
    ? encryptedPhotos?.filter((p) => !secureBlobIds.has(p.blobId))
    : encryptedPhotos;

  // ── Group photos by day for date separators ─────────────────────────────
  // Matches the Android app's "EEEE, MMMM d, yyyy" format
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

  // Group encrypted photos by day
  type EncryptedDayGroup = { key: string; label: string; photos: CachedPhoto[] };
  const encryptedDayGroups: EncryptedDayGroup[] = (() => {
    if (!filteredPhotos || filteredPhotos.length === 0) return [];
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

  const hasContent = filteredPhotos && filteredPhotos.length > 0;

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
              <AppIcon name="trashcan" size="w-4 h-4" themed={false} />
              Delete
            </button>
          </div>
        )}

        {error && <p className="text-red-600 dark:text-red-400 text-sm mb-4">{error}</p>}

        {/* Floating upload button */}
        <label
          className="fixed bottom-6 right-6 z-50 w-14 h-14 flex items-center justify-center rounded-2xl shadow-lg cursor-pointer select-none transition-colors"
          style={{ backgroundColor: "#A8C7FA" }}
          title="Upload photos"
        >
          <svg xmlns="http://www.w3.org/2000/svg" className="w-7 h-7" fill="none" viewBox="0 0 24 24" stroke="#1C1B1F" strokeWidth={2.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
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
              Place photos in the storage directory or upload them to get started.
            </p>
          </div>
        )}

        {/* Encrypted mode tiles — grouped by day */}
        {encryptedDayGroups.map((group) => {
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
              <div className={gridClasses}>
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
