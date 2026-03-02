import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { type CachedPhoto, ACCEPTED_MIME_TYPES } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { type PlainPhoto } from "../utils/gallery";
import MediaTile from "../components/gallery/MediaTile";
import PlainMediaTile from "../components/gallery/PlainMediaTile";
import { useGalleryData } from "../hooks/useGalleryData";
import { useGalleryMigration } from "../hooks/useGalleryMigration";
import { useGalleryUpload } from "../hooks/useGalleryUpload";

// ── Component ─────────────────────────────────────────────────────────────────

export default function Gallery() {
  const navigate = useNavigate();

  // ── Core data hook ──────────────────────────────────────────────────────
  const {
    mode, loading, error, setError, plainPhotos, encryptedPhotos,
    secureBlobIds, migrationStatus, migrationTotal, migrationCompleted,
    setMigrationStatus, setMigrationTotal, setMigrationCompleted,
    loadPlainPhotos, loadEncryptedPhotos,
  } = useGalleryData();

  // ── Encryption migration ────────────────────────────────────────────────
  useGalleryMigration({
    migrationStatus,
    setMigrationStatus,
    setMigrationTotal,
    setMigrationCompleted,
    loadEncryptedPhotos,
  });

  // ── Upload ──────────────────────────────────────────────────────────────
  const {
    uploading, uploadProgress, inputRef, handleDrop, handleFileInput,
  } = useGalleryUpload({ mode, loadPlainPhotos, loadEncryptedPhotos, setError });

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

  // ── Filter out photos in secure galleries (private) ─────────────────────
  const filteredPlainPhotos = secureBlobIds.size > 0
    ? plainPhotos.filter((p) => !secureBlobIds.has(p.id))
    : plainPhotos;
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

  // Group plain photos by day — always compute, regardless of mode.
  // Plain photos may exist alongside encrypted mode (auto-scanned files, migration).
  type PlainDayGroup = { key: string; label: string; photos: PlainPhoto[] };
  const plainDayGroups: PlainDayGroup[] = (() => {
    if (filteredPlainPhotos.length === 0) return [];
    const groups = new Map<string, PlainDayGroup>();
    for (const photo of filteredPlainPhotos) {
      const ts = photo.taken_at || photo.created_at;
      const dk = dayKey(ts);
      if (!groups.has(dk)) {
        groups.set(dk, { key: dk, label: dayLabel(ts), photos: [] });
      }
      groups.get(dk)!.photos.push(photo);
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

  const hasContent =
    filteredPlainPhotos.length > 0 ||
    (filteredPhotos && filteredPhotos.length > 0);

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
              Place photos in the storage directory or upload them to get started.
            </p>
          </div>
        )}

        {/* Plain photo tiles — shown in plain mode, or during active migration */}
        {plainDayGroups.length > 0 && (mode === "plain" || migrationStatus === "encrypting" || migrationStatus === "decrypting") && plainDayGroups.map((group) => {
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
              <div className="grid grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
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
              <div className="grid grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
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
