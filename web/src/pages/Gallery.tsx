/**
 * Main gallery page — displays the user's photo/video grid.
 *
 * All photos are encrypted (client-side AES-256-GCM). Delegates data loading
 * to useGalleryData and file upload to useGalleryUpload.
 *
 * When the user switches to "backup" view mode (Settings → Active Server),
 * the gallery fetches the backup server's photo list via the proxy API and
 * displays it as a read-only grid.
 */
import { useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { type CachedPhoto, ACCEPTED_MIME_TYPES, db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import MediaTile from "../components/gallery/MediaTile";
import JustifiedGrid from "../components/gallery/JustifiedGrid";
import { useGalleryData } from "../hooks/useGalleryData";
import { useGalleryUpload } from "../hooks/useGalleryUpload";
import { useBackupStore } from "../store/backup";
import { useAuthStore } from "../store/auth";
import { useIsBackupServer } from "../hooks/useIsBackupServer";

// ── Component ─────────────────────────────────────────────────────────────────

export default function Gallery() {
  const navigate = useNavigate();

  // ── Core data hook ──────────────────────────────────────────────────────
  const {
    loading, error, setError, encryptedPhotos,
    secureBlobIds,
    loadEncryptedPhotos,
  } = useGalleryData();

  // ── Backup view mode ────────────────────────────────────────────────────
  const { viewMode, activeBackupServerId, backupServers } = useBackupStore();
  const accessToken = useAuthStore((s) => s.accessToken);
  const isBackupView = viewMode === "backup" && !!activeBackupServerId;
  const isBackupServer = useIsBackupServer();

  // BackupPhotoRecord shape returned by listBackupPhotos proxy
  type BackupPhotoRecord = {
    id: string; filename: string; file_path: string;
    mime_type: string; media_type: string;
    size_bytes: number; width: number; height: number;
    duration_secs: number | null; taken_at: string | null;
    thumb_path: string | null; created_at: string;
  };

  const [backupPhotos, setBackupPhotos] = useState<BackupPhotoRecord[] | null>(null);
  const [backupLoading, setBackupLoading] = useState(false);
  const [backupError, setBackupError] = useState("");
  const [backupLightboxIdx, setBackupLightboxIdx] = useState<number | null>(null);

  // Load backup photos whenever the view switches to backup mode or the
  // active server changes.
  useEffect(() => {
    if (!isBackupView || !activeBackupServerId) {
      setBackupPhotos(null);
      setBackupError("");
      return;
    }
    let cancelled = false;
    setBackupLoading(true);
    setBackupError("");
    api.backup.listBackupPhotos(activeBackupServerId)
      .then((photos) => {
        if (!cancelled) {
          // Sort newest-first by taken_at / created_at
          const sorted = [...photos].sort((a, b) => {
            const ta = a.taken_at ? new Date(a.taken_at).getTime() : new Date(a.created_at).getTime();
            const tb = b.taken_at ? new Date(b.taken_at).getTime() : new Date(b.created_at).getTime();
            return tb - ta;
          });
          setBackupPhotos(sorted);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setBackupError(err instanceof Error ? err.message : "Failed to load backup photos");
        }
      })
      .finally(() => {
        if (!cancelled) setBackupLoading(false);
      });
    return () => { cancelled = true; };
  }, [isBackupView, activeBackupServerId]);

  // ── Upload ──────────────────────────────────────────────────────────────
  const {
    uploading, uploadProgress, inputRef, handleDrop, handleFileInput,
  } = useGalleryUpload({ loadEncryptedPhotos, setError });

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
        const cached = await db.photos.get(id);
        // Always use encrypted blob soft-delete (encrypted-only mode)
        const blobId = cached?.storageBlobId || id;
        const result = await api.blobs.softDelete(blobId, {
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
        // Remove immediately from local IDB so the gallery updates at once
        await db.photos.delete(id);
      }
      await loadEncryptedPhotos();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to delete selected items");
    }
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

  // ── Backup gallery day-groups ───────────────────────────────────────────
  type BackupDayGroup = { key: string; label: string; photos: BackupPhotoRecord[] };
  const backupDayGroups: BackupDayGroup[] = (() => {
    if (!backupPhotos || backupPhotos.length === 0) return [];
    const groups = new Map<string, BackupDayGroup>();
    for (const photo of backupPhotos) {
      const ts = photo.taken_at ?? photo.created_at;
      const dk = dayKey(ts);
      if (!groups.has(dk)) {
        groups.set(dk, { key: dk, label: dayLabel(ts), photos: [] });
      }
      groups.get(dk)!.photos.push(photo);
    }
    return Array.from(groups.values());
  })();

  const activeBackupServer = backupServers.find((s) => s.id === activeBackupServerId);
  const hasContent = filteredPhotos && filteredPhotos.length > 0;

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="p-4">
        {/* ── Selection mode bar (hidden on backup servers) ──────────── */}
        {selectionMode && !isBackupView && !isBackupServer && (
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

        {/* Floating upload button — hidden when viewing a backup server or when this IS a backup server */}
        {!isBackupView && !isBackupServer && (
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
        )}

        {/* ── BACKUP SERVER VIEW ────────────────────────────────────────── */}
        {isBackupView ? (
          <div>
            {/* Banner showing which backup server we're browsing */}
            <div className="flex items-center gap-2 bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg px-4 py-2 mb-4">
              <svg className="w-4 h-4 text-blue-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 12h14M12 5l7 7-7 7" />
              </svg>
              <span className="text-sm font-medium text-blue-700 dark:text-blue-300">
                Viewing backup: <span className="font-semibold">{activeBackupServer?.name ?? "Unknown server"}</span>
              </span>
              <span className="ml-auto text-xs text-blue-500 dark:text-blue-400">Read-only</span>
            </div>

            {backupLoading && (
              <p className="text-gray-500 dark:text-gray-400 text-center py-12">Loading backup photos…</p>
            )}

            {backupError && (
              <p className="text-red-600 dark:text-red-400 text-sm mb-4">{backupError}</p>
            )}

            {!backupLoading && !backupError && backupPhotos?.length === 0 && (
              <div className="text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
                <p className="text-gray-500 dark:text-gray-400 mb-2">No media on backup server</p>
                <p className="text-gray-400 text-sm">
                  Photos will appear here once the primary server has synced them.
                </p>
              </div>
            )}

            {/* Backup photos grouped by day */}
            {backupDayGroups.map((group) => (
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
                <JustifiedGrid
                  items={group.photos}
                  getAspectRatio={(p) => (p.width && p.height) ? p.width / p.height : 1}
                  getKey={(p) => p.id}
                  renderItem={(photo) => {
                    const thumbUrl = `/api/admin/backup/servers/${activeBackupServerId}/photos/${photo.id}/thumb?token=${encodeURIComponent(accessToken ?? "")}`;
                    const isVideo = photo.media_type === "video";
                    const globalIdx = backupPhotos!.indexOf(photo);
                    return (
                      <div
                        className="relative w-full h-full bg-gray-200 dark:bg-gray-700 overflow-hidden cursor-pointer"
                        title={photo.filename}
                        onClick={() => setBackupLightboxIdx(globalIdx)}
                      >
                        <img
                          src={thumbUrl}
                          alt={photo.filename}
                          className="w-full h-full object-cover"
                          loading="lazy"
                        />
                        {/* Video badge */}
                        {isVideo && (
                          <div className="absolute bottom-1 left-1 bg-black/60 rounded-full p-0.5">
                            <svg className="w-3 h-3 text-white" fill="currentColor" viewBox="0 0 24 24">
                              <path d="M8 5v14l11-7z" />
                            </svg>
                          </div>
                        )}
                      </div>
                    );
                  }}
                />
              </div>
            ))}

            {/* Backup photo lightbox */}
            {backupLightboxIdx !== null && backupPhotos && backupPhotos[backupLightboxIdx] && (() => {
              const photo = backupPhotos[backupLightboxIdx];
              const lightboxThumbUrl = `/api/admin/backup/servers/${activeBackupServerId}/photos/${photo.id}/thumb?token=${encodeURIComponent(accessToken ?? "")}`;
              return (
                <div
                  className="fixed inset-0 z-50 bg-black/90 flex items-center justify-center"
                  onClick={() => setBackupLightboxIdx(null)}
                >
                  {/* Close button */}
                  <button
                    className="absolute top-4 right-4 text-white hover:text-gray-300 z-10"
                    onClick={() => setBackupLightboxIdx(null)}
                  >
                    <svg className="w-8 h-8" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                  {/* Prev button */}
                  {backupLightboxIdx > 0 && (
                    <button
                      className="absolute left-4 top-1/2 -translate-y-1/2 text-white hover:text-gray-300 z-10"
                      onClick={(e) => { e.stopPropagation(); setBackupLightboxIdx(backupLightboxIdx - 1); }}
                    >
                      <svg className="w-10 h-10" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
                      </svg>
                    </button>
                  )}
                  {/* Next button */}
                  {backupLightboxIdx < backupPhotos.length - 1 && (
                    <button
                      className="absolute right-4 top-1/2 -translate-y-1/2 text-white hover:text-gray-300 z-10"
                      onClick={(e) => { e.stopPropagation(); setBackupLightboxIdx(backupLightboxIdx + 1); }}
                    >
                      <svg className="w-10 h-10" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
                      </svg>
                    </button>
                  )}
                  {/* Photo */}
                  <img
                    src={lightboxThumbUrl}
                    alt={photo.filename}
                    className="max-w-full max-h-full object-contain"
                    onClick={(e) => e.stopPropagation()}
                  />
                  {/* Filename */}
                  <div className="absolute bottom-4 left-1/2 -translate-x-1/2 text-white text-sm bg-black/60 px-3 py-1 rounded">
                    {photo.filename}
                  </div>
                </div>
              );
            })()}
          </div>
        ) : (
        /* ── PRIMARY / ENCRYPTED GALLERY VIEW ──────────────────────────── */
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
              <JustifiedGrid
                items={group.photos}
                getAspectRatio={(p) => (p.width && p.height) ? p.width / p.height : 1}
                getKey={(p) => p.blobId}
                renderItem={(photo, localIdx) => {
                  const globalIdx = groupStartIdx + localIdx;
                  return (
                    <MediaTile
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
                }}
              />
            </div>
          );
        })}
      </div>
        )} {/* end primary/backup ternary */}
      </main>
    </div>
  );
}
