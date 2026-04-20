/**
 * Albums page — lists user albums (encrypted via IndexedDB manifests),
 * smart/default albums (Favorites, Photos, Videos, GIFs, Audio),
 * and shared albums (server-managed, multi-user).
 */
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex } from "../crypto/crypto";
import { db, type CachedAlbum, type CachedPhoto } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { getErrorMessage } from "../utils/formatters";
import { useIsBackupServer } from "../hooks/useIsBackupServer";
import type { FaceCluster } from "../api/ai";

type SharedAlbumInfo = {
  id: string;
  name: string;
  owner_username: string;
  is_owner: boolean;
  photo_count: number;
  member_count: number;
  created_at: string;
};

import type { ShareUser } from "../types/sharing";

export default function Albums() {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const isBackupServer = useIsBackupServer();
  const [showCreate, setShowCreate] = useState(false);
  const [newAlbumName, setNewAlbumName] = useState("");
  const navigate = useNavigate();

  // Shared albums state
  const [sharedAlbums, setSharedAlbums] = useState<SharedAlbumInfo[]>([]);
  const [sharedLoading, setSharedLoading] = useState(true);
  const [showCreateShared, setShowCreateShared] = useState(false);
  const [newSharedName, setNewSharedName] = useState("");
  const [shareUsers, setShareUsers] = useState<ShareUser[]>([]);
  const [sharePickerAlbumId, setSharePickerAlbumId] = useState<string | null>(null);
  const [confirmDeleteSharedId, setConfirmDeleteSharedId] = useState<string | null>(null);

  // People clusters
  const [peopleClusters, setPeopleClusters] = useState<FaceCluster[]>([]);
  const [peopleThumbUrls, setPeopleThumbUrls] = useState<Record<number, string>>({});

  // Memories
  const [memories, setMemories] = useState<Array<{
    id: string; name: string; city: string; country: string;
    date_label: string; photo_count: number;
    first_photo_id: string | null; first_thumb_path: string | null;
  }>>([]);
  const [memoryThumbUrls, setMemoryThumbUrls] = useState<Record<string, string>>({});

  // Encrypted photos from IndexedDB (for smart album counts)
  const encryptedPhotos = useLiveQuery(() => db.photos.toArray());

  const albums = useLiveQuery(() => db.albums.orderBy("name").toArray());

  useEffect(() => {
    loadAlbums();
    loadSharedAlbums();
    loadPeopleClusters();
    loadMemories();
  }, []);

  async function loadPeopleClusters() {
    try {
      const data = await api.ai.listFaceClusters();
      setPeopleClusters(data);
    } catch { /* AI may not be enabled */ }
  }

  async function loadMemories() {
    try {
      const data = await api.geo.listMemories();
      setMemories(data);
    } catch { /* Geo may not be enabled */ }
  }

  // Load thumbnails for people clusters
  useEffect(() => {
    if (peopleClusters.length === 0) return;
    let cancelled = false;
    (async () => {
      const urls: Record<number, string> = {};
      for (const c of peopleClusters) {
        if (!c.representative) continue;
        const photo = await db.photos.where("serverPhotoId").equals(c.representative).first();
        if (cancelled) return;
        if (photo?.thumbnailData) {
          const mime = photo.thumbnailMimeType || "image/jpeg";
          urls[c.id] = URL.createObjectURL(new Blob([photo.thumbnailData], { type: mime }));
        }
      }
      if (!cancelled) setPeopleThumbUrls(urls);
    })();
    return () => { cancelled = true; };
  }, [peopleClusters]);

  // Load thumbnails for memories
  useEffect(() => {
    if (memories.length === 0) return;
    let cancelled = false;
    (async () => {
      const urls: Record<string, string> = {};
      for (const m of memories) {
        if (!m.first_photo_id) continue;
        const photo = await db.photos.where("serverPhotoId").equals(m.first_photo_id).first();
        if (cancelled) return;
        if (photo?.thumbnailData) {
          const mime = photo.thumbnailMimeType || "image/jpeg";
          urls[m.id] = URL.createObjectURL(new Blob([photo.thumbnailData], { type: mime }));
        }
      }
      if (!cancelled) setMemoryThumbUrls(urls);
    })();
    return () => { cancelled = true; };
  }, [memories]);

  // Compute encrypted smart album counts + first thumbnails from IndexedDB
  const encryptedPhotoCounts = encryptedPhotos ? {
    all: encryptedPhotos.length,
    favorites: encryptedPhotos.filter(p => !!p.isFavorite).length,
    photos: encryptedPhotos.filter(p => p.mediaType === "photo" || p.mediaType === "gif").length,
    gifs: encryptedPhotos.filter(p => p.mediaType === "gif").length,
    videos: encryptedPhotos.filter(p => p.mediaType === "video").length,
    audio: encryptedPhotos.filter(p => p.mediaType === "audio").length,
  } : null;

  // Find the first photo with a thumbnail for each category.
  function findCoverPhoto(filter: (p: CachedPhoto) => boolean): CachedPhoto | undefined {
    if (!encryptedPhotos) return undefined;
    return encryptedPhotos.find(p => filter(p) && p.thumbnailData);
  }

  const smartAlbumCovers = {
    favorites: findCoverPhoto(p => !!p.isFavorite),
    photos: findCoverPhoto(p => p.mediaType === "photo" || p.mediaType === "gif"),
    gifs: findCoverPhoto(p => p.mediaType === "gif"),
    videos: findCoverPhoto(p => p.mediaType === "video"),
    audio: findCoverPhoto(p => p.mediaType === "audio"),
  };

  async function loadAlbums() {
    setLoading(true);
    try {
      const res = await api.blobs.list({ blob_type: "album_manifest" });
      const serverAlbumIds = new Set<string>();

      for (const blob of res.blobs) {
        try {
          const encrypted = await api.blobs.download(blob.id);
          const decrypted = await decrypt(encrypted);
          const payload = JSON.parse(new TextDecoder().decode(decrypted));

          serverAlbumIds.add(payload.album_id);
          await db.albums.put({
            albumId: payload.album_id,
            manifestBlobId: blob.id,
            name: payload.name,
            createdAt: new Date(payload.created_at).getTime(),
            coverPhotoBlobId: payload.cover_photo_blob_id,
            photoBlobIds: payload.photo_blob_ids || [],
          });
        } catch {
          // Skip albums we can't decrypt
        }
      }

      // Remove stale albums from IndexedDB that no longer exist on the server
      const localAlbums = await db.albums.toArray();
      const staleIds = localAlbums
        .map((a) => a.albumId)
        .filter((id) => !serverAlbumIds.has(id));
      if (staleIds.length > 0) {
        await db.albums.bulkDelete(staleIds);
      }
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }

  async function createAlbum(e: React.FormEvent) {
    e.preventDefault();
    if (!newAlbumName.trim()) return;

    try {
      // crypto.randomUUID() requires a secure context (HTTPS); fall back for HTTP
      const albumId = typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : ([1e7].toString() + -1e3 + -4e3 + -8e3 + -1e11).replace(/[018]/g, (c: string) =>
            (Number(c) ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (Number(c) / 4)))).toString(16)
          );
      const payload = JSON.stringify({
        v: 1,
        album_id: albumId,
        name: newAlbumName.trim(),
        created_at: new Date().toISOString(),
        cover_photo_blob_id: null,
        photo_blob_ids: [],
      });

      const encrypted = await encrypt(new TextEncoder().encode(payload));
      const hash = await sha256Hex(new Uint8Array(encrypted));
      const res = await api.blobs.upload(encrypted, "album_manifest", hash);

      await db.albums.put({
        albumId,
        manifestBlobId: res.blob_id,
        name: newAlbumName.trim(),
        createdAt: Date.now(),
        photoBlobIds: [],
      });

      setNewAlbumName("");
      setShowCreate(false);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  // ── Shared Album Handlers ────────────────────────────────────────────────

  async function loadSharedAlbums() {
    setSharedLoading(true);
    try {
      const list = await api.sharing.listAlbums();
      setSharedAlbums(list);
    } catch {
      // Sharing might not be available
    } finally {
      setSharedLoading(false);
    }
  }

  async function createSharedAlbum(e: React.FormEvent) {
    e.preventDefault();
    if (!newSharedName.trim()) return;
    try {
      await api.sharing.createAlbum(newSharedName.trim());
      setNewSharedName("");
      setShowCreateShared(false);
      await loadSharedAlbums();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function deleteSharedAlbum(albumId: string) {
    try {
      await api.sharing.deleteAlbum(albumId);
      setConfirmDeleteSharedId(null);
      await loadSharedAlbums();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function openSharePicker(albumId: string) {
    setSharePickerAlbumId(albumId);
    try {
      const users = await api.sharing.listUsers();
      setShareUsers(users);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function addMemberToAlbum(userId: string) {
    if (!sharePickerAlbumId) return;
    try {
      await api.sharing.addMember(sharePickerAlbumId, userId);
      await loadSharedAlbums();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="p-4">
        {/* ── User Albums ────────────────────────────────────────────────── */}
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-sm font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">Albums</h2>
          {!isBackupServer && (
          <button
            onClick={() => setShowCreate(!showCreate)}
            className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3.5 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors shadow-sm shadow-blue-900/20"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
            </svg>
            New Album
          </button>
          )}
        </div>

      {!isBackupServer && showCreate && (
        <form onSubmit={createAlbum} className="mb-6 flex gap-2">
          <input
            type="text"
            value={newAlbumName}
            onChange={(e) => setNewAlbumName(e.target.value)}
            placeholder="Album name"
            maxLength={200}
            className="flex-1 border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
            autoFocus
          />
          <button
            type="submit"
            className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm"
          >
            Create
          </button>
        </form>
      )}

      {error && <p className="text-red-600 dark:text-red-400 text-sm mb-4">{error}</p>}

      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4">
        {/* ── Smart albums pinned at top ────────────────────────────────── */}
        <SmartAlbumCard
          label="Favorites"
          count={encryptedPhotoCounts?.favorites ?? 0}
          coverPhoto={smartAlbumCovers.favorites}
          onClick={() => navigate("/albums/smart-favorites")}
        />
        <SmartAlbumCard
          label="Photos"
          count={encryptedPhotoCounts?.photos ?? 0}
          coverPhoto={smartAlbumCovers.photos}
          onClick={() => navigate("/albums/smart-photos")}
        />
        <SmartAlbumCard
          label="GIFs"
          count={encryptedPhotoCounts?.gifs ?? 0}
          coverPhoto={smartAlbumCovers.gifs}
          onClick={() => navigate("/albums/smart-gifs")}
        />
        <SmartAlbumCard
          label="Videos"
          count={encryptedPhotoCounts?.videos ?? 0}
          coverPhoto={smartAlbumCovers.videos}
          onClick={() => navigate("/albums/smart-videos")}
        />
        {(encryptedPhotoCounts?.audio ?? 0) > 0 && (
          <SmartAlbumCard
            label="Audio"
            count={encryptedPhotoCounts?.audio ?? 0}
            coverPhoto={smartAlbumCovers.audio}
            onClick={() => navigate("/albums/smart-audio")}
          />
        )}

        {/* ── User-created albums ───────────────────────────────────────── */}
        {loading && (!albums || albums.length === 0) && (
          <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-12">
            Loading albums...
          </p>
        )}
        {!loading && (!albums || albums.length === 0) && (
          <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-12">
            No albums yet. Create one to get started.
          </p>
        )}
        {albums?.map((album) => (
          <AlbumCard key={album.albumId} album={album} onClick={() => navigate(`/albums/${album.albumId}`)} />
        ))}
      </div>

      {/* ── Shared Albums ──────────────────────────────────────────────────── */}
      <div className="mt-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Shared Albums</h2>
        </div>

        {showCreateShared && (
          <form onSubmit={createSharedAlbum} className="mb-4 flex gap-2">
            <input
              type="text"
              value={newSharedName}
              onChange={(e) => setNewSharedName(e.target.value)}
              placeholder="Shared album name"
              maxLength={200}
              className="flex-1 border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-green-500"
              autoFocus
            />
            <button type="submit" className="bg-green-600 text-white px-4 py-2 rounded-md hover:bg-green-700 text-sm">
              Create
            </button>
          </form>
        )}

        {/* Share user picker modal */}
        {sharePickerAlbumId && (
          <div className="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4" onClick={() => setSharePickerAlbumId(null)}>
            <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-sm w-full p-6" onClick={(e) => e.stopPropagation()}>
              <h3 className="text-lg font-semibold mb-4">Share with User</h3>
              <div className="space-y-2 max-h-64 overflow-y-auto">
                {shareUsers.map((u) => (
                  <button
                    key={u.id}
                    onClick={() => { addMemberToAlbum(u.id); setSharePickerAlbumId(null); }}
                    className="w-full text-left px-3 py-2 rounded-md hover:bg-gray-100 dark:hover:bg-gray-700 text-sm flex items-center gap-2"
                  >
                    <svg className="w-5 h-5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
                    </svg>
                    {u.username}
                  </button>
                ))}
                {shareUsers.length === 0 && (
                  <p className="text-gray-500 text-sm text-center py-4">No users found</p>
                )}
              </div>
              <button
                onClick={() => setSharePickerAlbumId(null)}
                className="mt-4 w-full py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-md"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4">
          {sharedLoading && sharedAlbums.length === 0 && (
            <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-8">
              Loading shared albums...
            </p>
          )}
          {!sharedLoading && sharedAlbums.length === 0 && (
            <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-8">
              No shared albums yet. Create one to share photos with other users.
            </p>
          )}
          {sharedAlbums.map((sa) => (
            <div
              key={sa.id}
              className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 cursor-pointer hover:shadow-md transition-shadow relative group"
            >
              <div
                onClick={() => navigate(`/shared/${sa.id}`)}
                className="aspect-square bg-gradient-to-br from-green-50 to-blue-50 dark:from-green-900/20 dark:to-blue-900/20 rounded mb-2 flex flex-col items-center justify-center"
              >
                <span className="text-2xl font-semibold text-green-600 dark:text-green-400">{sa.photo_count}</span>
                <span className="text-xs text-gray-400 mt-0.5">{sa.member_count} member{sa.member_count !== 1 ? "s" : ""}</span>
              </div>
              <p className="font-medium truncate">{sa.name}</p>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                {sa.is_owner ? "You" : sa.owner_username}
              </p>

              {/* Actions (owner only) */}
              {sa.is_owner && (
                <div className="absolute top-2 right-2 hidden group-hover:flex gap-1">
                  <button
                    onClick={(e) => { e.stopPropagation(); openSharePicker(sa.id); }}
                    className="p-1 bg-white dark:bg-gray-700 rounded shadow text-green-600 hover:text-green-700"
                    title="Share"
                  >
                    <AppIcon name="shared" />
                  </button>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      if (confirmDeleteSharedId === sa.id) {
                        deleteSharedAlbum(sa.id);
                      } else {
                        setConfirmDeleteSharedId(sa.id);
                      }
                    }}
                    className={`p-1 bg-white dark:bg-gray-700 rounded shadow ${
                      confirmDeleteSharedId === sa.id
                        ? "text-red-700 bg-red-50 dark:bg-red-900/30"
                        : "text-red-500 hover:text-red-700"
                    }`}
                    title={confirmDeleteSharedId === sa.id ? "Click again to confirm" : "Delete"}
                  >
                    <AppIcon name="trashcan" />
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* ── People ─────────────────────────────────────────────────────────── */}
      {peopleClusters.length > 0 && (
        <div className="mt-8">
          <h2 className="text-sm font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider mb-3">People</h2>
          <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 gap-4">
            {peopleClusters.map((cluster) => (
              <div
                key={cluster.id}
                onClick={() => navigate(`/albums/smart-people/${cluster.id}`)}
                className="bg-white dark:bg-gray-800 rounded-lg shadow p-3 cursor-pointer hover:shadow-md transition-shadow"
              >
                <div className="aspect-square bg-gray-100 dark:bg-gray-700 rounded-full mb-2 mx-auto w-20 h-20 flex items-center justify-center overflow-hidden">
                  {peopleThumbUrls[cluster.id] ? (
                    <img
                      src={peopleThumbUrls[cluster.id]}
                      alt={cluster.label || "Unknown"}
                      className="w-full h-full object-cover rounded-full"
                    />
                  ) : (
                    <svg className="w-10 h-10 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
                    </svg>
                  )}
                </div>
                <p className="font-medium text-center text-sm truncate">
                  {cluster.label || "Unknown Person"}
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400 text-center">
                  {cluster.photo_count} photo{cluster.photo_count !== 1 ? "s" : ""}
                </p>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* ── Memories ───────────────────────────────────────────────────────── */}
      {memories.length > 0 && (
        <div className="mt-8">
          <h2 className="text-sm font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider mb-3">Memories</h2>
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4">
            {memories.map((memory) => (
              <div
                key={memory.id}
                onClick={() => navigate(`/albums/smart-memories/${memory.id}`)}
                className="bg-white dark:bg-gray-800 rounded-lg shadow cursor-pointer hover:shadow-md transition-shadow overflow-hidden"
              >
                <div className="aspect-video bg-gray-100 dark:bg-gray-700 flex items-center justify-center overflow-hidden">
                  {memoryThumbUrls[memory.id] ? (
                    <img
                      src={memoryThumbUrls[memory.id]}
                      alt={memory.name}
                      className="w-full h-full object-cover"
                    />
                  ) : (
                    <svg className="w-8 h-8 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15 10.5a3 3 0 11-6 0 3 3 0 016 0z" />
                      <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 10.5c0 7.142-7.5 11.25-7.5 11.25S4.5 17.642 4.5 10.5a7.5 7.5 0 1115 0z" />
                    </svg>
                  )}
                </div>
                <div className="p-3">
                  <p className="font-medium text-sm truncate">{memory.name}</p>
                  <p className="text-xs text-gray-500 dark:text-gray-400">
                    {memory.photo_count} photo{memory.photo_count !== 1 ? "s" : ""} · {memory.country}
                  </p>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      </main>
    </div>
  );
}

// ── Album Card with cover thumbnail ──────────────────────────────────────────

function AlbumCard({ album, onClick }: { album: CachedAlbum; onClick: () => void }) {
  const [thumbUrl, setThumbUrl] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (album.photoBlobIds.length === 0) return;

    // Use the first photo in the album as the cover
    const firstBlobId = album.photoBlobIds[0];

    (async () => {
      // Try to load thumbnail from local IndexedDB first (encrypted mode)
      const localPhoto = await db.photos.get(firstBlobId);
      if (cancelled) return;
      if (localPhoto?.thumbnailData) {
        const mime = localPhoto.thumbnailMimeType || (localPhoto.mediaType === "gif" ? "image/gif" : "image/jpeg");
        const blob = new Blob([localPhoto.thumbnailData], { type: mime });
        setThumbUrl(URL.createObjectURL(blob));
        return;
      }
    })();

    return () => { cancelled = true; };
  }, [album.photoBlobIds]);

  useEffect(() => {
    return () => {
      if (thumbUrl) URL.revokeObjectURL(thumbUrl);
    };
  }, [thumbUrl]);

  return (
    <div
      onClick={onClick}
      className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 cursor-pointer hover:shadow-md transition-shadow"
    >
      <div className="aspect-square bg-gray-100 dark:bg-gray-700 rounded mb-2 flex items-center justify-center overflow-hidden">
        {thumbUrl ? (
          <img src={thumbUrl} alt={album.name} className="w-full h-full object-cover" />
        ) : (
          <span className="text-gray-400 text-3xl">
            {album.photoBlobIds.length}
          </span>
        )}
      </div>
      <p className="font-medium truncate">{album.name}</p>
      <p className="text-sm text-gray-500 dark:text-gray-400">
        {album.photoBlobIds.length} items
      </p>
    </div>
  );
}

// ── Smart Album Card with cover thumbnail ────────────────────────────────────

function SmartAlbumCard({
  label,
  count,
  onClick,
  coverPhoto,
}: {
  label: string;
  count: number;
  onClick: () => void;
  /** The first matching CachedPhoto for this category (may have thumbnailData or serverPhotoId) */
  coverPhoto?: CachedPhoto;
}) {
  const [thumbUrl, setThumbUrl] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    if (coverPhoto?.thumbnailData) {
      // Encrypted thumbnail data already in IndexedDB
      const mime = coverPhoto.thumbnailMimeType || (coverPhoto.mediaType === "gif" ? "image/gif" : "image/jpeg");
      const blob = new Blob([coverPhoto.thumbnailData], { type: mime });
      const url = URL.createObjectURL(blob);
      if (!cancelled) setThumbUrl(url);
      return () => { cancelled = true; URL.revokeObjectURL(url); };
    }

    // No thumbnail source — reset
    if (!cancelled) setThumbUrl(null);
    return () => { cancelled = true; };
  }, [coverPhoto?.thumbnailData, coverPhoto?.thumbnailMimeType, coverPhoto?.mediaType]);

  // Revoke previous object URL when thumbUrl changes
  useEffect(() => {
    return () => {
      if (thumbUrl) URL.revokeObjectURL(thumbUrl);
    };
  }, [thumbUrl]);

  return (
    <div
      onClick={onClick}
      className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 cursor-pointer hover:shadow-md transition-shadow"
    >
      <div className="aspect-square bg-gray-100 dark:bg-gray-700 rounded mb-2 flex items-center justify-center overflow-hidden">
        {thumbUrl ? (
          <img src={thumbUrl} alt={label} className="w-full h-full object-cover" />
        ) : (
          <span className="text-gray-400 text-3xl">{count}</span>
        )}
      </div>
      <p className="font-medium truncate">{label}</p>
      <p className="text-sm text-gray-500 dark:text-gray-400">
        {count} items
      </p>
    </div>
  );
}
