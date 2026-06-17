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
import { useAppNavigate } from "../hooks/useAppNavigate";
import { api } from "../api/client";
import { type CachedPhoto, ACCEPTED_MIME_TYPES, db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import AddToAlbumModal from "../components/AddToAlbumModal";
import { ThumbnailTile, type ThumbnailSource, applyDimensionCorrection, correctDimensionsFromThumbnail } from "../gallery";
import JustifiedGrid from "../components/gallery/JustifiedGrid";
import { GallerySkeleton } from "../components/skeletons";
import { getEffectiveAspectRatio } from "../utils/thumbnailCss";
import { useGalleryData } from "../hooks/useGalleryData";
import { useGalleryUpload } from "../hooks/useGalleryUpload";
import { useBackupStore } from "../store/backup";
import { useAuthStore } from "../store/auth";
import { useIsBackupServer } from "../hooks/useIsBackupServer";
import { toast } from "../store/toast";

// ── Component ─────────────────────────────────────────────────────────────────

export default function Gallery() {
  const navigate = useAppNavigate();

  // ── Core data hook ──────────────────────────────────────────────────────
  const {
    loading, error, setError, encryptedPhotos,
    secureBlobIds,
    loadEncryptedPhotos,
  } = useGalleryData();

  // Surface load/upload errors as a dismissible toast popup instead of an
  // under-navbar red bar (#8). Clearing the source state avoids re-firing.
  useEffect(() => {
    if (error) {
      toast.error(error);
      setError("");
    }
  }, [error, setError]);

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
    uploading, uploadProgress, inputRef, folderInputRef, handleDrop, handleFileInput, handleFolderInput,
  } = useGalleryUpload({ loadEncryptedPhotos, setError });

  // ── Multi-select state (mobile long-press) ─────────────────────────────
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [showUploadMenu, setShowUploadMenu] = useState(false);
  const [showAddToAlbum, setShowAddToAlbum] = useState(false);

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
  /** Toggle every photo in a date group. If all are already selected, clears
   *  just those; otherwise adds them and enters selection mode. */
  function toggleSelectGroup(ids: string[]) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      const allSelected = ids.length > 0 && ids.every((id) => next.has(id));
      if (allSelected) {
        for (const id of ids) next.delete(id);
      } else {
        for (const id of ids) next.add(id);
      }
      if (next.size === 0) {
        setSelectionMode(false);
      } else {
        setSelectionMode(true);
      }
      return next;
    });
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

  // ── Collapse burst stacks (group by burstId, keep first frame) ─────────
  const collapsedPhotos = (() => {
    if (!filteredPhotos) return undefined;
    const burstGroups = new Map<string, CachedPhoto[]>();
    const result: (CachedPhoto & { _burstCount?: number })[] = [];
    for (const photo of filteredPhotos) {
      if (photo.burstId) {
        if (!burstGroups.has(photo.burstId)) {
          burstGroups.set(photo.burstId, []);
        }
        burstGroups.get(photo.burstId)!.push(photo);
      } else {
        result.push(photo);
      }
    }
    // For each burst group, the newest frame represents the stack.  Copy
    // instead of mutating: these objects are shared with the live query
    // cache, and a stale `_burstCount` stamped onto a cached object kept
    // showing a burst badge after the group shrank.
    for (const [, frames] of burstGroups) {
      result.push({ ...frames[0], _burstCount: frames.length });
    }
    // Re-sort by takenAt descending to maintain display order.
    // Photos without a timestamp sort to the end instead of poisoning the
    // comparator with NaN.
    result.sort((a, b) => (b.takenAt ?? 0) - (a.takenAt ?? 0));
    return result;
  })();

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
  type EncryptedDayGroup = { key: string; label: string; photos: (CachedPhoto & { _burstCount?: number })[] };
  const encryptedDayGroups: EncryptedDayGroup[] = (() => {
    if (!collapsedPhotos || collapsedPhotos.length === 0) return [];
    const groups = new Map<string, EncryptedDayGroup>();
    for (const photo of collapsedPhotos) {
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
  const hasContent = collapsedPhotos && collapsedPhotos.length > 0;

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      <main className={`p-4 ${selectionMode && !isBackupView && !isBackupServer ? "pt-16" : ""}`}>
        {/* ── Selection mode bar (fixed just under the AppHeader, always visible) ── */}
        {selectionMode && !isBackupView && !isBackupServer && (
          <div className="fixed top-14 left-0 right-0 z-40 flex items-center justify-between bg-gray-200/95 dark:bg-gray-800/95 backdrop-blur px-4 py-2 shadow-sm">
            <div className="flex items-center gap-3">
              <button onClick={clearSelection} className="text-gray-700 hover:text-fg-muted dark:hover:text-white transition-colors" aria-label="Cancel selection">
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" /></svg>
              </button>
              <span className="text-sm font-medium text-fg-muted">{selectedIds.size} selected</span>
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => setShowAddToAlbum(true)}
                disabled={selectedIds.size === 0}
                className="btn btn-primary btn-md inline-flex items-center"
                title="Add to album"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                </svg>
                Album
              </button>
              <button
                onClick={deleteSelected}
                disabled={selectedIds.size === 0}
                className="btn btn-danger btn-md inline-flex items-center"
                title="Delete"
              >
                <AppIcon name="trashcan" size="w-4 h-4" themed={false} />
                Delete
              </button>
            </div>
          </div>
        )}

        {/* Add-to-album picker */}
        {showAddToAlbum && selectedIds.size > 0 && (
          <AddToAlbumModal
            blobIds={Array.from(selectedIds)}
            onClose={() => setShowAddToAlbum(false)}
            onAdded={(_album, _count) => {
              setShowAddToAlbum(false);
              clearSelection();
            }}
          />
        )}

        {/* Errors surface via the global toast host (#8) */}

        {/* Floating upload button — hidden when viewing a backup server or when this IS a backup server.
            z-[60] keeps the FAB + its upward-opening menu above the conversion/
            import banner (z-50), which sits just above it and otherwise overlaps
            and intercepts taps while convert/import is running (#2). The file
            inputs gate only on local `uploading`, never on server-side
            conversion, so manual upload stays available during background work. */}
        {!isBackupView && !isBackupServer && (
        <div className="fixed bottom-6 right-6 z-[60]">
          {/* Upload menu popover */}
          {showUploadMenu && (
            <>
              {/* Backdrop to close menu on outside click */}
              <div className="fixed inset-0 z-40" onClick={() => setShowUploadMenu(false)} />
              <div className="card shadow-pop absolute bottom-16 right-0 z-50 py-1 min-w-[160px]">
                <label className="flex items-center gap-3 px-4 py-2.5 hover:bg-surface-sunken dark:hover:bg-white/10 cursor-pointer transition-colors">
                  <svg className="w-5 h-5 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14" />
                    <rect x="3" y="3" width="18" height="18" rx="2" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                  <span className="text-sm font-medium text-fg-muted">Select files</span>
                  <input
                    ref={inputRef}
                    type="file"
                    multiple
                    accept={ACCEPTED_MIME_TYPES}
                    className="hidden"
                    onChange={(e) => { setShowUploadMenu(false); handleFileInput(e); }}
                    disabled={uploading}
                  />
                </label>
                <label className="flex items-center gap-3 px-4 py-2.5 hover:bg-surface-sunken dark:hover:bg-white/10 cursor-pointer transition-colors">
                  <svg className="w-5 h-5 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                  </svg>
                  <span className="text-sm font-medium text-fg-muted">Select folder</span>
                  <input
                    ref={folderInputRef}
                    type="file"
                    className="hidden"
                    onChange={(e) => { setShowUploadMenu(false); handleFolderInput(e); }}
                    disabled={uploading}
                    {...({ webkitdirectory: "", directory: "" } as React.InputHTMLAttributes<HTMLInputElement>)}
                  />
                </label>
              </div>
            </>
          )}
          {/* FAB button */}
          <button
            className="w-14 h-14 flex items-center justify-center rounded-2xl bg-accent-600 text-white ring-1 ring-inset ring-white/15 shadow-lg shadow-accent-900/25 hover:bg-accent-500 hover:shadow-xl active:scale-95 cursor-pointer select-none transition-all duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent-400 focus-visible:ring-offset-2 focus-visible:ring-offset-canvas"
            title="Upload photos"
            onClick={() => setShowUploadMenu((v) => !v)}
          >
            <svg xmlns="http://www.w3.org/2000/svg" className="w-7 h-7" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
            </svg>
          </button>
        </div>
        )}

        {/* ── BACKUP SERVER VIEW ────────────────────────────────────────── */}
        {isBackupView ? (
          <div>
            {/* Banner showing which backup server we're browsing */}
            <div className="flex items-center gap-2 bg-accent-50 dark:bg-accent-900/30 border border-accent-200 dark:border-accent-800 rounded-lg px-4 py-2 mb-4">
              <svg className="w-4 h-4 text-accent-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 12h14M12 5l7 7-7 7" />
              </svg>
              <span className="text-sm font-medium text-accent-700 dark:text-accent-300">
                Viewing backup: <span className="font-semibold">{activeBackupServer?.name ?? "Unknown server"}</span>
              </span>
              <span className="ml-auto text-xs text-accent-500 dark:text-accent-400">Read-only</span>
            </div>

            {backupLoading && (
              <p className="text-fg-muted text-center py-12">Loading backup photos…</p>
            )}

            {backupError && (
              <p className="text-red-600 dark:text-red-400 text-sm mb-4">{backupError}</p>
            )}

            {!backupLoading && !backupError && backupPhotos?.length === 0 && (
              <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
                <p className="text-fg-muted mb-2">No media on backup server</p>
                <p className="text-fg-muted text-sm">
                  Photos will appear here once the primary server has synced them.
                </p>
              </div>
            )}

            {/* Backup photos grouped by day */}
            {backupDayGroups.map((group) => (
              <div key={group.key}>
                <div className="flex items-center gap-2 py-2 mt-2 first:mt-0">
                  <h3 className="text-sm font-semibold text-fg-muted">
                    {group.label}
                  </h3>
                  <div className="flex-1 h-px bg-edge" />
                  <span className="text-xs text-fg-muted">
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
                        className="relative w-full h-full bg-edge overflow-hidden cursor-pointer"
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
        {loading && !hasContent && <GallerySkeleton />}

        {!loading && !hasContent && (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted mb-2">No media yet</p>
            <p className="text-fg-muted text-sm">
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
                <h3 className="text-sm font-semibold text-fg-muted">
                  {group.label}
                </h3>
                <div className="flex-1 h-px bg-edge" />
                <span className="text-xs text-fg-muted">
                  {group.photos.length}
                </span>
                {/* Select-all-for-day circle. Tapping toggles selection for every
                    photo in this date group and enters selection mode if needed. */}
                {!isBackupServer && (() => {
                  const groupIds = group.photos.map((p) => p.blobId);
                  const allSelected = groupIds.length > 0 && groupIds.every((id) => selectedIds.has(id));
                  return (
                    <button
                      type="button"
                      onClick={() => toggleSelectGroup(groupIds)}
                      aria-label={allSelected ? `Deselect all on ${group.label}` : `Select all on ${group.label}`}
                      title={allSelected ? "Deselect all on this day" : "Select all on this day"}
                      className={`w-5 h-5 rounded-full border-2 flex items-center justify-center transition-all flex-shrink-0 ${
                        allSelected
                          ? "bg-green-500 border-green-500 shadow"
                          : "bg-white/40 dark:bg-gray-700/60 border-edge-strong hover:bg-white dark:hover:bg-gray-600"
                      }`}
                    >
                      {allSelected && (
                        <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                        </svg>
                      )}
                    </button>
                  );
                })()}
              </div>
              <JustifiedGrid
                items={group.photos}
                getAspectRatio={(p) => getEffectiveAspectRatio(p.width, p.height, p.cropData)}
                getKey={(p) => p.blobId}
                renderItem={(photo, localIdx) => {
                  const globalIdx = groupStartIdx + localIdx;
                  const source: ThumbnailSource = {
                    blobId: photo.blobId,
                    storageBlobId: photo.storageBlobId,
                    serverPhotoId: photo.serverPhotoId,
                    serverSide: photo.serverSide,
                    thumbnailData: photo.thumbnailData,
                    thumbnailMimeType: photo.thumbnailMimeType,
                    encryptedThumbBlobId: photo.thumbnailBlobId,
                  };
                  return (
                    <ThumbnailTile
                      source={source}
                      mediaType={photo.mediaType}
                      filename={photo.filename}
                      cropData={photo.cropData}
                      duration={photo.duration}
                      photoSubtype={photo.photoSubtype}
                      burstCount={(photo as CachedPhoto & { _burstCount?: number })._burstCount}
                      selectionMode={selectionMode}
                      isSelected={selectedIds.has(photo.blobId)}
                      onClick={() => {
                        if (selectionMode) toggleSelect(photo.blobId);
                        else navigate(`/photo/${photo.blobId}`, {
                          state: { photoIds: collapsedPhotos!.map(p => p.blobId), currentIndex: globalIdx },
                        });
                      }}
                      onLongPress={() => {
                        if (!selectionMode) enterSelectionMode(photo.blobId);
                      }}
                      onDimensionMismatch={(nw, nh) => {
                        const correction = correctDimensionsFromThumbnail(nw, nh, photo.width, photo.height);
                        if (correction) {
                          applyDimensionCorrection(
                            photo.blobId, photo.serverPhotoId,
                            correction.width, correction.height,
                          );
                        }
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
