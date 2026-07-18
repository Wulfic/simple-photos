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
import { db, type CachedAlbum } from "../../db";
import { saveAlbumManifest } from "../../utils/albumManifest";
import { useAlbumPhotos } from "../../hooks/useAlbumPhotos";
import AppHeader from "../../components/AppHeader";
import AppIcon from "../../components/AppIcon";
import DetailHeader from "../../components/DetailHeader";
import AddPhotosPanel from "../../components/AddPhotosPanel";
import CastDialog, { CastIcon } from "../../components/CastDialog";
import { Modal } from "../../components/ui";
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
import { addPhotosToSecureGallery, secureAddResultMessage } from "../../utils/secureAdd";
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
  // Header overflow (⋮) menu + its dialogs (#35: Rename, Share, Cast, Delete
  // collapsed off the header; the standalone `+` handles Add Photos).
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const [showRename, setShowRename] = useState(false);
  const [renameInput, setRenameInput] = useState("");
  const [castOpen, setCastOpen] = useState(false);

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

  // Close the overflow menu on outside click or Escape (same pattern as the
  // AppHeader / viewer overflow menus).
  useEffect(() => {
    if (!menuOpen) return;
    function onClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuOpen(false);
    }
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  // Unified album resolution: membership, secure-exclusion and the count all
  // come from one source, so the header badge can no longer diverge from the
  // rendered grid (#12 missing counts, #20 count flicker). `album` is the
  // manifest used by the CRUD handlers below.
  const {
    photos: albumPhotos,
    album,
    allPhotos,
    secureBlobIds,
  } = useAlbumPhotos(albumId);

  // Preserve scroll position when opening a photo and returning to the album.
  const { pathname } = useLocation();

  useScrollMemory(pathname, albumPhotos.length > 0);

  // Photos NOT in this album (for "add photos" view), also excluding secure photos
  const availablePhotos = useMemo(() => {
    if (!album) return [];
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
      const result = await addPhotosToSecureGallery(secureAddTarget.galleryId, [...selectedIds]);
      const msg = secureAddResultMessage(result, secureAddTarget.galleryName);
      if (msg.success) toast.success(msg.success);
      if (msg.error) toast.error(msg.error);
      // Only leave the album / end the secure-add session once something moved;
      // if the whole batch failed, keep the selection so the user can retry.
      if (result.added > 0) {
        clearSelection();
        const target = secureAddTarget.galleryId;
        cancelSecureAdd();
        navigate(`/secure-gallery?album=${target}`);
      }
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
    await saveAlbumManifest(updatedAlbum);
  }

  async function deleteAlbum() {
    if (!album) return;
    if (!confirm(`Delete album "${album.name}"? Photos will not be deleted.`))
      return;
    try {
      // Tombstone FIRST for a Takeout-reconstructed album. Without this the
      // delete below is undone: the next reconstruction pass rebuilds the album
      // from the untouched server-side membership, on every device. Doing it
      // before the local delete means a failure here leaves the album intact
      // rather than deleting it locally and having it silently reappear.
      if (album.albumId.startsWith("src-")) {
        await api.photos.dismissSourceAlbum(album.albumId);
      }
      if (album.manifestBlobId) {
        await api.blobs.delete(album.manifestBlobId);
      }
      await db.albums.delete(album.albumId);
      navigate("/albums");
    } catch (err: unknown) {
      console.error("[RegularAlbumView] delete album failed", err);
      setError(getErrorMessage(err));
    }
  }

  function openRename() {
    if (!album) return;
    setRenameInput(album.name);
    setShowRename(true);
  }

  async function renameAlbum() {
    if (!album) return;
    const name = renameInput.trim();
    if (!name || name === album.name) {
      setShowRename(false);
      return;
    }
    try {
      await updateAlbumManifest({ ...album, name });
      setShowRename(false);
    } catch (err: unknown) {
      console.error("[RegularAlbumView] rename album failed", err);
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

      {/* Rename album modal (#35) */}
      {showRename && (
        <Modal open onClose={() => setShowRename(false)} title="Rename album" size="sm">
          <form
            onSubmit={(e) => { e.preventDefault(); renameAlbum(); }}
            className="p-4 flex flex-col gap-4"
          >
            <input
              type="text"
              value={renameInput}
              onChange={(e) => setRenameInput(e.target.value)}
              className="input w-full"
              placeholder="Album name"
              autoFocus
              maxLength={100}
            />
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setShowRename(false)}
                className="btn btn-ghost btn-md"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={!renameInput.trim()}
                className="btn btn-primary btn-md"
              >
                Save
              </button>
            </div>
          </form>
        </Modal>
      )}

      {/* Cast dialog (#35) — reuses the global browser-cast flow */}
      <CastDialog open={castOpen} onClose={() => setCastOpen(false)} />

      <main className="p-4">
        {/* Sub-header with album name. Per #35 the item-count text is gone and
            the header actions collapse into a `+` (Add Photos) and a ⋮ overflow
            menu (Rename · Share · Cast · Delete). */}
        <DetailHeader
          backTo="/albums"
          backTitle="Back to Albums"
          title={album.name}
          actions={!isBackupServer ? (
            <>
              <button
                onClick={() => setShowAddPhotos(!showAddPhotos)}
                title={showAddPhotos ? "Done adding" : "Add photos"}
                aria-label="Add photos"
                className={`inline-flex items-center justify-center w-9 h-9 rounded-md transition-all duration-200 shadow-sm ${
                  showAddPhotos
                    ? "bg-accent-600 text-white border border-accent-500 hover:bg-accent-700"
                    : "text-fg-muted bg-white dark:bg-white/10 border border-edge hover:bg-surface-sunken dark:hover:bg-white/20"
                }`}
              >
                {showAddPhotos ? (
                  <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                  </svg>
                ) : (
                  <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                  </svg>
                )}
              </button>

              <div className="relative" ref={menuRef}>
                <button
                  onClick={() => setMenuOpen((v) => !v)}
                  title="More options"
                  aria-label="More options"
                  aria-haspopup="menu"
                  aria-expanded={menuOpen}
                  className={`inline-flex items-center justify-center w-9 h-9 rounded-md transition-all duration-200 shadow-sm border border-edge ${
                    menuOpen
                      ? "bg-surface-sunken dark:bg-white/20 text-fg"
                      : "text-fg-muted bg-white dark:bg-white/10 hover:bg-surface-sunken dark:hover:bg-white/20"
                  }`}
                >
                  <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z" />
                  </svg>
                </button>
                {menuOpen && (
                  <div
                    className="absolute right-0 top-full mt-2 w-44 bg-surface rounded-lg shadow-2xl border border-edge py-1"
                    style={{ zIndex: 9999 }}
                    role="menu"
                  >
                    <button
                      onClick={() => { openRename(); setMenuOpen(false); }}
                      className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                      role="menuitem"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
                      </svg>
                      Rename
                    </button>
                    <button
                      onClick={() => { openSharePicker(); setMenuOpen(false); }}
                      className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                      role="menuitem"
                    >
                      <AppIcon name="shared" />
                      Share
                    </button>
                    <button
                      onClick={() => { setCastOpen(true); setMenuOpen(false); }}
                      className="w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors"
                      role="menuitem"
                    >
                      <CastIcon className="w-4 h-4 shrink-0" />
                      Cast
                    </button>
                    <button
                      onClick={() => { deleteAlbum(); setMenuOpen(false); }}
                      className="w-full text-left px-4 py-2 text-sm text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30 flex items-center gap-2 transition-colors"
                      role="menuitem"
                    >
                      <AppIcon name="trashcan" />
                      Delete
                    </button>
                  </div>
                )}
              </div>
            </>
          ) : undefined}
        >
          <SlideshowTriggers slideshow={slideshow} />
        </DetailHeader>

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
