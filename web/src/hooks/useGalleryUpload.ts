/**
 * Hook for uploading files from the Gallery page.
 *
 * Handles thumbnail generation, optional AES-256-GCM encryption (in encrypted
 * mode), SHA-256 dedup hashing, blob upload, and photo registration. Tracks
 * upload progress and errors for UI display.
 */
import { useCallback, useRef, useState } from "react";
import { api } from "../api/client";
import { encrypt, sha256Hex } from "../crypto/crypto";
import {
  blobTypeFromMime,
  mediaTypeFromMime,
} from "../db";
import { createFallbackThumbnail, createAudioFallbackThumbnail, arrayBufferToBase64 } from "../utils/media";
import { useProcessingStore } from "../store/processing";
import { generateThumbnail, getImageDimensions } from "../utils/gallery";
import type { EncryptionMode, ThumbnailPayload } from "./useGalleryData";

// ── Types ─────────────────────────────────────────────────────────────────────

interface PhotoPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type: "photo" | "gif" | "video" | "audio";
  width: number;
  height: number;
  duration?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64
}

export interface UploadDeps {
  mode: EncryptionMode | null;
  loadPlainPhotos: () => Promise<void>;
  loadEncryptedPhotos: () => Promise<void>;
  setError: (msg: string) => void;
}

// ── Hook ──────────────────────────────────────────────────────────────────────

/**
 * Handles file upload for the Gallery page.
 *
 * In plain mode, the selected files are ignored — instead, a server-side
 * scan registers files already placed in the storage directory.
 * In encrypted mode, each selected file is encrypted client-side and
 * uploaded through the blob API.
 */
export function useGalleryUpload({ mode, loadPlainPhotos, loadEncryptedPhotos, setError }: UploadDeps) {
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<{ done: number; total: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const { startTask, endTask } = useProcessingStore();

  const handleUpload = useCallback(async (files: FileList) => {
    if (mode === "plain") {
      // In plain mode, files must already be in the storage directory.
      // Trigger a server-side scan to register them.
      setUploading(true);
      startTask("upload");
      setError("");
      try {
        const res = await api.admin.scanAndRegister();
        if (res.registered > 0) {
          await loadPlainPhotos();
        } else {
          setError("No new files found. Place files in the storage directory first.");
        }
        // Trigger immediate conversion for any files needing web previews/thumbnails
        api.admin.triggerConvert().catch(() => {});
      } catch (err: unknown) {
        setError(err instanceof Error ? err.message : "Scan failed");
      } finally {
        setUploading(false);
        endTask("upload");
      }
      return;
    }

    // Encrypted mode: encrypt and upload
    setUploading(true);
    startTask("upload");
    setError("");

    const IMAGE_VIDEO_EXTENSIONS = /\.(jpe?g|png|gif|webp|heic|heif|avif|bmp|tiff?|dng|cr2|nef|arw|orf|rw2|ico|cur|hdr|svg|mp4|mov|avi|mkv|webm|m4v|3gp|wmv|asf|h264|mpg|mpeg|mp3|aiff|flac|ogg|wav|wma)$/i;
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
      // Trigger immediate conversion pass in case any uploads need web previews
      api.admin.triggerConvert().catch(() => {});
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setUploading(false);
      setUploadProgress(null);
      endTask("upload");
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps -- startTask, endTask,
  // setError are stable store/parent refs; loadPlainPhotos and loadEncryptedPhotos
  // are declared in the parent hook but depend only on stable state setters.
  }, [mode, loadPlainPhotos, loadEncryptedPhotos, startTask, endTask, setError]);

  async function uploadSingleFile(file: File) {
    const arrayBuf = await file.arrayBuffer();
    const data = new Uint8Array(arrayBuf);
    const mediaType = mediaTypeFromMime(file.type);
    const serverBlobType = blobTypeFromMime(file.type);

    // Generate thumbnail (JPEG frame for videos, scaled image for photos/GIFs)
    let thumbnailData: ArrayBuffer;
    if (mediaType === "audio") {
      thumbnailData = await createAudioFallbackThumbnail();
    } else {
      try {
        thumbnailData = await generateThumbnail(file, 256);
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
    const thumbPayload = JSON.stringify({
      v: 1,
      photo_blob_id: "", // filled after photo upload
      width: 256,
      height: 256,
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

  function handleDrop(e: React.DragEvent) {
    e.preventDefault();
    if (e.dataTransfer.files.length > 0) handleUpload(e.dataTransfer.files);
  }

  function handleFileInput(e: React.ChangeEvent<HTMLInputElement>) {
    if (e.target.files && e.target.files.length > 0) handleUpload(e.target.files);
    // Reset input so the same file can be re-selected
    if (inputRef.current) inputRef.current.value = "";
  }

  return {
    uploading,
    uploadProgress,
    inputRef,
    handleUpload,
    handleDrop,
    handleFileInput,
  };
}
