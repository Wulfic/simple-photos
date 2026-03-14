/**
 * Album detail page — renders photos for a user-created album or a
 * "smart album" (Favorites, Photos, GIFs, Videos, Audio).
 *
 * Supports both plain and encrypted modes and handles album CRUD,
 * photo addition/removal, cover photo selection, and sharing controls.
 */
import { useEffect, useState, useMemo, useRef, useCallback } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import { db, type CachedPhoto, type CachedAlbum } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { type PlainPhoto } from "../utils/gallery";
import PlainMediaTile from "../components/gallery/PlainMediaTile";
import { useThumbnailSizeStore } from "../store/thumbnailSize";
import { getErrorMessage } from "../utils/formatters";

// ── Smart album definitions ───────────────────────────────────────────────────

const SMART_ALBUM_DEFS: Record<string, { label: string; filterEncrypted: (p: CachedPhoto) => boolean; filterPlain: (p: PlainPhoto) => boolean }> = {
  "smart-favorites": {
    label: "Favorites",
    filterEncrypted: () => false, // not supported in encrypted mode
    filterPlain: (p) => p.is_favorite,
  },
  "smart-photos": {
    label: "Photos",
    filterEncrypted: (p) => p.mediaType === "photo" || p.mediaType === "gif",
    filterPlain: (p) => p.media_type === "photo" || p.media_type === "gif",
  },
  "smart-gifs": {
    label: "GIFs",
    filterEncrypted: (p) => p.mediaType === "gif",
    filterPlain: (p) => p.media_type === "gif",
  },
  "smart-videos": {
    label: "Videos",
    filterEncrypted: (p) => p.mediaType === "video",
    filterPlain: (p) => p.media_type === "video",
  },
  "smart-audio": {
    label: "Audio",
    filterEncrypted: (p) => p.mediaType === "audio",
    filterPlain: (p) => p.media_type === "audio",
  },
};

function isSmartAlbum(id: string | undefined): id is string {
  return !!id && id in SMART_ALBUM_DEFS;
}

type ShareUser = { id: string; username: string };

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
  const gridClasses = useThumbnailSizeStore((s) => s.gridClasses)();
  const [encryptionMode, setEncryptionMode] = useState<"plain" | "encrypted" | null>(null);
  const [plainPhotos, setPlainPhotos] = useState<PlainPhoto[]>([]);
  const [loading, setLoading] = useState(true);
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());

  // Encrypted photos from IndexedDB
  const allEncryptedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  useEffect(() => {
    (async () => {
      try {
        // Fetch secure blob IDs
        try {
          const secureRes = await api.secureGalleries.secureBlobIds();
          setSecureBlobIds(new Set(secureRes.blob_ids));
        } catch { /* ignore */ }

        const settings = await api.encryption.getSettings();
        const mode = settings.encryption_mode as "plain" | "encrypted";
        setEncryptionMode(mode);

        if (mode === "plain") {
          // Fetch all plain photos
          const allPhotos: PlainPhoto[] = [];
          let cursor: string | undefined;
          do {
            const res = await api.photos.list({ after: cursor, limit: 200 });
            allPhotos.push(...res.photos);
            cursor = res.next_cursor ?? undefined;
          } while (cursor);
          allPhotos.sort((a, b) => {
            const aDate = a.taken_at || a.created_at;
            const bDate = b.taken_at || b.created_at;
            return bDate.localeCompare(aDate);
          });
          setPlainPhotos(allPhotos);
        }
      } catch { /* fallback */ }
      setLoading(false);
    })();
  }, []);

  // Compute filtered photos
  const filteredEncrypted = useMemo(() => {
    if (encryptionMode !== "encrypted" || !allEncryptedPhotos) return [];
    return allEncryptedPhotos
      .filter((p) => !secureBlobIds.has(p.blobId))
      .filter(def.filterEncrypted);
  }, [allEncryptedPhotos, secureBlobIds, encryptionMode]);

  const filteredPlain = useMemo(() => {
    if (encryptionMode !== "plain") return [];
    return plainPhotos
      .filter((p) => !secureBlobIds.has(p.id))
      .filter(def.filterPlain);
  }, [plainPhotos, secureBlobIds, encryptionMode]);

  const photoCount = encryptionMode === "plain" ? filteredPlain.length : filteredEncrypted.length;

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
        ) : encryptionMode === "encrypted" ? (
          <div className={gridClasses}>
            {filteredEncrypted.map((photo, idx) => (
              <AlbumTile
                key={photo.blobId}
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
            ))}
          </div>
        ) : (
          <div className={gridClasses}>
            {filteredPlain.map((photo, idx) => (
              <PlainMediaTile
                key={photo.id}
                photo={photo}
                onClick={() => {
                  navigate(`/photo/plain/${photo.id}`, {
                    state: {
                      photoIds: filteredPlain.map((p) => p.id),
                      currentIndex: idx,
                    },
                  });
                }}
              />
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

// ── Regular Album View ────────────────────────────────────────────────────────

function RegularAlbumView({ albumId }: { albumId: string | undefined }) {
  const navigate = useNavigate();
  const gridClasses = useThumbnailSizeStore((s) => s.gridClasses)();
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
      .catch(() => { /* secure galleries may not be available */ });
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
      <div className={gridClasses}>
        {albumPhotos.length === 0 && (
          <div className="col-span-full text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
            <p className="text-gray-500 dark:text-gray-400 mb-2">This album is empty</p>
            <p className="text-gray-400 text-sm">
              Click "Add Photos" to add media from your gallery
            </p>
          </div>
        )}

        {albumPhotos.map((photo, idx) => (
          <AlbumTile
            key={photo.blobId}
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
        ))}
      </div>
      </main>
    </div>
  );
}

// ── Add Photos Panel ──────────────────────────────────────────────────────────

interface AddPhotosPanelProps {
  photos: CachedPhoto[];
  onAdd: (ids: string[]) => void;
  onCancel: () => void;
}

function AddPhotosPanel({ photos, onAdd, onCancel }: AddPhotosPanelProps) {
  const [selected, setSelected] = useState<Set<string>>(new Set());

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <div className="mb-6 p-4 bg-blue-50 dark:bg-blue-900/30 rounded-lg">
      <div className="flex items-center justify-between mb-3">
        <p className="text-sm font-medium text-blue-800 dark:text-blue-300">
          Select photos to add ({selected.size} selected)
        </p>
        <div className="flex gap-2">
          <button
            onClick={() => onAdd(Array.from(selected))}
            disabled={selected.size === 0}
            className="bg-blue-600 text-white px-3 py-1 rounded text-sm hover:bg-blue-700 disabled:opacity-50"
          >
            Add Selected
          </button>
          <button
            onClick={onCancel}
            className="bg-gray-200 dark:bg-gray-600 text-gray-700 dark:text-gray-300 px-3 py-1 rounded text-sm hover:bg-gray-300 dark:hover:bg-gray-500"
          >
            Cancel
          </button>
        </div>
      </div>

      {photos.length === 0 ? (
        <p className="text-gray-500 dark:text-gray-400 text-sm">
          All photos are already in this album.
        </p>
      ) : (
        <div className="grid grid-cols-4 sm:grid-cols-6 md:grid-cols-8 gap-1 max-h-64 overflow-y-auto">
          {photos.map((photo) => {
            const isSelected = selected.has(photo.blobId);
            return (
              <div
                key={photo.blobId}
                className={`relative aspect-square rounded overflow-hidden cursor-pointer border-2 ${
                  isSelected ? "border-blue-600" : "border-transparent"
                }`}
                onClick={() => toggle(photo.blobId)}
              >
                <ThumbnailImg photo={photo} />
                {isSelected && (
                  <div className="absolute inset-0 bg-blue-600/30 flex items-center justify-center">
                    <svg
                      className="w-6 h-6 text-white"
                      fill="currentColor"
                      viewBox="0 0 20 20"
                    >
                      <path
                        fillRule="evenodd"
                        d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z"
                        clipRule="evenodd"
                      />
                    </svg>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ── Album Tile ────────────────────────────────────────────────────────────────

interface AlbumTileProps {
  photo: CachedPhoto;
  isSelectionMode: boolean;
  isSelected: boolean;
  onClick: () => void;
  onLongPress: () => void;
  onRemove: () => void;
}

function AlbumTile({ photo, isSelectionMode, isSelected, onClick, onLongPress, onRemove }: AlbumTileProps) {
  const longPressRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const didLongPress = useRef(false);

  function handlePointerDown() {
    didLongPress.current = false;
    longPressRef.current = setTimeout(() => {
      didLongPress.current = true;
      onLongPress();
      longPressRef.current = null;
    }, 500);
  }

  function handlePointerUp() {
    if (longPressRef.current) {
      clearTimeout(longPressRef.current);
      longPressRef.current = null;
    }
    if (!didLongPress.current) {
      onClick();
    }
  }

  function handlePointerLeave() {
    if (longPressRef.current) {
      clearTimeout(longPressRef.current);
      longPressRef.current = null;
    }
  }

  return (
    <div
      className={`relative aspect-square bg-gray-100 dark:bg-gray-700 rounded overflow-hidden cursor-pointer group ${
        isSelected ? "ring-2 ring-blue-500" : ""
      }`}
      onPointerDown={handlePointerDown}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerLeave}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="w-full h-full">
        <ThumbnailImg photo={photo} />
      </div>

      {/* Selection circle */}
      {isSelectionMode && (
        <div className={`absolute top-1.5 right-1.5 w-6 h-6 rounded-full border-2 flex items-center justify-center ${
          isSelected
            ? "bg-green-500 border-green-500"
            : "bg-white/80 border-gray-400/50"
        }`}>
          {isSelected && (
            <svg className="w-4 h-4 text-white" fill="currentColor" viewBox="0 0 20 20">
              <path fillRule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clipRule="evenodd" />
            </svg>
          )}
        </div>
      )}

      {/* Media type badge */}
      {photo.mediaType === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {photo.duration ? (
            <span>{formatDuration(photo.duration)}</span>
          ) : null}
        </div>
      )}
      {photo.mediaType === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}

      {/* Remove button on hover (only when NOT in selection mode) */}
      {!isSelectionMode && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          onPointerDown={(e) => e.stopPropagation()}
          className="absolute top-1 right-1 bg-red-600 text-white rounded-full w-6 h-6 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-xs"
          title="Remove from album"
        >
          ×
        </button>
      )}
    </div>
  );
}

// ── Thumbnail helper ──────────────────────────────────────────────────────────

function ThumbnailImg({ photo }: { photo: CachedPhoto }) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    if (photo.thumbnailData) {
      const url = URL.createObjectURL(
        new Blob([photo.thumbnailData], { type: "image/jpeg" })
      );
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    }
  }, [photo.thumbnailData]);

  if (src) {
    return (
      <img
        src={src}
        alt={photo.filename}
        className="w-full h-full object-cover"
        loading="lazy"
      />
    );
  }

  return (
    <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center bg-gray-100 dark:bg-gray-700">
      {photo.filename}
    </div>
  );
}

function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}
