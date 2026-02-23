import { useEffect, useState, useCallback } from "react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import AppHeader from "../components/AppHeader";

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
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + " " + sizes[i];
}

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
      setItems(resp.items);
    } catch (e: any) {
      setError(e.message || "Failed to load trash");
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
      await api.trash.restore(id);
      setItems((prev) => prev.filter((i) => i.id !== id));
      setSelectedIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } catch (e: any) {
      setError(e.message || "Failed to restore");
    } finally {
      setActionLoading(null);
    }
  }

  async function handlePermanentDelete(id: string) {
    setActionLoading(id);
    try {
      await api.trash.permanentDelete(id);
      setItems((prev) => prev.filter((i) => i.id !== id));
      setSelectedIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    } catch (e: any) {
      setError(e.message || "Failed to delete");
    } finally {
      setActionLoading(null);
    }
  }

  async function handleEmptyTrash() {
    setActionLoading("empty");
    try {
      await api.trash.emptyTrash();
      setItems([]);
      setSelectedIds(new Set());
      setConfirmEmpty(false);
    } catch (e: any) {
      setError(e.message || "Failed to empty trash");
    } finally {
      setActionLoading(null);
    }
  }

  async function handleRestoreSelected() {
    setActionLoading("bulk-restore");
    try {
      for (const id of selectedIds) {
        await api.trash.restore(id);
      }
      setItems((prev) => prev.filter((i) => !selectedIds.has(i.id)));
      setSelectedIds(new Set());
    } catch (e: any) {
      setError(e.message || "Failed to restore selected items");
      loadTrash(); // Refresh to get accurate state
    } finally {
      setActionLoading(null);
    }
  }

  async function handleDeleteSelected() {
    setActionLoading("bulk-delete");
    try {
      for (const id of selectedIds) {
        await api.trash.permanentDelete(id);
      }
      setItems((prev) => prev.filter((i) => !selectedIds.has(i.id)));
      setSelectedIds(new Set());
    } catch (e: any) {
      setError(e.message || "Failed to delete selected items");
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
      <AppHeader>
        {items.length > 0 && (
          <div className="flex items-center gap-2">
            {selectedIds.size > 0 && (
              <>
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
              </>
            )}
            <button
              onClick={() => setConfirmEmpty(true)}
              disabled={actionLoading !== null}
              className="px-3 py-1.5 text-sm font-medium text-red-700 dark:text-red-400 bg-red-100 dark:bg-red-900/30 rounded-lg hover:bg-red-200 dark:hover:bg-red-900/50 transition-colors disabled:opacity-50"
            >
              Empty Trash
            </button>
          </div>
        )}
      </AppHeader>

      <main className="max-w-screen-2xl mx-auto px-4 py-6">
        {/* ── Stats Bar ─────────────────────────────────────────────── */}
        <div className="flex items-center justify-between mb-6">
          <div>
            <h1 className="text-2xl font-bold text-gray-900 dark:text-white flex items-center gap-3">
              <svg className="w-7 h-7 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
              </svg>
              Trash
            </h1>
            <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
              {items.length === 0
                ? "No items in trash"
                : `${items.length} item${items.length !== 1 ? "s" : ""} · ${formatBytes(totalSize)} · Items are permanently deleted after 30 days`}
            </p>
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
            <svg className="w-16 h-16 mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
            </svg>
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
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
            {items.map((item) => {
              const isSelected = selectedIds.has(item.id);
              const thumbUrl = accessToken
                ? `${api.trash.thumbUrl(item.id)}?t=${accessToken}`
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

                  {/* Action buttons (visible on hover or when selected) */}
                  <div className={`absolute bottom-0 inset-x-0 flex gap-1 p-2 transition-opacity ${
                    isSelected ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                  }`}>
                    <button
                      onClick={(e) => { e.stopPropagation(); handleRestore(item.id); }}
                      disabled={actionLoading !== null}
                      className="flex-1 py-1 text-xs font-medium text-white bg-green-600/90 hover:bg-green-500 rounded transition-colors disabled:opacity-50"
                      title="Restore"
                    >
                      Restore
                    </button>
                    <button
                      onClick={(e) => { e.stopPropagation(); handlePermanentDelete(item.id); }}
                      disabled={actionLoading !== null}
                      className="flex-1 py-1 text-xs font-medium text-white bg-red-600/90 hover:bg-red-500 rounded transition-colors disabled:opacity-50"
                      title="Delete permanently"
                    >
                      Delete
                    </button>
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
