import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex } from "../crypto/crypto";
import { db, type CachedAlbum } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";

export default function Albums() {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [showCreate, setShowCreate] = useState(false);
  const [newAlbumName, setNewAlbumName] = useState("");
  const navigate = useNavigate();

  const albums = useLiveQuery(() => db.albums.orderBy("name").toArray());

  useEffect(() => {
    loadAlbums();
  }, []);

  async function loadAlbums() {
    setLoading(true);
    try {
      const res = await api.blobs.list({ blob_type: "album_manifest" });
      for (const blob of res.blobs) {
        try {
          const encrypted = await api.blobs.download(blob.id);
          const decrypted = await decrypt(encrypted);
          const payload = JSON.parse(new TextDecoder().decode(decrypted));

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
      </main>
    </div>
  );
}
