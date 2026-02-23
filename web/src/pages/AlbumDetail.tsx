import { useEffect, useState, useMemo } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex } from "../crypto/crypto";
import { db, type CachedPhoto, type CachedAlbum } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";

type ShareUser = { id: string; username: string };

export default function AlbumDetail() {
  const { albumId } = useParams<{ albumId: string }>();
  const navigate = useNavigate();
  const [error, setError] = useState("");
  const [showAddPhotos, setShowAddPhotos] = useState(false);
  const [showSharePicker, setShowSharePicker] = useState(false);
  const [shareUsers, setShareUsers] = useState<ShareUser[]>([]);
  const [shareSuccess, setShareSuccess] = useState("");

  const album = useLiveQuery(
    () => (albumId ? db.albums.get(albumId) : undefined),
    [albumId]
  );

  const allPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  // Photos that belong to this album
  const albumPhotos = useMemo(() => {
    if (!album || !allPhotos) return [];
    const idSet = new Set(album.photoBlobIds);
    return allPhotos.filter((p) => idSet.has(p.blobId));
  }, [album, allPhotos]);

  // Photos NOT in this album (for "add photos" view)
  const availablePhotos = useMemo(() => {
    if (!album || !allPhotos) return [];
    const idSet = new Set(album.photoBlobIds);
    return allPhotos.filter((p) => !idSet.has(p.blobId));
  }, [album, allPhotos]);

  if (!albumId) {
    return <p className="p-4 text-red-600 dark:text-red-400">Invalid album ID</p>;
  }

  async function removePhoto(blobId: string) {
    if (!album) return;
    try {
      const updated = album.photoBlobIds.filter((id) => id !== blobId);
      await updateAlbumManifest({ ...album, photoBlobIds: updated });
    } catch (err: any) {
      setError(err.message);
    }
  }

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
    } catch (err: any) {
      setError(err.message);
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
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function openSharePicker() {
    setShowSharePicker(true);
    setShareSuccess("");
    try {
      const users = await api.sharing.listUsers();
      setShareUsers(users);
    } catch (err: any) {
      setError(err.message);
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
    } catch (err: any) {
      setError(err.message);
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
      <AppHeader>
        <button
          onClick={openSharePicker}
          className="inline-flex items-center gap-1.5 bg-green-600 text-white px-3.5 py-1.5 rounded-md hover:bg-green-500 text-sm font-medium transition-colors shadow-sm shadow-green-900/20"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M18 18.72a9.094 9.094 0 003.741-.479 3 3 0 00-4.682-2.72m.94 3.198l.001.031c0 .225-.012.447-.037.666A11.944 11.944 0 0112 21c-2.17 0-4.207-.576-5.963-1.584A6.062 6.062 0 016 18.719m12 0a5.971 5.971 0 00-.941-3.197m0 0A5.995 5.995 0 0012 12.75a5.995 5.995 0 00-5.058 2.772m0 0a3 3 0 00-4.681 2.72 8.986 8.986 0 003.74.477m.94-3.197a5.971 5.971 0 00-.94 3.197M15 6.75a3 3 0 11-6 0 3 3 0 016 0zm6 3a2.25 2.25 0 11-4.5 0 2.25 2.25 0 014.5 0zm-13.5 0a2.25 2.25 0 11-4.5 0 2.25 2.25 0 014.5 0z" />
          </svg>
          Share Album
        </button>
        <button
          onClick={() => setShowAddPhotos(!showAddPhotos)}
          className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3.5 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors shadow-sm shadow-blue-900/20"
        >
          {showAddPhotos ? "Done" : "Add Photos"}
        </button>
        <button
          onClick={deleteAlbum}
          className="inline-flex items-center gap-1.5 bg-red-600 text-white px-3.5 py-1.5 rounded-md hover:bg-red-500 text-sm font-medium transition-colors shadow-sm shadow-red-900/20"
        >
          Delete
        </button>
      </AppHeader>

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
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate("/albums")}
            className="text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 dark:text-gray-300 transition-colors"
            title="Back to Albums"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
            </svg>
          </button>
          <h2 className="text-xl font-semibold">{album.name}</h2>
          <span className="text-gray-400 text-sm">{album.photoBlobIds.length} items</span>
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
      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
        {albumPhotos.length === 0 && (
          <div className="col-span-full text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
            <p className="text-gray-500 dark:text-gray-400 mb-2">This album is empty</p>
            <p className="text-gray-400 text-sm">
              Click "Add Photos" to add media from your gallery
            </p>
          </div>
        )}

        {albumPhotos.map((photo) => (
          <AlbumTile
            key={photo.blobId}
            photo={photo}
            onClick={() => navigate(`/photo/${photo.blobId}`)}
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
  onClick: () => void;
  onRemove: () => void;
}

function AlbumTile({ photo, onClick, onRemove }: AlbumTileProps) {
  return (
    <div className="relative aspect-square bg-gray-100 dark:bg-gray-700 rounded overflow-hidden cursor-pointer group">
      <div onClick={onClick} className="w-full h-full">
        <ThumbnailImg photo={photo} />
      </div>

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

      {/* Remove button on hover */}
      <button
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
        className="absolute top-1 right-1 bg-red-600 text-white rounded-full w-6 h-6 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-xs"
        title="Remove from album"
      >
        ×
      </button>
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
