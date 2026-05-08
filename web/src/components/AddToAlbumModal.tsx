/**
 * AddToAlbumModal — modal that lists the user's local (encrypted) albums and
 * lets them pick one to add the currently-selected blob IDs to. Reuses the
 * same manifest re-upload pattern as `AlbumDetail.addPhotos`.
 */
import { useEffect, useState } from "react";
import { db, type CachedAlbum } from "../db";
import { encrypt, sha256Hex } from "../crypto/crypto";
import { api } from "../api/client";

interface AddToAlbumModalProps {
  blobIds: string[];
  onClose: () => void;
  onAdded: (album: CachedAlbum, addedCount: number) => void;
}

export default function AddToAlbumModal({ blobIds, onClose, onAdded }: AddToAlbumModalProps) {
  const [albums, setAlbums] = useState<CachedAlbum[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    db.albums.toArray().then((list) => {
      if (!cancelled) setAlbums(list.sort((a, b) => a.name.localeCompare(b.name)));
    });
    return () => { cancelled = true; };
  }, []);

  async function pickAlbum(album: CachedAlbum) {
    if (busyId) return;
    setBusyId(album.albumId);
    setError("");
    try {
      const merged = [...new Set([...album.photoBlobIds, ...blobIds])];
      const addedCount = merged.length - album.photoBlobIds.length;
      const cover = album.coverPhotoBlobId || merged[0] || undefined;
      const updated: CachedAlbum = {
        ...album,
        photoBlobIds: merged,
        coverPhotoBlobId: cover,
      };

      // Delete old manifest (best-effort)
      if (updated.manifestBlobId) {
        try { await api.blobs.delete(updated.manifestBlobId); } catch { /* already gone */ }
      }

      const payload = JSON.stringify({
        v: 1,
        album_id: updated.albumId,
        name: updated.name,
        created_at: new Date(updated.createdAt).toISOString(),
        cover_photo_blob_id: updated.coverPhotoBlobId || null,
        photo_blob_ids: updated.photoBlobIds,
      });
      const encrypted = await encrypt(new TextEncoder().encode(payload));
      const hash = await sha256Hex(new Uint8Array(encrypted));
      const res = await api.blobs.upload(encrypted, "album_manifest", hash);

      const stored: CachedAlbum = { ...updated, manifestBlobId: res.blob_id };
      await db.albums.put(stored);
      onAdded(stored, addedCount);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to add to album");
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div
      className="fixed inset-0 z-[60] bg-black/60 flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-white dark:bg-gray-800 rounded-xl shadow-xl w-full max-w-md max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">
            Add {blobIds.length} {blobIds.length === 1 ? "item" : "items"} to album
          </h3>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-white"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="flex-1 overflow-y-auto">
          {albums === null && (
            <p className="text-center text-sm text-gray-500 dark:text-gray-400 py-8">Loading albums…</p>
          )}
          {albums?.length === 0 && (
            <p className="text-center text-sm text-gray-500 dark:text-gray-400 py-8">
              No albums yet. Create one from the Albums page first.
            </p>
          )}
          {albums && albums.length > 0 && (
            <ul className="divide-y divide-gray-100 dark:divide-gray-700">
              {albums.map((a) => (
                <li key={a.albumId}>
                  <button
                    onClick={() => pickAlbum(a)}
                    disabled={busyId !== null}
                    className="w-full text-left px-4 py-3 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors flex items-center justify-between disabled:opacity-50"
                  >
                    <span className="text-sm font-medium text-gray-800 dark:text-gray-100">{a.name}</span>
                    <span className="text-xs text-gray-500 dark:text-gray-400">
                      {busyId === a.albumId ? "Adding…" : `${a.photoBlobIds.length} items`}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {error && (
          <p className="px-4 py-2 text-sm text-red-600 dark:text-red-400 border-t border-gray-200 dark:border-gray-700">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
