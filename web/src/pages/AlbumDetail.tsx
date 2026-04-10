/**
 * Album detail page — renders photos for a user-created album or a
 * "smart album" (Favorites, Photos, GIFs, Videos, Audio).
 *
 * Handles album CRUD, photo addition/removal, cover photo selection,
 * and sharing controls.
 */
import { useEffect, useState, useMemo, useRef, useCallback } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { db, type CachedPhoto, type CachedAlbum } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import AlbumTile from "../components/AlbumTile";
import AddPhotosPanel from "../components/AddPhotosPanel";
import JustifiedGrid from "../components/gallery/JustifiedGrid";
import { getErrorMessage } from "../utils/formatters";
import { useIsBackupServer } from "../hooks/useIsBackupServer";
import { useAuthStore } from "../store/auth";

// ── Smart album definitions ───────────────────────────────────────────────────

const SMART_ALBUM_DEFS: Record<string, { label: string; filterEncrypted: (p: CachedPhoto) => boolean }> = {
  "smart-favorites": {
    label: "Favorites",
    filterEncrypted: (p) => !!p.isFavorite,
  },
  "smart-photos": {
    label: "Photos",
    filterEncrypted: (p) => p.mediaType === "photo" || p.mediaType === "gif",
  },
  "smart-gifs": {
    label: "GIFs",
    filterEncrypted: (p) => p.mediaType === "gif",
  },
  "smart-videos": {
    label: "Videos",
    filterEncrypted: (p) => p.mediaType === "video",
  },
  "smart-audio": {
    label: "Audio",
    filterEncrypted: (p) => p.mediaType === "audio",
  },
};

function isSmartAlbum(id: string | undefined): id is string {
  return !!id && id in SMART_ALBUM_DEFS;
}

import type { ShareUser } from "../types/sharing";

export default function AlbumDetail() {
  const { albumId } = useParams<{ albumId: string }>();
  const navigate = useNavigate();

  // ── Smart album rendering (delegates to a separate sub-component) ───────
  if (isSmartAlbum(albumId)) {
    return <SmartAlbumView albumId={albumId} />;
  }

  return <RegularAlbumView albumId={albumId} />;
}

// ── Smart Album View ──────────────────────────────────────────────────────────

function SmartAlbumView({ albumId }: { albumId: string }) {
  const navigate = useNavigate();
  const def = SMART_ALBUM_DEFS[albumId];
  const [loading, setLoading] = useState(true);
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());

  // Encrypted photos from IndexedDB
  const allEncryptedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  useEffect(() => {
    (async () => {
      try {
        const secureRes = await api.secureGalleries.secureBlobIds();
        setSecureBlobIds(new Set(secureRes.blob_ids));
      } catch { /* secure galleries may not be available */ }
      setLoading(false);
    })();
  }, []);

  // Compute filtered photos
  const filteredEncrypted = useMemo(() => {
    if (!allEncryptedPhotos) return [];
    return allEncryptedPhotos
      .filter((p) => !secureBlobIds.has(p.blobId))
      .filter(def.filterEncrypted);
  }, [allEncryptedPhotos, secureBlobIds]);

  const photoCount = filteredEncrypted.length;

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="p-4">
        {/* Sub-header */}
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate("/albums")}
            className="text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 transition-colors shrink-0"
            title="Back to Albums"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          <h2 className="text-xl font-semibold truncate">{def.label}</h2>
          <span className="text-gray-400 text-sm shrink-0">{photoCount} items</span>
        </div>

        {loading ? (
          <p className="text-gray-500 dark:text-gray-400 text-center py-12">Loading…</p>
        ) : photoCount === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
            <p className="text-gray-500 dark:text-gray-400">No {def.label.toLowerCase()} found</p>
          </div>
        ) : (
          <JustifiedGrid
            items={filteredEncrypted}
            getAspectRatio={(p) => (p.width && p.height) ? p.width / p.height : 1}
            getKey={(p) => p.blobId}
            renderItem={(photo, idx) => (
              <AlbumTile
                photo={photo}
                isSelectionMode={false}
                isSelected={false}
                onClick={() => {
                  navigate(`/photo/${photo.blobId}`, {
                    state: {
                      photoIds: filteredEncrypted.map((p) => p.blobId),
                      currentIndex: idx,
                    },
                  });
                }}
                onLongPress={() => {}}
                onRemove={() => {}}
              />
            )}
          />
        )}
      </main>
    </div>
  );
}

// ── Regular Album View ────────────────────────────────────────────────────────

function RegularAlbumView({ albumId }: { albumId: string | undefined }) {
  const navigate = useNavigate();
  const isBackupServer = useIsBackupServer();
  const [error, setError] = useState("");
  const [showAddPhotos, setShowAddPhotos] = useState(false);
  const [showSharePicker, setShowSharePicker] = useState(false);
  const [shareUsers, setShareUsers] = useState<ShareUser[]>([]);
  const [shareSuccess, setShareSuccess] = useState("");
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());

  // Fetch secure blob IDs so secure photos are excluded from regular albums
  useEffect(() => {
    api.secureGalleries.secureBlobIds()
      .then((res) => setSecureBlobIds(new Set(res.blob_ids)))
      .catch((err: unknown) => {
        // 404 = secure galleries feature not available — expected
        const status = (err as { status?: number })?.status;
        if (status !== 404) {
          console.error("Failed to fetch secure blob IDs:", err);
        }
      });
  }, []);

  const album = useLiveQuery(
    () => (albumId ? db.albums.get(albumId) : undefined),
    [albumId]
  );

  const allPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  // Photos that belong to this album (excluding any in secure galleries)
  const albumPhotos = useMemo(() => {
    if (!album || !allPhotos) return [];
    const idSet = new Set(album.photoBlobIds);
    return allPhotos.filter((p) => idSet.has(p.blobId) && !secureBlobIds.has(p.blobId));
  }, [album, allPhotos, secureBlobIds]);

  // Photos NOT in this album (for "add photos" view), also excluding secure photos
  const availablePhotos = useMemo(() => {
    if (!album || !allPhotos) return [];
    const idSet = new Set(album.photoBlobIds);
    return allPhotos.filter((p) => !idSet.has(p.blobId) && !secureBlobIds.has(p.blobId));
  }, [album, allPhotos, secureBlobIds]);

  if (!albumId) {
    return <p className="p-4 text-red-600 dark:text-red-400">Invalid album ID</p>;
  }

  async function removePhoto(blobId: string) {
    if (!album) return;
    try {
      const updated = album.photoBlobIds.filter((id) => id !== blobId);
      await updateAlbumManifest({ ...album, photoBlobIds: updated });
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  // ── Multi-select state ──────────────────────────────────────────────────
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const isSelectionMode = selectedIds.size > 0;
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  function enterSelectionMode(blobId: string) {
    setSelectedIds(new Set([blobId]));
  }

  function toggleSelect(blobId: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(blobId)) next.delete(blobId);
      else next.add(blobId);
      return next;
    });
  }

  function selectAll() {
    setSelectedIds(new Set(albumPhotos.map((p) => p.blobId)));
  }

  function clearSelection() {
    setSelectedIds(new Set());
  }

  async function removeSelected() {
    if (!album || selectedIds.size === 0) return;
    try {
      const updated = album.photoBlobIds.filter((id) => !selectedIds.has(id));
      await updateAlbumManifest({ ...album, photoBlobIds: updated });
      clearSelection();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  const handleTilePointerDown = useCallback((blobId: string) => {
    longPressTimerRef.current = setTimeout(() => {
      enterSelectionMode(blobId);
      longPressTimerRef.current = null;
    }, 500);
  }, []);

  const handleTilePointerUp = useCallback(() => {
    if (longPressTimerRef.current) {
      clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);

  const handleTilePointerLeave = useCallback(() => {
    if (longPressTimerRef.current) {
      clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);

  async function addPhotos(blobIds: string[]) {
    if (!album) return;
    try {
      const updated = [...new Set([...album.photoBlobIds, ...blobIds])];
      // Use first added photo as cover if none exists
      const cover = album.coverPhotoBlobId || updated[0] || undefined;
      await updateAlbumManifest({
        ...album,
        photoBlobIds: updated,
        coverPhotoBlobId: cover,
      });
      setShowAddPhotos(false);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function updateAlbumManifest(updatedAlbum: CachedAlbum) {
    // Delete the old manifest blob on the server
    if (updatedAlbum.manifestBlobId) {
      try {
        await api.blobs.delete(updatedAlbum.manifestBlobId);
      } catch {
        // Old manifest may already be deleted — continue
      }
    }

    // Upload new manifest
    const payload = JSON.stringify({
      v: 1,
      album_id: updatedAlbum.albumId,
      name: updatedAlbum.name,
      created_at: new Date(updatedAlbum.createdAt).toISOString(),
      cover_photo_blob_id: updatedAlbum.coverPhotoBlobId || null,
      photo_blob_ids: updatedAlbum.photoBlobIds,
    });

    const encrypted = await encrypt(new TextEncoder().encode(payload));
    const hash = await sha256Hex(new Uint8Array(encrypted));
    const res = await api.blobs.upload(encrypted, "album_manifest", hash);

    // Update local cache
    await db.albums.put({
      ...updatedAlbum,
      manifestBlobId: res.blob_id,
    });
  }

  async function deleteAlbum() {
    if (!album) return;
    if (!confirm(`Delete album "${album.name}"? Photos will not be deleted.`))
      return;
    try {
      if (album.manifestBlobId) {
        await api.blobs.delete(album.manifestBlobId);
      }
      await db.albums.delete(album.albumId);
      navigate("/albums");
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function openSharePicker() {
    setShowSharePicker(true);
    setShareSuccess("");
    try {
      const users = await api.sharing.listUsers();
      setShareUsers(users);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function handleShareWithUser(userId: string) {
    if (!album) return;
    try {
      // Create a shared album with the same name, then add the user as a member
      const created = await api.sharing.createAlbum(album.name);
      await api.sharing.addMember(created.id, userId);
      setShareSuccess(`Album shared successfully!`);
      setShowSharePicker(false);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  if (!album) {
    return (
      <div className="p-4 text-center py-12">
        <p className="text-gray-500 dark:text-gray-400">Loading album…</p>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      {/* Share user picker modal */}
      {showSharePicker && (
        <div className="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4" onClick={() => setShowSharePicker(false)}>
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-sm w-full p-6" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-lg font-semibold mb-4">Share "{album.name}" with</h3>
            <div className="space-y-2 max-h-64 overflow-y-auto">
              {shareUsers.map((u) => (
                <button
                  key={u.id}
                  onClick={() => handleShareWithUser(u.id)}
                  className="w-full text-left px-3 py-2 rounded-md hover:bg-gray-100 dark:hover:bg-gray-700 text-sm flex items-center gap-2"
                >
                  <svg className="w-5 h-5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
                  </svg>
                  {u.username}
                </button>
              ))}
              {shareUsers.length === 0 && (
                <p className="text-gray-500 text-sm text-center py-4">No other users found</p>
              )}
            </div>
            <button
              onClick={() => setShowSharePicker(false)}
              className="mt-4 w-full py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-md"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      <main className="p-4">
        {/* Sub-header with album name */}
        {shareSuccess && (
          <p className="text-green-600 dark:text-green-400 text-sm mb-4 p-3 bg-green-50 dark:bg-green-900/30 rounded">
            {shareSuccess}
          </p>
        )}
        <div className="flex items-center justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <button
              onClick={() => navigate("/albums")}
              className="text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 transition-colors shrink-0"
              title="Back to Albums"
            >
              <AppIcon name="back-arrow" size="w-5 h-5" />
            </button>
            <h2 className="text-xl font-semibold truncate">{album.name}</h2>
            <span className="text-gray-400 text-sm shrink-0">{album.photoBlobIds.length} items</span>
          </div>

          {/* Action buttons */}
          {!isBackupServer && (
          <div className="flex items-center gap-2 shrink-0">
            <button
              onClick={openSharePicker}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 text-gray-600 dark:text-gray-300 bg-white dark:bg-white/10 border border-gray-200 dark:border-white/10 hover:bg-gray-100 dark:hover:bg-white/20 shadow-sm"
            >
              <AppIcon name="shared" />
              <span className="hidden sm:inline">Share</span>
            </button>
            <button
              onClick={() => setShowAddPhotos(!showAddPhotos)}
              className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 shadow-sm ${
                showAddPhotos
                  ? "bg-blue-600 text-white border border-blue-500 hover:bg-blue-700"
                  : "text-gray-600 dark:text-gray-300 bg-white dark:bg-white/10 border border-gray-200 dark:border-white/10 hover:bg-gray-100 dark:hover:bg-white/20"
              }`}
            >
              {showAddPhotos ? "Done" : "Add Photos"}
            </button>
            <button
              onClick={deleteAlbum}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 text-red-600 dark:text-red-400 bg-white dark:bg-white/10 border border-gray-200 dark:border-white/10 hover:bg-red-50 dark:hover:bg-red-900/30 shadow-sm"
            >
              Delete
            </button>
          </div>
          )}
        </div>

      {error && <p className="text-red-600 dark:text-red-400 text-sm mb-4">{error}</p>}

      {/* Add-photos picker */}
      {showAddPhotos && (
        <AddPhotosPanel
          photos={availablePhotos}
          onAdd={addPhotos}
          onCancel={() => setShowAddPhotos(false)}
        />
      )}

      {/* Album photo grid */}
      {isSelectionMode && (
        <div className="flex items-center justify-between gap-3 mb-4 p-3 bg-blue-50 dark:bg-blue-900/30 rounded-lg">
          <div className="flex items-center gap-3">
            <button
              onClick={clearSelection}
              className="text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
            <span className="text-sm font-medium">{selectedIds.size} selected</span>
            <button
              onClick={selectAll}
              className="text-blue-600 dark:text-blue-400 text-sm hover:underline"
            >
              Select All
            </button>
          </div>
          <button
            onClick={removeSelected}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium bg-orange-600 text-white hover:bg-orange-700 shadow-sm"
          >
            Remove ({selectedIds.size})
          </button>
        </div>
      )}
      {albumPhotos.length === 0 ? (
        <div className="text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
          <p className="text-gray-500 dark:text-gray-400 mb-2">This album is empty</p>
          <p className="text-gray-400 text-sm">
            Click "Add Photos" to add media from your gallery
          </p>
        </div>
      ) : (
        <JustifiedGrid
          items={albumPhotos}
          getAspectRatio={(p) => (p.width && p.height) ? p.width / p.height : 1}
          getKey={(p) => p.blobId}
          renderItem={(photo, idx) => (
            <AlbumTile
              photo={photo}
              isSelectionMode={isSelectionMode}
              isSelected={selectedIds.has(photo.blobId)}
              onClick={() => {
                if (isSelectionMode) {
                  toggleSelect(photo.blobId);
                } else {
                  navigate(`/photo/${photo.blobId}`, {
                    state: {
                      photoIds: albumPhotos.map((p) => p.blobId),
                      currentIndex: idx,
                      albumId,
                    },
                  });
                }
              }}
              onLongPress={() => enterSelectionMode(photo.blobId)}
              onRemove={() => removePhoto(photo.blobId)}
            />
          )}
        />
      )}
      </main>
    </div>
  );
}
