/**
 * Hook for uploading files from the Gallery page.
 *
 * Routes every selected file through `/api/photos/upload` so manual uploads
 * receive the same server-side processing as files registered by the
 * setup-time autoscan / ingest pipeline (EXIF/GPS extraction, server-side
 * format conversion of HEIC/MKV/etc., audio_backup_enabled enforcement,
 * AI/geo backfill, and ingest encryption).
 */
import { useCallback, useRef, useState } from "react";
import { api } from "../api/client";
import { mediaTypeFromMime } from "../db";
import { useProcessingStore } from "../store/processing";
import { guessMimeFromName } from "../utils/media";

export interface UploadDeps {
  loadEncryptedPhotos: () => Promise<void>;
  setError: (msg: string) => void;
}

// ── Hook ──────────────────────────────────────────────────────────────────────

/**
 * Handles file upload for the Gallery page. Files are streamed as raw bytes
 * to `/api/photos/upload`; the server stores, converts, extracts metadata,
 * and (using the stored encryption key) encrypts via the ingest pipeline.
 */
export function useGalleryUpload({ loadEncryptedPhotos, setError }: UploadDeps) {
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<{ done: number; total: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const folderInputRef = useRef<HTMLInputElement>(null);
  const { startTask, endTask } = useProcessingStore();

  const handleUpload = useCallback(async (files: FileList) => {
    setUploading(true);
    startTask("upload");
    setError("");

    // Camera RAW formats are not supported — explicitly block them first so
    // they are silently dropped even if the browser reports an image/ MIME type
    // (e.g. image/x-canon-cr2). Extension check takes priority over MIME type.
    const RAW_EXTENSIONS =
      /\.(cr2|cr3|nef|arw|dng|raf|orf|rw2|rw1|pef|sr2|srf|raw|3fr|erf|kdc|mef|mrw|nrw|ptx|r3d|rwl|srw|x3f)$/i;

    // Vector / unsupported image formats the server cannot decode (no SVG
    // feature in the `image` crate, and we deliberately don't ship librsvg).
    // These MUST be blocked explicitly because their MIME type is
    // `image/svg+xml`, which slips past the `startsWith("image/")` accept
    // check below and would otherwise upload → fail server-side → surface an
    // "Unsupported file format" banner on the gallery. We drop them silently
    // at the boundary instead (the server logs any that arrive anyway).
    const UNSUPPORTED_EXTENSIONS = /\.(svgz?|eps|ai)$/i;

    // Browser-native + convertible extensions accepted by the server
    // (`is_supported_extension` ∪ `is_convertible`). Drop other files at
    // the boundary so unrecognised formats are silently skipped rather
    // than producing a server 400 shown to the user.
    const ACCEPTED_EXTENSIONS =
      /\.(jpe?g|png|gif|webp|avif|bmp|ico|mp4|webm|mp3|flac|ogg|wav|heic|heif|tiff?|mkv|avi|mov|wmv|wma|m4a|aiff?|3gp)$/i;
    const fileArray = Array.from(files).filter(
      (f) =>
        !RAW_EXTENSIONS.test(f.name) &&
        !UNSUPPORTED_EXTENSIONS.test(f.name) &&
        (f.type.startsWith("image/") ||
          f.type.startsWith("video/") ||
          f.type.startsWith("audio/") ||
          ACCEPTED_EXTENSIONS.test(f.name)),
    );

    setUploadProgress({ done: 0, total: fileArray.length });

    let firstError: string | null = null;
    try {
      for (let i = 0; i < fileArray.length; i++) {
        const file = fileArray[i];
        setUploadProgress({ done: i, total: fileArray.length });
        try {
          await uploadSingleFile(file);
        } catch (err: unknown) {
          const msg = err instanceof Error ? err.message : "Upload failed";
          if (!firstError) firstError = `${file.name}: ${msg}`;
          // Continue with remaining files — one bad file shouldn't abort the
          // whole batch (e.g. a single audio rejected by the server toggle).
        }
      }
      setUploadProgress({ done: fileArray.length, total: fileArray.length });
      if (firstError) setError(firstError);
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
    const mimeType = file.type || guessMimeFromName(file.name);
    // mediaTypeFromMime is left in the import list because the server may
    // reject audio uploads; keep this call so future client-side filtering
    // (e.g. a UI hint when audio is disabled) has the value cached.
    void mediaTypeFromMime(mimeType);
    // Forward the File's lastModified so the server can use it as the
    // taken_at fallback when EXIF is missing — mirrors the autoscan
    // pipeline (which uses on-disk mtime) so manually uploaded files land
    // in the correct timeline slot instead of always "now".
    await api.photos.upload(arrayBuf, file.name, mimeType, {
      fileModifiedAt: file.lastModified,
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
