/**
 * Hook that encapsulates viewer action handlers: delete, download,
 * save edits, save copy, remove from album, and toggle favorite.
 *
 * Keeps the Viewer component focused on rendering and state management.
 */
import { useState, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { db, type MediaType } from "../db";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import { applyEditsToImageDownload } from "../utils/media";
import type { CropMetadata, PreloadEntry } from "./useViewerMedia";

interface ViewerLocationState {
  photoIds?: string[];
  currentIndex?: number;
  albumId?: string;
}

interface UseViewerActionsParams {
  id: string | undefined;
  mediaUrl: string | null;
  filename: string;
  mediaType: MediaType;
  albumId: string | undefined;
  photoIds: string[] | undefined;
  currentIndex: number;
  cropCorners: { x: number; y: number; w: number; h: number };
  brightness: number;
  rotateValue: number;
  trimStart: number;
  trimEnd: number;
  mediaDuration: number;
  cropData: CropMetadata | null;
  mimeType: string;
  setCropData: (data: CropMetadata | null) => void;
  setCropCorners: (data: { x: number; y: number; w: number; h: number }) => void;
  setBrightness: (v: number) => void;
  setRotateValue: (v: number) => void;
  setTrimStart: (v: number) => void;
  setTrimEnd: (v: number) => void;
  setEditMode: (v: boolean) => void;
  setError: (msg: string) => void;
  preloadCache: React.MutableRefObject<Map<string, PreloadEntry>>;
}

export default function useViewerActions({
  id,
  mediaUrl,
  filename,
  mediaType,
  albumId,
  photoIds,
  currentIndex,
  cropCorners,
  brightness,
  rotateValue,
  trimStart,
  trimEnd,
  mediaDuration,
  cropData,
  mimeType,
  setCropData,
  setCropCorners,
  setBrightness,
  setRotateValue,
  setTrimStart,
  setTrimEnd,
  setEditMode,
  setError,
  preloadCache,
}: UseViewerActionsParams) {
  const navigate = useNavigate();

  const [showLeavePrompt, setShowLeavePrompt] = useState(false);
  const [saveCopySuccess, setSaveCopySuccess] = useState(false);

  /** Build the edit metadata object from current edit state */
  const buildEditMetadata = useCallback((): CropMetadata | null => {
    const c = cropCorners;
    const isDefaultCrop = c.x <= 0.01 && c.y <= 0.01 && c.w >= 0.99 && c.h >= 0.99;
    const isDefaultBrightness = Math.abs(brightness) < 1;
    const isDefaultRotate = rotateValue === 0;
    const isDefaultTrim = trimStart <= 0.01 && (mediaDuration <= 0 || Math.abs(trimEnd - mediaDuration) < 0.5);
    const allDefault = isDefaultCrop && isDefaultBrightness && isDefaultRotate && isDefaultTrim;
    if (allDefault) return null;
    return {
      x: Math.max(0, Math.min(1, c.x)),
      y: Math.max(0, Math.min(1, c.y)),
      width: Math.max(0.05, Math.min(1, c.w)),
      height: Math.max(0.05, Math.min(1, c.h)),
      rotate: rotateValue,
      brightness,
      ...((!isDefaultTrim) ? { trimStart, trimEnd } : {}),
    };
  }, [cropCorners, brightness, rotateValue, trimStart, trimEnd, mediaDuration]);

  const handleSaveEdit = useCallback(async () => {
    if (!id) return;
    const meta = buildEditMetadata();
    if (!meta) {
      // All defaults — clear metadata
      try {
        await db.photos.update(id, { cropData: undefined });
        setCropData(null);
      } catch { /* ignore */ }
    } else {
      try {
        await db.photos.update(id, { cropData: JSON.stringify(meta) });
        setCropData(meta);
      } catch { /* ignore */ }
    }
    setEditMode(false);
  }, [id, buildEditMetadata, setCropData, setEditMode]);

  const handleSaveCopy = useCallback(async () => {
    if (!id) return;
    const meta = buildEditMetadata();
    const metaJson = meta ? JSON.stringify(meta) : null;
    try {
      {
        // Duplicate the IndexedDB entry with its own ID + new metadata,
        // sync the copy to the server, and generate an edited thumbnail.
        const original = await db.photos.get(id);
        if (original) {
          const copyFilename = original.filename.startsWith("Copy of ")
            ? original.filename
            : `Copy of ${original.filename}`;

          // ── Server sync ────────────────────────────────────────────
          // If the original has a server-side photo record, call the
          // server's duplicate endpoint so the copy persists across
          // sessions and devices. The server row shares the same
          // encrypted_blob_id so no data is duplicated.
          let serverCopyId: string | undefined;
          if (original.serverPhotoId) {
            // No catch here — let errors propagate up to the outer catch so the
            // user sees the failure rather than silently getting a local-only copy.
            const res = await api.photos.duplicate(original.serverPhotoId, metaJson);
            serverCopyId = res.id;
          }

          // Re-use original thumbnail; the UI applies cropData via CSS

          const copyId = typeof crypto.randomUUID === "function"
            ? crypto.randomUUID()
            : Date.now().toString(36) + Math.random().toString(36).substring(2);
          // For server-side originals where the duplicate succeeded, keep
          // serverSide: true so MediaTile fetches the thumbnail via the API
          // endpoint (/api/photos/:id/thumbnail) using the copy's serverCopyId.
          // For encrypted originals or failed server duplicates, clear
          // serverSide so the stale-cleanup logic doesn't delete the copy
          // (which uses a random blobId, not a server photo ID).
          const copyShouldBeServerSide = !!(original.serverSide && serverCopyId);

          await db.photos.put({
            ...original,
            blobId: copyId,
            serverSide: copyShouldBeServerSide || undefined,
            // contentHash must not be duplicated — each row has a unique identity
            contentHash: undefined,
            storageBlobId: original.storageBlobId || original.blobId,
            filename: copyFilename,
            cropData: metaJson ?? undefined,
            takenAt: Date.now(),
            thumbnailData: original.thumbnailData,
            serverPhotoId: serverCopyId,
          });
          console.log("[Viewer:saveCopy] Copy saved to IDB:", {
            copyId, serverCopyId, copyShouldBeServerSide, filename: copyFilename,
            originalBlobId: id, originalServerSide: original.serverSide,
          });

          // Fire-and-forget: trigger backup sync so the copy propagates to all
          // configured backup servers without blocking the UI.
          if (serverCopyId) {
            const servers = useBackupStore.getState().backupServers;
            for (const srv of servers) {
              api.backup.triggerSync(srv.id).catch(() => { /* non-fatal */ });
            }
          }
        }
      }
      setEditMode(false);
      // Brief success flash — auto-clears after 2 seconds
      setSaveCopySuccess(true);
      setTimeout(() => setSaveCopySuccess(false), 2000);
    } catch (err) {
      console.error("[Viewer] Save Copy failed:", err);
      setError("Save Copy failed — please try again.");
    }
  }, [id, buildEditMetadata, setEditMode, setError]);

  const handleClearCrop = useCallback(async () => {
    if (!id) return;
    try {
      await db.photos.update(id, { cropData: undefined });
      setCropData(null);
      setCropCorners({ x: 0, y: 0, w: 1, h: 1 });
      setBrightness(0);
      setRotateValue(0);
      setTrimStart(0);
      setTrimEnd(mediaDuration);
    } catch { /* ignore */ }
  }, [id, setCropData, setCropCorners, setBrightness, setRotateValue, setTrimStart, setTrimEnd, mediaDuration]);

  const handleLeaveAndSave = useCallback(async () => {
    await handleSaveEdit();
    setShowLeavePrompt(false);
    navigate("/gallery");
  }, [handleSaveEdit, navigate]);

  const handleLeaveAndDiscard = useCallback(() => {
    setEditMode(false);
    setShowLeavePrompt(false);
    navigate("/gallery");
  }, [setEditMode, navigate]);

  const handleDelete = useCallback(async () => {
    const msg = "Move this item to trash? You can restore it within 30 days.";
    if (!id || !confirm(msg)) return;
    try {
      const cached = await db.photos.get(id);

      // Prefer the photos API when we have a server-side photo record ID.
      // This correctly handles: autoscanned photos (serverSide), encrypted-
      // synced photos (serverPhotoId from sync), and copies created via
      // the server duplicate endpoint (serverPhotoId from duplicate).
      if (cached?.serverPhotoId) {
        try {
          await api.photos.delete(cached.serverPhotoId);
        } catch (deleteErr) {
          // If the server record no longer exists (stale cache, local-only
          // copy whose server duplicate failed), fall through to local
          // cleanup instead of surfacing a confusing "Not found" error.
          const isNotFound =
            deleteErr instanceof Error && deleteErr.message === "Not found";
          if (!isNotFound) throw deleteErr;
        }
        await db.photos.delete(id);
        navigate("/gallery");
        return;
      }

      // Fallback: encrypted blob without a photos-table entry.
      // Use storageBlobId for copies that reference the original's blob.
      const blobId = cached?.storageBlobId || id;
      let trashResult: { trash_id: string; expires_at: string } | null = null;
      try {
        trashResult = await api.blobs.softDelete(blobId, {
          thumbnail_blob_id: cached?.thumbnailBlobId,
          filename: cached?.filename ?? "unknown",
          mime_type: cached?.mimeType ?? "application/octet-stream",
          media_type: cached?.mediaType,
          size_bytes: 0,
          width: cached?.width,
          height: cached?.height,
          duration_secs: cached?.duration,
          taken_at: cached?.takenAt
            ? new Date(cached.takenAt).toISOString()
            : undefined,
        });
      } catch (deleteErr) {
        // Blob may already be trashed or missing (e.g. local-only copy).
        // Clean up IndexedDB regardless so the item disappears from the UI.
        const isNotFound =
          deleteErr instanceof Error && deleteErr.message === "Not found";
        if (!isNotFound) throw deleteErr;
      }
      // Move to local trash table so we can show thumbnails in Trash view
      if (cached && trashResult) {
        await db.trash.put({
          trashId: trashResult.trash_id,
          blobId: id,
          thumbnailBlobId: cached.thumbnailBlobId,
          filename: cached.filename,
          mimeType: cached.mimeType,
          mediaType: cached.mediaType,
          width: cached.width,
          height: cached.height,
          takenAt: cached.takenAt,
          deletedAt: Date.now(),
          expiresAt: trashResult.expires_at,
          thumbnailData: cached.thumbnailData,
          duration: cached.duration,
          albumIds: cached.albumIds ?? [],
        });
      }
      await db.photos.delete(id);
      navigate("/gallery");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Delete failed");
    }
  }, [id, navigate, setError]);

  const handleRemoveFromAlbum = useCallback(async () => {
    if (!id || !albumId) return;
    try {
      const album = await db.albums.get(albumId);
      if (!album) return;
      const updated = album.photoBlobIds.filter((bid: string) => bid !== id);

      // Delete old manifest blob
      if (album.manifestBlobId) {
        try { await api.blobs.delete(album.manifestBlobId); } catch { /* ok */ }
      }

      // Upload new manifest
      const payload = JSON.stringify({
        v: 1,
        album_id: album.albumId,
        name: album.name,
        created_at: new Date(album.createdAt).toISOString(),
        cover_photo_blob_id: album.coverPhotoBlobId || null,
        photo_blob_ids: updated,
      });
      const { encrypt: enc, sha256Hex: sha } = await import("../crypto/crypto");
      const encrypted = await enc(new TextEncoder().encode(payload));
      const hash = await sha(new Uint8Array(encrypted));
      const res = await api.blobs.upload(encrypted, "album_manifest", hash);

      await db.albums.put({ ...album, photoBlobIds: updated, manifestBlobId: res.blob_id });

      // Navigate to next photo or back to album
      if (photoIds && photoIds.length > 1) {
        const remaining = photoIds.filter((pid) => pid !== id);
        const nextIdx = Math.min(currentIndex, remaining.length - 1);
        const nextId = remaining[nextIdx];
        navigate("/photo/" + nextId, { replace: true, state: { photoIds: remaining, currentIndex: nextIdx, albumId } });
      } else {
        navigate(`/album/${albumId}`);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Remove failed");
    }
  }, [id, albumId, photoIds, currentIndex, navigate, setError]);

  const handleDownload = useCallback(async () => {
    if (!mediaUrl) return;

    const isImage = mediaType === "photo" || mediaType === "gif";
    const isVideoOrAudio = mediaType === "video" || mediaType === "audio";

    // ── Images: bake edits via Canvas 2D (no server round-trip needed) ─────
    if (cropData && isImage) {
      try {
        const editedBlob = await applyEditsToImageDownload(mediaUrl, cropData, mimeType);
        const url = URL.createObjectURL(editedBlob);
        const a = document.createElement("a");
        a.href = url;
        const ext = mimeType.startsWith("image/png") ? ".png" : ".jpg";
        const base = (filename || "photo").replace(/\.[^.]+$/, "");
        a.download = base + ext;
        a.click();
        URL.revokeObjectURL(url);
        return;
      } catch (err) {
        setError(err instanceof Error ? err.message : "Download failed");
        return;
      }
    }

    // ── Video / Audio: send to server ffmpeg render endpoint ───────────────
    if (cropData && isVideoOrAudio) {
      // Look up the serverPhotoId — render endpoint is keyed by photos table ID
      const cached = id ? await db.photos.get(id) : undefined;
      const serverPhotoId = cached?.serverPhotoId ?? (cached?.serverSide ? id : undefined);
      if (serverPhotoId) {
        try {
          const cropJson = JSON.stringify(cropData);
          const blob = await api.photos.renderFile(serverPhotoId, cropJson);
          const url = URL.createObjectURL(blob);
          const a = document.createElement("a");
          a.href = url;
          a.download = `Edited ${filename || "media"}`;
          a.click();
          URL.revokeObjectURL(url);
          return;
        } catch (err) {
          setError(err instanceof Error ? err.message : "Server render failed");
          return;
        }
      }
      // No serverPhotoId (encrypted-mode video) — fall through to raw download
      // since we can't run ffmpeg client-side without WASM
    }

    // ── No edits, or encrypted video without serverPhotoId: raw file ───────
    const a = document.createElement("a");
    a.href = mediaUrl;
    a.download = filename || "media";
    a.click();
  }, [mediaUrl, cropData, mimeType, mediaType, filename, id, setError]);

  const handleDownloadOriginal = useCallback(async () => {
    if (!id) return;
    try {
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const res = await fetch(api.photos.fileUrl(id), { headers });
      if (!res.ok) throw new Error(`Download failed: ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename || "media";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("[Viewer] Download original failed:", err);
    }
  }, [id, filename]);


  const handleToggleFavorite = useCallback(async () => {
    if (!id) return;
    try {
      // The viewer id is the blobId — look up the server photo ID
      // from the IndexedDB cache (populated by encrypted-sync).
      const cached = await db.photos.get(id);
      if (!cached?.serverPhotoId) return; // No server mapping yet — can't toggle
      const photoId = cached.serverPhotoId;
      const res = await api.photos.toggleFavorite(photoId);
      // Persist the new favorite state in IndexedDB so it survives page reloads
      await db.photos.update(id, { isFavorite: res.is_favorite });
      return res.is_favorite;
    } catch {
      return undefined;
    }
  }, [id]);

  return {
    showLeavePrompt,
    setShowLeavePrompt,
    saveCopySuccess,
    buildEditMetadata,
    handleSaveEdit,
    handleSaveCopy,
    handleClearCrop,
    handleLeaveAndSave,
    handleLeaveAndDiscard,
    handleDelete,
    handleRemoveFromAlbum,
    handleDownload,
    handleDownloadOriginal,
    handleToggleFavorite,
  };
}
