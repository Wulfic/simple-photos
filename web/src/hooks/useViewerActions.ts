/**
 * Hook that encapsulates viewer action handlers: delete, download,
 * save edits, save copy, remove from album, and toggle favorite.
 *
 * Keeps the Viewer component focused on rendering and state management.
 */
import { useState, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { db, type MediaType } from "../db";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import { useProcessingStore } from "../store/processing";
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
  const [isRenderingVideo, setIsRenderingVideo] = useState(false);
  /** When true, show a download-choice dialog (converted vs original source) */
  const [showDownloadChoice, setShowDownloadChoice] = useState(false);
  // Holds the in-flight render promise so a second download click during
  // conversion waits for the same job rather than starting a duplicate.
  const renderInflightRef = useRef<Promise<Blob> | null>(null);

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
    const metaJson = meta ? JSON.stringify(meta) : null;
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
    // Keep fullPhotos cache in sync so the Viewer doesn't show stale edits
    // when re-opening the same photo (fullPhotos is the fast-path cache).
    try {
      const existing = await db.fullPhotos?.get(id);
      if (existing) {
        await db.fullPhotos?.update(id, { cropData: metaJson ?? undefined });
      }
    } catch { /* non-fatal */ }
    // Update preload cache so swiping back shows the edit immediately
    const cached = preloadCache.current.get(id);
    if (cached) {
      preloadCache.current.set(id, { ...cached, cropData: meta });
    }
    // Sync to server so Android and other clients see the edit
    try {
      const dbEntry = await db.photos.get(id);
      if (dbEntry?.serverPhotoId) {
        await api.photos.setCrop(dbEntry.serverPhotoId, metaJson);
      }
    } catch { /* non-fatal */ }
    setEditMode(false);
  }, [id, buildEditMetadata, setCropData, setEditMode, preloadCache]);

  const handleSaveCopy = useCallback(async () => {
    if (!id) return;
    const meta = buildEditMetadata();
    const metaJson = meta ? JSON.stringify(meta) : null;
    try {
      // Read original data before leaving edit mode
      const original = await db.photos.get(id);
      if (!original) {
        setError("Could not find photo data — try refreshing the page.");
        return;
      }

      const copyFilename = original.filename.startsWith("Copy of ")
        ? original.filename
        : `Copy of ${original.filename}`;

      const copyId = typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : (() => { const a = new Uint8Array(16); crypto.getRandomValues(a); return Array.from(a, b => b.toString(16).padStart(2, '0')).join(''); })();

      // ── Exit edit mode immediately — don't block the UI ──────────
      setEditMode(false);
      setSaveCopySuccess(true);
      setTimeout(() => setSaveCopySuccess(false), 2000);

      // ── Server sync runs in the background ───────────────────────
      // Show a "Saving copy" spinner in the nav bar while the server
      // renders the duplicate (ffmpeg for video can take 30+ seconds).
      if (original.serverPhotoId) {
        const { startTask, endTask } = useProcessingStore.getState();
        startTask("saveCopy");

        api.photos.duplicate(original.serverPhotoId, metaJson)
          .then(async (res) => {
            const serverCopyId = res.id;
            const copyShouldBeServerSide = !!(original.serverSide && serverCopyId);
            // Use the copy's own encrypted blob so the viewer fetches the
            // rendered file (with edits baked in) instead of the original.
            const copyBlobId = res.encrypted_blob_id ?? undefined;
            const copyThumbBlobId = res.encrypted_thumb_blob_id ?? undefined;

            await db.photos.put({
              ...original,
              blobId: copyId,
              serverSide: copyShouldBeServerSide || undefined,
              contentHash: undefined,
              storageBlobId: copyBlobId ?? (original.storageBlobId || original.blobId),
              thumbnailBlobId: copyThumbBlobId ?? original.thumbnailBlobId,
              filename: copyFilename,
              cropData: serverCopyId ? undefined : (metaJson ?? undefined),
              takenAt: original.takenAt,
              thumbnailData: original.thumbnailData,
              serverPhotoId: serverCopyId,
            });
            console.log("[Viewer:saveCopy] Copy saved to IDB:", {
              copyId, serverCopyId, copyShouldBeServerSide, filename: copyFilename,
              originalBlobId: id, originalServerSide: original.serverSide,
            });

            // Trigger backup sync for the new copy
            const servers = useBackupStore.getState().backupServers;
            for (const srv of servers) {
              api.backup.triggerSync(srv.id).catch(() => { /* non-fatal */ });
            }
          })
          .catch((err) => {
            console.error("[Viewer] Save Copy server sync failed:", err);
            // Create a local-only copy as fallback so the user doesn't lose work
            db.photos.put({
              ...original,
              blobId: copyId,
              serverSide: undefined,
              contentHash: undefined,
              storageBlobId: original.storageBlobId || original.blobId,
              filename: copyFilename,
              cropData: metaJson ?? undefined,
              takenAt: original.takenAt,
              thumbnailData: original.thumbnailData,
              serverPhotoId: undefined,
            }).catch(() => { /* last resort */ });
          })
          .finally(() => {
            endTask("saveCopy");
          });
      } else {
        // No server photo — create local-only copy immediately
        await db.photos.put({
          ...original,
          blobId: copyId,
          serverSide: undefined,
          contentHash: undefined,
          storageBlobId: original.storageBlobId || original.blobId,
          filename: copyFilename,
          cropData: metaJson ?? undefined,
          takenAt: original.takenAt,
          thumbnailData: original.thumbnailData,
          serverPhotoId: undefined,
        });
        console.log("[Viewer:saveCopy] Local-only copy saved to IDB:", {
          copyId, filename: copyFilename, originalBlobId: id,
        });
      }
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

      // Always use encrypted blob soft-delete (encrypted-only mode).
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

    // ── Converted file without edits: ask user which version to download ─
    if (!cropData) {
      const cached = id ? await db.photos.get(id) : undefined;
      if (cached?.sourcePath) {
        setShowDownloadChoice(true);
        return;
      }
    }

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
        // If a render is already in flight reuse the same promise — when it
        // resolves it will auto-download.  Just return so we don't queue another.
        if (renderInflightRef.current) return;

        const cropJson = JSON.stringify(cropData);
        const renderPromise = api.photos.renderFile(serverPhotoId, cropJson);
        renderInflightRef.current = renderPromise;
        setIsRenderingVideo(true);
        try {
          const blob = await renderPromise;
          const url = URL.createObjectURL(blob);
          const a = document.createElement("a");
          a.href = url;
          a.download = `Edited ${filename || "media"}`;
          a.click();
          URL.revokeObjectURL(url);
        } catch (err) {
          setError(err instanceof Error ? err.message : "Server render failed");
        } finally {
          renderInflightRef.current = null;
          setIsRenderingVideo(false);
        }
        return;
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

  /** Download the converted (browser-native) version — dismiss the choice dialog */
  const handleDownloadConverted = useCallback(() => {
    setShowDownloadChoice(false);
    if (!mediaUrl) return;
    const a = document.createElement("a");
    a.href = mediaUrl;
    a.download = filename || "media";
    a.click();
  }, [mediaUrl, filename]);

  /** Download the original unconverted source file from the server */
  const handleDownloadSource = useCallback(async () => {
    setShowDownloadChoice(false);
    if (!id) return;
    try {
      const cached = await db.photos.get(id);
      const serverPhotoId = cached?.serverPhotoId;
      if (!serverPhotoId) {
        setError("Server photo ID not available");
        return;
      }
      const { accessToken } = useAuthStore.getState();
      const headers: Record<string, string> = { "X-Requested-With": "SimplePhotos" };
      if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
      const res = await fetch(api.photos.sourceFileUrl(serverPhotoId), { headers });
      if (!res.ok) throw new Error(`Download failed: ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      // Extract original filename from sourcePath
      const sourceName = cached?.sourcePath?.split("/").pop() || filename || "original";
      a.download = sourceName;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Download source failed");
    }
  }, [id, filename, setError]);


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
    isRenderingVideo,
    showDownloadChoice,
    setShowDownloadChoice,
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
    handleDownloadConverted,
    handleDownloadSource,
    handleToggleFavorite,
  };
}
