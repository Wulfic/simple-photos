/**
 * Trash page — displays soft-deleted photos with restore/permanent-delete
 * actions. Uses local IndexedDB cache with decrypted thumbnails.
 */
import { useEffect, useState, useCallback } from "react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { db, type CachedTrashItem } from "../db";
import AppHeader from "../components/AppHeader";
import { formatBytes, getErrorMessage } from "../utils/formatters";
import AppIcon from "../components/AppIcon";
import { useThumbnailSizeStore } from "../store/thumbnailSize";
import { useIsBackupServer } from "../hooks/useIsBackupServer";

// ── Types ─────────────────────────────────────────────────────────────────────

interface TrashItem {
  id: string;
  photo_id: string;
  filename: string;
  file_path: string;
  mime_type: string;
  media_type: string;
  size_bytes: number;
  width: number;
  height: number;
  duration_secs: number | null;
  taken_at: string | null;
  thumb_path: string | null;
  deleted_at: string;
  expires_at: string;
  encrypted_blob_id: string | null;
  thumbnail_blob_id: string | null;
  /** Local thumbnail object URL for encrypted items */
  _localThumbUrl?: string;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function timeUntil(isoDate: string): string {
  const now = new Date();
  const expires = new Date(isoDate);
  const diff = expires.getTime() - now.getTime();

  if (diff <= 0) return "Expiring soon";

  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  const hours = Math.floor((diff % (1000 * 60 * 60 * 24)) / (1000 * 60 * 60));

  if (days > 0) return `${days}d ${hours}h remaining`;
  return `${hours}h remaining`;
}

function timeSince(isoDate: string): string {
  const now = new Date();
  const deleted = new Date(isoDate);
  const diff = now.getTime() - deleted.getTime();

  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  const hours = Math.floor((diff % (1000 * 60 * 60 * 24)) / (1000 * 60 * 60));

  if (days > 0) return `${days}d ago`;
  if (hours > 0) return `${hours}h ago`;
  return "Just now";
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function Trash() {
  const { accessToken } = useAuthStore();
  const gridClasses = useThumbnailSizeStore((s) => s.gridClasses)();
  const isBackupServer = useIsBackupServer();
  const [items, setItems] = useState<TrashItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [confirmEmpty, setConfirmEmpty] = useState(false);

  // ── Load trash items ────────────────────────────────────────────────────

  const loadTrash = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const resp = await api.trash.list({ limit: 500 });

      // For encrypted items, load cached thumbnails from local Dexie trash table
      const enriched: TrashItem[] = [];
      for (const item of resp.items) {
        if (item.encrypted_blob_id) {
          const localItem = await db.trash.get(item.id);
          if (localItem?.thumbnailData) {
            const mime = localItem.mediaType === "gif" ? "image/gif" : "image/jpeg";
            const blob = new Blob([localItem.thumbnailData], {
              type: mime,
            });
            (item as TrashItem)._localThumbUrl = URL.createObjectURL(blob);
          }
        }
        enriched.push(item as TrashItem);
      }

      setItems(enriched);
    } catch (e: unknown) {
      setError(getErrorMessage(e, "Failed to load trash"));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadTrash();
  }, [loadTrash]);

  // ── Actions ─────────────────────────────────────────────────────────────

  async function handleRestore(id: string) {
    setActionLoading(id);
    try {
      const item = items.find((i) => i.id === id);
      await api.trash.restore(id);

      // For encrypted items, restore the local Dexie photo record
      if (item?.encrypted_blob_id) {
        const localTrash = await db.trash.get(id);
        if (localTrash) {
          await db.photos.put({
            blobId: localTrash.blobId,
            thumbnailBlobId: localTrash.thumbnailBlobId,
            filename: localTrash.filename,
            mimeType: localTrash.mimeType,
            mediaType: localTrash.mediaType,
            width: localTrash.width,
            height: localTrash.height,
            takenAt: localTrash.takenAt,
            thumbnailData: localTrash.thumbnailData,
            duration: localTrash.duration,
            albumIds: localTrash.albumIds ?? [],
          });
          await db.trash.delete(id);
        }
        // Revoke the local thumbnail URL
        if (item._localThumbUrl) URL.revokeObjectURL(item._localThumbUrl);
      }

      setItems((prev) => prev.filter((i) => i.id !== id));
      setSelectedIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } catch (e: unknown) {
      setError(getErrorMessage(e, "Failed to restore"));
    } finally {
      setActionLoading(null);
    }
  }

  async function handlePermanentDelete(id: string) {
    setActionLoading(id);
    try {
      const item = items.find((i) => i.id === id);
      await api.trash.permanentDelete(id);

      // Clean up local Dexie trash entry for encrypted items
      if (item?.encrypted_blob_id) {
        await db.trash.delete(id).catch((e) => {
          console.error(`Failed to delete trash entry ${id} from local DB:`, e);
        });
        if (item._localThumbUrl) URL.revokeObjectURL(item._localThumbUrl);
      }

      setItems((prev) => prev.filter((i) => i.id !== id));
      setSelectedIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } catch (e: unknown) {
      setError(getErrorMessage(e, "Failed to delete"));
    } finally {
      setActionLoading(null);
    }
  }

  async function handleEmptyTrash() {
    setActionLoading("empty");
    try {
      await api.trash.emptyTrash();
      // Clean up all local Dexie trash entries and revoke URLs
      for (const item of items) {
        if (item._localThumbUrl) URL.revokeObjectURL(item._localThumbUrl);
      }
      await db.trash.clear();
      setItems([]);
      setSelectedIds(new Set());
      setConfirmEmpty(false);
    } catch (e: unknown) {
      setError(getErrorMessage(e, "Failed to empty trash"));
    } finally {
      setActionLoading(null);
    }
  }

  async function handleRestoreSelected() {
    setActionLoading("bulk-restore");
    try {
      for (const id of selectedIds) {
        const item = items.find((i) => i.id === id);
        await api.trash.restore(id);

        // For encrypted items, restore local Dexie photo record
        if (item?.encrypted_blob_id) {
          const localTrash = await db.trash.get(id);
          if (localTrash) {
            await db.photos.put({
              blobId: localTrash.blobId,
              thumbnailBlobId: localTrash.thumbnailBlobId,
              filename: localTrash.filename,
              mimeType: localTrash.mimeType,
              mediaType: localTrash.mediaType,
              width: localTrash.width,
              height: localTrash.height,
              takenAt: localTrash.takenAt,
              thumbnailData: localTrash.thumbnailData,
              duration: localTrash.duration,
              albumIds: localTrash.albumIds ?? [],
            });
            await db.trash.delete(id);
          }
          if (item._localThumbUrl) URL.revokeObjectURL(item._localThumbUrl);
        }
      }
      setItems((prev) => prev.filter((i) => !selectedIds.has(i.id)));
      setSelectedIds(new Set());
    } catch (e: unknown) {
      setError(getErrorMessage(e, "Failed to restore selected items"));
      loadTrash(); // Refresh to get accurate state
    } finally {
      setActionLoading(null);
    }
  }

  async function handleDeleteSelected() {
    setActionLoading("bulk-delete");
    try {
      for (const id of selectedIds) {
        const item = items.find((i) => i.id === id);
        await api.trash.permanentDelete(id);
        if (item?.encrypted_blob_id) {
          await db.trash.delete(id).catch((e) => {
            console.error(`Failed to delete trash entry ${id} from local DB:`, e);
          });
          if (item._localThumbUrl) URL.revokeObjectURL(item._localThumbUrl);
        }
      }
      setItems((prev) => prev.filter((i) => !selectedIds.has(i.id)));
      setSelectedIds(new Set());
    } catch (e: unknown) {
      setError(getErrorMessage(e, "Failed to delete selected items"));
      loadTrash();
    } finally {
      setActionLoading(null);
    }
  }

  // ── Selection ───────────────────────────────────────────────────────────

  function toggleSelect(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function selectAll() {
    if (selectedIds.size === items.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(items.map((i) => i.id)));
    }
  }

  // ── Render ──────────────────────────────────────────────────────────────

  const totalSize = items.reduce((sum, i) => sum + i.size_bytes, 0);

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="max-w-screen-2xl mx-auto px-4 py-6">
        {/* ── Stats Bar ─────────────────────────────────────────────── */}
        <div className="mb-6">
          <div className="flex items-center justify-between">
            <div>
              <h1 className="text-2xl font-bold text-gray-900 dark:text-white flex items-center gap-3">
                <AppIcon name="trashcan" size="w-7 h-7" />
                Trash
              </h1>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                {items.length === 0
                  ? "No items in trash"
                  : `${items.length} item${items.length !== 1 ? "s" : ""} · ${formatBytes(totalSize)} · Items are permanently deleted after 30 days`}
              </p>
            </div>
          </div>

          {/* Action buttons — stacked below the header */}
          <div className="flex flex-col items-start gap-2 mt-4">
            {!isBackupServer && items.length > 0 && (
              <button
                onClick={() => setConfirmEmpty(true)}
                disabled={actionLoading !== null}
                className="px-4 py-2 text-sm font-medium text-white bg-red-600 hover:bg-red-700 rounded-lg transition-colors disabled:opacity-50 flex items-center gap-2"
              >
                <AppIcon name="trashcan" />
                Empty Trash
              </button>
            )}
            {!isBackupServer && selectedIds.size > 0 && (
              <div className="flex items-center gap-2">
                <button
                  onClick={handleRestoreSelected}
                  disabled={actionLoading !== null}
                  className="px-3 py-1.5 text-sm font-medium text-green-700 dark:text-green-400 bg-green-100 dark:bg-green-900/30 rounded-lg hover:bg-green-200 dark:hover:bg-green-900/50 transition-colors disabled:opacity-50"
                >
                  Restore ({selectedIds.size})
                </button>
                <button
                  onClick={handleDeleteSelected}
                  disabled={actionLoading !== null}
                  className="px-3 py-1.5 text-sm font-medium text-red-700 dark:text-red-400 bg-red-100 dark:bg-red-900/30 rounded-lg hover:bg-red-200 dark:hover:bg-red-900/50 transition-colors disabled:opacity-50"
                >
                  Delete ({selectedIds.size})
                </button>
              </div>
            )}
          </div>
        </div>

        {/* ── Error ─────────────────────────────────────────────────── */}
        {error && (
          <div className="mb-4 p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 rounded-lg text-sm flex items-center justify-between">
            <span>{error}</span>
            <button onClick={() => setError(null)} className="text-red-500 hover:text-red-700">
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        )}

        {/* ── Loading ───────────────────────────────────────────────── */}
        {loading && (
          <div className="flex items-center justify-center py-20">
            <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
          </div>
        )}

        {/* ── Empty State ───────────────────────────────────────────── */}
        {!loading && items.length === 0 && (
          <div className="flex flex-col items-center justify-center py-20 text-gray-400 dark:text-gray-500">
            <AppIcon name="trashcan" size="w-16 h-16" className="mb-4" />
            <p className="text-lg font-medium">Trash is empty</p>
            <p className="text-sm mt-1">Deleted photos will appear here for 30 days</p>
          </div>
        )}

        {/* ── Select All ────────────────────────────────────────────── */}
        {!loading && items.length > 0 && (
          <div className="flex items-center gap-3 mb-4">
            <button
              onClick={selectAll}
              className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
            >
              {selectedIds.size === items.length ? "Deselect all" : "Select all"}
            </button>
            {selectedIds.size > 0 && (
              <span className="text-sm text-gray-500 dark:text-gray-400">
                {selectedIds.size} selected
              </span>
            )}
          </div>
        )}

        {/* ── Photo Grid ────────────────────────────────────────────── */}
        {!loading && items.length > 0 && (
          <div className={gridClasses}>
            {items.map((item) => {
              const isSelected = selectedIds.has(item.id);
              const thumbUrl = item._localThumbUrl
                ? item._localThumbUrl
                : accessToken
                  ? `${api.trash.thumbUrl(item.id)}?token=${accessToken}`
                  : api.trash.thumbUrl(item.id);

              return (
                <div
                  key={item.id}
                  className={`group relative aspect-square rounded-lg overflow-hidden cursor-pointer border-2 transition-all ${
                    isSelected
                      ? "border-blue-500 ring-2 ring-blue-500/30"
                      : "border-transparent hover:border-gray-300 dark:hover:border-gray-600"
                  }`}
                  onClick={() => toggleSelect(item.id)}
                >
                  {/* Thumbnail */}
                  <img
                    src={thumbUrl}
                    alt={item.filename}
                    loading="lazy"
                    className="w-full h-full object-cover bg-gray-200 dark:bg-gray-700"
                    onError={(e) => {
                      (e.target as HTMLImageElement).src = "";
                      (e.target as HTMLImageElement).classList.add("bg-gray-300", "dark:bg-gray-600");
                    }}
                  />

                  {/* Selection checkbox */}
                  <div
                    className={`absolute top-2 left-2 w-6 h-6 rounded-full border-2 flex items-center justify-center transition-all ${
                      isSelected
                        ? "bg-blue-500 border-blue-500 text-white"
                        : "bg-white/70 dark:bg-gray-800/70 border-gray-300 dark:border-gray-500 opacity-0 group-hover:opacity-100"
                    }`}
                  >
                    {isSelected && (
                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                      </svg>
                    )}
                  </div>

                  {/* Media type badge */}
                  {item.media_type !== "photo" && (
                    <div className="absolute top-2 right-2 px-1.5 py-0.5 bg-black/60 text-white text-xs rounded font-medium">
                      {item.media_type === "video" ? "Video" : "GIF"}
                    </div>
                  )}

                  {/* Bottom overlay with info */}
                  <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/70 to-transparent p-2 pt-6 opacity-0 group-hover:opacity-100 transition-opacity">
                    <p className="text-white text-xs truncate font-medium">{item.filename}</p>
                    <div className="flex items-center justify-between mt-0.5">
                      <span className="text-white/70 text-xs">{timeSince(item.deleted_at)}</span>
                      <span className="text-white/70 text-xs">{timeUntil(item.expires_at)}</span>
                    </div>
                  </div>


                </div>
              );
            })}
          </div>
        )}
      </main>

      {/* ── Empty Trash Confirmation Modal ──────────────────────────── */}
      {confirmEmpty && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
          <div className="bg-white dark:bg-gray-800 rounded-2xl shadow-2xl max-w-md w-full mx-4 p-6">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-10 h-10 rounded-full bg-red-100 dark:bg-red-900/30 flex items-center justify-center">
                <svg className="w-5 h-5 text-red-600 dark:text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
                </svg>
              </div>
              <div>
                <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
                  Empty Trash?
                </h3>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  This will permanently delete {items.length} item{items.length !== 1 ? "s" : ""} ({formatBytes(totalSize)}).
                  This action cannot be undone.
                </p>
              </div>
            </div>
            <div className="flex gap-3 mt-6">
              <button
                onClick={() => setConfirmEmpty(false)}
                className="flex-1 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 bg-gray-100 dark:bg-gray-700 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleEmptyTrash}
                disabled={actionLoading === "empty"}
                className="flex-1 px-4 py-2 text-sm font-medium text-white bg-red-600 rounded-lg hover:bg-red-700 transition-colors disabled:opacity-50"
              >
                {actionLoading === "empty" ? "Deleting..." : "Delete All"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
