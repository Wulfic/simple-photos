/**
 * AddToAlbumModal — modal that lists the user's local (encrypted) albums and
 * lets them pick one to add the currently-selected blob IDs to. Reuses the
 * same manifest re-upload pattern as `AlbumDetail.addPhotos`.
 */
import { useEffect, useRef, useState } from "react";
import { db, type CachedAlbum } from "../db";
import { encrypt, sha256Hex } from "../crypto/crypto";
import { api } from "../api/client";
import { randomUuid } from "../utils/uuid";
import { expandBurstSelection } from "../utils/burstExpand";
import { Modal } from "./ui";

interface AddToAlbumModalProps {
  blobIds: string[];
  onClose: () => void;
  onAdded: (album: CachedAlbum, addedCount: number) => void;
}

export default function AddToAlbumModal({ blobIds, onClose, onAdded }: AddToAlbumModalProps) {
  const [albums, setAlbums] = useState<CachedAlbum[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [creatingBusy, setCreatingBusy] = useState(false);
  const newNameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    db.albums.toArray().then((list) => {
      if (!cancelled) setAlbums(list.sort((a, b) => a.name.localeCompare(b.name)));
    });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (creating) newNameInputRef.current?.focus();
  }, [creating]);

  async function pickAlbum(album: CachedAlbum) {
    if (busyId) return;
    setBusyId(album.albumId);
    setError("");
    try {
      // Expand any collapsed burst representative to its full set of frames so
      // the whole stack lands in the album, not just the cover frame.
      const expanded = await expandBurstSelection(blobIds);
      const merged = [...new Set([...album.photoBlobIds, ...expanded])];
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

  async function createAndAdd(e: React.FormEvent) {
    e.preventDefault();
    const name = newName.trim();
    if (!name || creatingBusy || busyId) return;
    setCreatingBusy(true);
    setError("");
    try {
      const albumId = randomUuid();
      const createdAt = Date.now();
      // Expand collapsed burst representatives so the whole stack is added.
      const photoBlobIds = [...new Set(await expandBurstSelection(blobIds))];
      const coverPhotoBlobId = photoBlobIds[0] || undefined;

      const payload = JSON.stringify({
        v: 1,
        album_id: albumId,
        name,
        created_at: new Date(createdAt).toISOString(),
        cover_photo_blob_id: coverPhotoBlobId || null,
        photo_blob_ids: photoBlobIds,
      });
      const encrypted = await encrypt(new TextEncoder().encode(payload));
      const hash = await sha256Hex(new Uint8Array(encrypted));
      const res = await api.blobs.upload(encrypted, "album_manifest", hash);

      const stored: CachedAlbum = {
        albumId,
        manifestBlobId: res.blob_id,
        name,
        createdAt,
        coverPhotoBlobId,
        photoBlobIds,
      };
      await db.albums.put(stored);
      onAdded(stored, photoBlobIds.length);
    } catch (err: unknown) {
      console.error("[AddToAlbumModal] create-and-add failed", err);
      setError(err instanceof Error ? err.message : "Failed to create album");
    } finally {
      setCreatingBusy(false);
    }
  }

  return (
    <Modal
      onClose={onClose}
      size="md"
      zClassName="z-[60]"
      panelClassName="max-h-[80vh] flex flex-col"
      title={`Add ${blobIds.length} ${blobIds.length === 1 ? "item" : "items"} to album`}
    >
        {/* Create-new-album affordance — sticky at the top of the picker */}
        <div className="border-b border-edge">
          {creating ? (
            <form onSubmit={createAndAdd} className="flex items-center gap-2 px-4 py-3">
              <input
                ref={newNameInputRef}
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="New album name"
                disabled={creatingBusy}
                className="input flex-1 min-w-0 py-1.5 disabled:opacity-50"
              />
              <button
                type="submit"
                disabled={!newName.trim() || creatingBusy}
                className="btn btn-primary btn-md shrink-0"
              >
                {creatingBusy ? "Creating…" : "Create & add"}
              </button>
              <button
                type="button"
                onClick={() => { setCreating(false); setNewName(""); }}
                disabled={creatingBusy}
                className="btn btn-ghost btn-md shrink-0"
              >
                Cancel
              </button>
            </form>
          ) : (
            <button
              onClick={() => setCreating(true)}
              disabled={busyId !== null}
              className="w-full flex items-center gap-2 px-4 py-3 text-left text-sm font-medium text-accent-600 dark:text-accent-400 hover:bg-surface-sunken dark:hover:bg-white/10 transition-colors disabled:opacity-50"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
              </svg>
              New album
            </button>
          )}
        </div>

        <div className="flex-1 overflow-y-auto">
          {albums === null && (
            <p className="text-center text-sm text-fg-muted py-8">Loading albums…</p>
          )}
          {albums?.length === 0 && (
            <p className="text-center text-sm text-fg-muted py-8">
              No albums yet. Use “New album” above to create one.
            </p>
          )}
          {albums && albums.length > 0 && (
            <ul className="divide-y divide-edge">
              {albums.map((a) => (
                <li key={a.albumId}>
                  <button
                    onClick={() => pickAlbum(a)}
                    disabled={busyId !== null}
                    className="w-full text-left px-4 py-3 hover:bg-surface-sunken dark:hover:bg-white/10 transition-colors flex items-center justify-between disabled:opacity-50"
                  >
                    <span className="text-sm font-medium text-fg">{a.name}</span>
                    <span className="text-xs text-fg-muted">
                      {busyId === a.albumId ? "Adding…" : `${a.photoBlobIds.length} items`}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {error && (
          <p className="px-4 py-2 text-sm text-red-600 dark:text-red-400 border-t border-edge">
            {error}
          </p>
        )}
    </Modal>
  );
}
