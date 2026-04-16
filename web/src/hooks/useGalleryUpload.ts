/**
 * Hook for uploading files from the Gallery page.
 *
 * Handles thumbnail generation, AES-256-GCM encryption, SHA-256 dedup
 * hashing, blob upload, and photo registration. Tracks upload progress
 * and errors for UI display.
 */
import { useCallback, useRef, useState } from "react";
import { api } from "../api/client";
import { encrypt, sha256Hex } from "../crypto/crypto";
import {
  blobTypeFromMime,
  mediaTypeFromMime,
} from "../db";
import { createFallbackThumbnail, createAudioFallbackThumbnail, arrayBufferToBase64 } from "../utils/media";
import type { ThumbnailPayload, PhotoPayload } from "../types/media";
import { useProcessingStore } from "../store/processing";
import { generateThumbnail } from "../gallery";
import { getImageDimensions } from "../utils/gallery";

export interface UploadDeps {
  loadEncryptedPhotos: () => Promise<void>;
  setError: (msg: string) => void;
}

// ── Hook ──────────────────────────────────────────────────────────────────────

/**
 * Handles file upload for the Gallery page.
 *
 * Each selected file is encrypted client-side (AES-256-GCM) and
 * uploaded through the blob API.
 */
export function useGalleryUpload({ loadEncryptedPhotos, setError }: UploadDeps) {
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<{ done: number; total: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const folderInputRef = useRef<HTMLInputElement>(null);
  const { startTask, endTask } = useProcessingStore();

  const handleUpload = useCallback(async (files: FileList) => {
    // Encrypt and upload
    setUploading(true);
    startTask("upload");
    setError("");

    const IMAGE_VIDEO_EXTENSIONS = /\.(jpe?g|png|gif|webp|avif|bmp|ico|svg|mp4|webm|mp3|flac|ogg|wav)$/i;
    const fileArray = Array.from(files).filter(
      (f) => f.type.startsWith("image/") || f.type.startsWith("video/") || f.type.startsWith("audio/") || IMAGE_VIDEO_EXTENSIONS.test(f.name)
    );

    setUploadProgress({ done: 0, total: fileArray.length });

    try {
      for (let i = 0; i < fileArray.length; i++) {
        const file = fileArray[i];
        setUploadProgress({ done: i, total: fileArray.length });
        await uploadSingleFile(file);
      }
      setUploadProgress({ done: fileArray.length, total: fileArray.length });
      await loadEncryptedPhotos();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setUploading(false);
      setUploadProgress(null);
      endTask("upload");
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps -- startTask, endTask,
  // setError are stable store/parent refs; loadEncryptedPhotos is declared in
  // the parent hook but depends only on stable state setters.
  }, [loadEncryptedPhotos, startTask, endTask, setError]);

  async function uploadSingleFile(file: File) {
    const arrayBuf = await file.arrayBuffer();
    const data = new Uint8Array(arrayBuf);
    const mediaType = mediaTypeFromMime(file.type);
    const serverBlobType = blobTypeFromMime(file.type);

    // Generate thumbnail (JPEG frame for videos, scaled image for photos, animated GIF for GIFs)
    let thumbnailData: ArrayBuffer;
    let thumbnailMimeType = "image/jpeg";
    if (mediaType === "audio") {
      thumbnailData = await createAudioFallbackThumbnail();
    } else {
      try {
        const thumbResult = await generateThumbnail(file, { size: 512 });
        thumbnailData = thumbResult.data;
        thumbnailMimeType = thumbResult.mimeType;
      } catch {
        console.warn(`Thumbnail generation failed for ${file.name}, using fallback`);
        thumbnailData = await createFallbackThumbnail();
      }
    }

    // Get actual dimensions
    const dims = await getImageDimensions(file);

    // Get video duration if applicable
    let duration: number | undefined;
    if (mediaType === "video") {
      duration = await getVideoDuration(file);
    }

    // ── Thumbnail blob ───────────────────────────────────────────────────────
    // Decode thumbnail to get actual dimensions (thumbnails are now
    // aspect-ratio-preserving, not always 256×256).
    const thumbDims = await new Promise<{ w: number; h: number }>((resolve) => {
      const img = new Image();
      const url = URL.createObjectURL(new Blob([thumbnailData], { type: thumbnailMimeType }));
      img.onload = () => { URL.revokeObjectURL(url); resolve({ w: img.naturalWidth, h: img.naturalHeight }); };
      img.onerror = () => { URL.revokeObjectURL(url); resolve({ w: 256, h: 256 }); };
      img.src = url;
    });
    const thumbPayload = JSON.stringify({
      v: 1,
      photo_blob_id: "", // filled after photo upload
      width: thumbDims.w,
      height: thumbDims.h,
      mime_type: thumbnailMimeType,
      data: arrayBufferToBase64(thumbnailData),
    } satisfies Partial<ThumbnailPayload>);

    const encThumb = await encrypt(new TextEncoder().encode(thumbPayload));
    const thumbHash = await sha256Hex(new Uint8Array(encThumb));
    // Use video_thumbnail type for video poster frames
    const thumbBlobType = mediaType === "video" ? "video_thumbnail" : "thumbnail";
    const thumbRes = await api.blobs.upload(encThumb, thumbBlobType, thumbHash);

    // ── Media blob ────────────────────────────────────────────────────────────
    const photoPayload = JSON.stringify({
      v: 1,
      filename: file.name,
      taken_at: new Date().toISOString(),
      mime_type: file.type,
      media_type: mediaType,
      width: dims.width,
      height: dims.height,
      duration,
      album_ids: [],
      thumbnail_blob_id: thumbRes.blob_id,
      data: arrayBufferToBase64(data),
    } satisfies Partial<PhotoPayload>);

    const encPhoto = await encrypt(new TextEncoder().encode(photoPayload));
    const photoHash = await sha256Hex(new Uint8Array(encPhoto));
    // Content hash: short hash of original raw bytes for cross-platform alignment
    const contentHash = (await sha256Hex(new Uint8Array(data))).substring(0, 12);
    await api.blobs.upload(encPhoto, serverBlobType, photoHash, contentHash);
  }

  function getVideoDuration(file: File): Promise<number> {
    return new Promise((resolve) => {
      const video = document.createElement("video");
      const url = URL.createObjectURL(file);
      video.onloadedmetadata = () => { URL.revokeObjectURL(url); resolve(video.duration); };
      video.onerror = () => { URL.revokeObjectURL(url); resolve(0); };
      video.src = url;
    });
  }

  /** Recursively collect all File objects from a DataTransferItem entry. */
  function collectFilesFromEntry(entry: FileSystemEntry): Promise<File[]> {
    return new Promise((resolve) => {
      if (entry.isFile) {
        (entry as FileSystemFileEntry).file(
          (f) => resolve([f]),
          () => resolve([]),
        );
      } else if (entry.isDirectory) {
        const reader = (entry as FileSystemDirectoryEntry).createReader();
        const allFiles: File[] = [];
        const readBatch = () => {
          reader.readEntries(async (entries) => {
            if (entries.length === 0) {
              resolve(allFiles);
              return;
            }
            for (const child of entries) {
              const files = await collectFilesFromEntry(child);
              allFiles.push(...files);
            }
            // readEntries may not return all in one call — keep reading
            readBatch();
          }, () => resolve(allFiles));
        };
        readBatch();
      } else {
        resolve([]);
      }
    });
  }

  async function handleDrop(e: React.DragEvent) {
    e.preventDefault();
    // Use webkitGetAsEntry to support folder drops
    if (e.dataTransfer.items) {
      const allFiles: File[] = [];
      const entries: FileSystemEntry[] = [];
      for (let i = 0; i < e.dataTransfer.items.length; i++) {
        const entry = e.dataTransfer.items[i].webkitGetAsEntry?.();
        if (entry) entries.push(entry);
      }
      for (const entry of entries) {
        const files = await collectFilesFromEntry(entry);
        allFiles.push(...files);
      }
      if (allFiles.length > 0) {
        // Create a synthetic FileList-like structure
        const dt = new DataTransfer();
        for (const f of allFiles) dt.items.add(f);
        handleUpload(dt.files);
      }
    } else if (e.dataTransfer.files.length > 0) {
      handleUpload(e.dataTransfer.files);
    }
  }

  function handleFileInput(e: React.ChangeEvent<HTMLInputElement>) {
    if (e.target.files && e.target.files.length > 0) handleUpload(e.target.files);
    // Reset input so the same file can be re-selected
    if (inputRef.current) inputRef.current.value = "";
  }

  function handleFolderInput(e: React.ChangeEvent<HTMLInputElement>) {
    if (e.target.files && e.target.files.length > 0) handleUpload(e.target.files);
    if (folderInputRef.current) folderInputRef.current.value = "";
  }

  return {
    uploading,
    uploadProgress,
    inputRef,
    folderInputRef,
    handleUpload,
    handleDrop,
    handleFileInput,
    handleFolderInput,
  };
}
