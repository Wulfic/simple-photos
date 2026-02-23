import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex } from "../crypto/crypto";
import { db, type CachedAlbum } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";

type SharedAlbumInfo = {
  id: string;
  name: string;
  owner_username: string;
  is_owner: boolean;
  photo_count: number;
  member_count: number;
  created_at: string;
};

type ShareUser = { id: string; username: string };

export default function Albums() {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
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

  const albums = useLiveQuery(() => db.albums.orderBy("name").toArray());

  useEffect(() => {
    loadAlbums();
    loadSharedAlbums();
  }, []);

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
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function createAlbum(e: React.FormEvent) {
    e.preventDefault();
    if (!newAlbumName.trim()) return;

    try {
      const albumId = crypto.randomUUID();
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
    } catch (err: any) {
      setError(err.message);
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
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function deleteSharedAlbum(albumId: string) {
    try {
      await api.sharing.deleteAlbum(albumId);
      setConfirmDeleteSharedId(null);
      await loadSharedAlbums();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function openSharePicker(albumId: string) {
    setSharePickerAlbumId(albumId);
    try {
      const users = await api.sharing.listUsers();
      setShareUsers(users);
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function addMemberToAlbum(userId: string) {
    if (!sharePickerAlbumId) return;
    try {
      await api.sharing.addMember(sharePickerAlbumId, userId);
      await loadSharedAlbums();
    } catch (err: any) {
      setError(err.message);
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader>
        <button
          onClick={() => setShowCreate(!showCreate)}
          className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3.5 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors shadow-sm shadow-blue-900/20"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
          New Album
        </button>
      </AppHeader>

      <main className="p-4">

      {showCreate && (
        <form onSubmit={createAlbum} className="mb-6 flex gap-2">
          <input
            type="text"
            value={newAlbumName}
            onChange={(e) => setNewAlbumName(e.target.value)}
            placeholder="Album name"
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
          <div
            key={album.albumId}
            onClick={() => navigate(`/albums/${album.albumId}`)}
            className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 cursor-pointer hover:shadow-md transition-shadow"
          >
            <div className="aspect-square bg-gray-100 dark:bg-gray-700 rounded mb-2 flex items-center justify-center">
              <span className="text-gray-400 text-3xl">
                {album.photoBlobIds.length}
              </span>
            </div>
            <p className="font-medium truncate">{album.name}</p>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              {album.photoBlobIds.length} items
            </p>
          </div>
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
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M19 7.5v3m0 0v3m0-3h3m-3 0h-3m-2.25-4.125a3.375 3.375 0 11-6.75 0 3.375 3.375 0 016.75 0zM4 19.235v-.11a6.375 6.375 0 0112.75 0v.109A12.318 12.318 0 0110.374 21c-2.331 0-4.512-.645-6.374-1.766z" />
                    </svg>
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
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
                    </svg>
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
      </main>
    </div>
  );
}
