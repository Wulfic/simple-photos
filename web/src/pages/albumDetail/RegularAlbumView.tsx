/**
 * Regular (user-created) album view — renders the photos for a manifest-backed
 * album and handles album CRUD, photo addition/removal, multi-select, cover
 * photo selection, secure-add, and sharing controls.
 */
import { useEffect, useState, useMemo, useRef, useCallback } from "react";
import { useLocation } from "react-router-dom";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { useScrollMemory } from "../../hooks/useScrollMemory";
import { api } from "../../api/client";
import { encrypt, sha256Hex } from "../../crypto/crypto";
import { db, type CachedPhoto, type CachedAlbum } from "../../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../../components/AppHeader";
import AppIcon from "../../components/AppIcon";
import AddPhotosPanel from "../../components/AddPhotosPanel";
import JustifiedGrid from "../../components/gallery/JustifiedGrid";
import AlbumTile from "../../components/AlbumTile";
import { getEffectiveAspectRatio } from "../../utils/thumbnailCss";
import { getErrorMessage } from "../../utils/formatters";
import { toast } from "../../store/toast";
import { useIsBackupServer } from "../../hooks/useIsBackupServer";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";
import { useSecureAdd } from "../../store/secureAdd";
import { addPhotosToSecureGallery } from "../../utils/secureAdd";
import type { ShareUser } from "../../types/sharing";
import SharePickerModal from "../../components/SharePickerModal";

export default function RegularAlbumView({ albumId }: { albumId: string | undefined }) {
  const navigate = useAppNavigate();
  const isBackupServer = useIsBackupServer();
  const [error, setError] = useState("");
  const [showAddPhotos, setShowAddPhotos] = useState(false);
  const [showSharePicker, setShowSharePicker] = useState(false);
  const [shareUsers, setShareUsers] = useState<ShareUser[]>([]);
  const [shareSuccess, setShareSuccess] = useState("");
  const [secureBlobIds, setSecureBlobIds] = useState<Set<string>>(new Set());

  // Surface errors as a dismissible toast popup instead of an under-navbar bar
  // (#8). e.g. sharing an album to yourself ("Cannot add yourself as a member").
  useEffect(() => {
    if (error) {
      toast.error(error);
      setError("");
    }
  }, [error]);
  useEffect(() => {
    if (shareSuccess) {
      toast.success(shareSuccess);
      setShareSuccess("");
    }
  }, [shareSuccess]);

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

  // Preserve scroll position when opening a photo and returning to the album.
  const { pathname } = useLocation();
  // Photos that belong to this album (excluding any in secure galleries)
  const albumPhotos = useMemo(() => {
    if (!album || !allPhotos) return [];
    const idSet = new Set(album.photoBlobIds);
    return allPhotos.filter((p) => idSet.has(p.blobId) && !secureBlobIds.has(p.blobId));
  }, [album, allPhotos, secureBlobIds]);

  useScrollMemory(pathname, albumPhotos.length > 0);

  // Photos NOT in this album (for "add photos" view), also excluding secure photos
  const availablePhotos = useMemo(() => {
    if (!album || !allPhotos) return [];
    const idSet = new Set(album.photoBlobIds);
    return allPhotos.filter((p) => !idSet.has(p.blobId) && !secureBlobIds.has(p.blobId));
  }, [album, allPhotos, secureBlobIds]);

  const slideshow = usePhotoSlideshow(albumPhotos);

  // ── Multi-select state ──────────────────────────────────────────────────
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const isSelectionMode = selectedIds.size > 0;
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Secure-add flow: when active, this album offers "Add to 🔒 <name>" and
  // tapping a tile toggles selection (instead of opening the viewer).
  const secureAddTarget = useSecureAdd((s) => s.target);
  const cancelSecureAdd = useSecureAdd((s) => s.cancel);
  const [addingSecure, setAddingSecure] = useState(false);

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

  async function addSelectedToSecure() {
    if (!secureAddTarget || selectedIds.size === 0 || addingSecure) return;
    setAddingSecure(true);
    try {
      const count = await addPhotosToSecureGallery(secureAddTarget.galleryId, [...selectedIds]);
      toast.success(`Added ${count} photo${count !== 1 ? "s" : ""} to ${secureAddTarget.galleryName}`);
      clearSelection();
      const target = secureAddTarget.galleryId;
      cancelSecureAdd();
      navigate(`/secure-gallery?album=${target}`);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setAddingSecure(false);
    }
  }

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
        <p className="text-fg-muted">Loading album…</p>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      {/* Share user picker modal */}
      {showSharePicker && (
        <SharePickerModal
          title={`Share "${album.name}" with`}
          users={shareUsers}
          onPick={(id) => handleShareWithUser(id)}
          onClose={() => setShowSharePicker(false)}
          emptyText="No other users found"
        />
      )}

      <main className="p-4">
        {/* Sub-header with album name */}
        <div className="flex items-center justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <button
              onClick={() => navigate("/albums")}
              className="text-fg-muted hover:text-fg transition-colors shrink-0"
              title="Back to Albums"
            >
              <AppIcon name="back-arrow" size="w-5 h-5" />
            </button>
            <h2 className="text-xl font-semibold truncate">{album.name}</h2>
            <span className="text-fg-muted text-sm shrink-0">{album.photoBlobIds.length} items</span>
            <SlideshowTriggers slideshow={slideshow} />
          </div>

          {/* Action buttons */}
          {!isBackupServer && (
          <div className="flex items-center gap-2 shrink-0">
            <button
              onClick={openSharePicker}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 text-fg-muted bg-white dark:bg-white/10 border border-edge hover:bg-surface-sunken dark:hover:bg-white/20 shadow-sm"
            >
              <AppIcon name="shared" />
              <span className="hidden sm:inline">Share</span>
            </button>
            <button
              onClick={() => setShowAddPhotos(!showAddPhotos)}
              className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 shadow-sm ${
                showAddPhotos
                  ? "bg-accent-600 text-white border border-accent-500 hover:bg-accent-700"
                  : "text-fg-muted bg-white dark:bg-white/10 border border-edge hover:bg-surface-sunken dark:hover:bg-white/20"
              }`}
            >
              {showAddPhotos ? "Done" : "Add Photos"}
            </button>
            <button
              onClick={deleteAlbum}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 text-red-600 dark:text-red-400 bg-white dark:bg-white/10 border border-edge hover:bg-red-50 dark:hover:bg-red-900/30 shadow-sm"
            >
              Delete
            </button>
          </div>
          )}
        </div>

      {/* Errors surface via the global toast host (#8) */}

      {/* Add-photos picker */}
      {showAddPhotos && (
        <AddPhotosPanel
          photos={availablePhotos}
          onAdd={addPhotos}
          onCancel={() => setShowAddPhotos(false)}
        />
      )}

      {/* Album photo grid */}
      {(isSelectionMode || secureAddTarget) && (
        <div className="flex items-center justify-between gap-3 mb-4 p-3 bg-accent-50 dark:bg-accent-900/30 rounded-lg">
          <div className="flex items-center gap-3">
            <button
              onClick={clearSelection}
              className="text-fg-muted hover:text-fg"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
            <span className="text-sm font-medium">
              {secureAddTarget ? `${selectedIds.size} selected to add to 🔒 ${secureAddTarget.galleryName}` : `${selectedIds.size} selected`}
            </span>
            <button
              onClick={selectAll}
              className="text-accent-600 dark:text-accent-400 text-sm hover:underline"
            >
              Select All
            </button>
          </div>
          {secureAddTarget ? (
            <button
              onClick={addSelectedToSecure}
              disabled={selectedIds.size === 0 || addingSecure}
              className="btn btn-primary btn-md inline-flex items-center gap-1.5"
              title={`Add to ${secureAddTarget.galleryName}`}
            >
              <span>🔒</span>
              {addingSecure ? "Adding…" : `Add to album (${selectedIds.size})`}
            </button>
          ) : (
            <button
              onClick={removeSelected}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium bg-orange-600 text-white hover:bg-orange-700 shadow-sm"
            >
              Remove ({selectedIds.size})
            </button>
          )}
        </div>
      )}
      {albumPhotos.length === 0 ? (
        <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
          <p className="text-fg-muted mb-2">This album is empty</p>
          <p className="text-fg-muted text-sm">
            Click "Add Photos" to add media from your gallery
          </p>
        </div>
      ) : (
        <JustifiedGrid
          items={albumPhotos}
          getAspectRatio={(p) => getEffectiveAspectRatio(p.width, p.height, p.cropData)}
          getKey={(p) => p.blobId}
          renderItem={(photo, idx) => (
            <AlbumTile
              photo={photo}
              isSelectionMode={isSelectionMode || !!secureAddTarget}
              isSelected={selectedIds.has(photo.blobId)}
              onClick={() => {
                if (isSelectionMode || secureAddTarget) {
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

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
