import { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt, encrypt, sha256Hex, hasCryptoKey } from "../crypto/crypto";
import {
  db,
  type CachedPhoto,
  blobTypeFromMime,
  mediaTypeFromMime,
  ACCEPTED_MIME_TYPES,
} from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";

// ── Payload shapes ────────────────────────────────────────────────────────────

interface PhotoPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type: "photo" | "gif" | "video";
  width: number;
  height: number;
  duration?: number;
  latitude?: number;
  longitude?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64
}

interface ThumbnailPayload {
  v: number;
  photo_blob_id: string;
  width: number;
  height: number;
  data: string; // base64 JPEG
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Generate a JPEG thumbnail from any image or video file. */
async function generateThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  if (file.type.startsWith("video/")) {
    return generateVideoThumbnail(file, size);
  }
  return generateImageThumbnail(file, size);
}

function generateImageThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(file);
    img.onload = () => {
      URL.revokeObjectURL(url);
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d")!;
      // Cover-crop: fill the square
      const scale = Math.max(size / img.width, size / img.height);
      const w = img.width * scale;
      const h = img.height * scale;
      ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
      canvas.toBlob(
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        0.8
      );
    };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new Error("Image load failed")); };
    img.src = url;
  });
}

/** Seek to 10 % of video duration and capture a frame. */
function generateVideoThumbnail(file: File, size: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const video = document.createElement("video");
    video.muted = true;
    video.playsInline = true;
    const url = URL.createObjectURL(file);

    video.onloadedmetadata = () => {
      // Seek to 10 % of the video (at least 1 s in)
      video.currentTime = Math.min(Math.max(video.duration * 0.1, 1), video.duration);
    };

    video.onseeked = () => {
      URL.revokeObjectURL(url);
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d")!;
      const scale = Math.max(size / video.videoWidth, size / video.videoHeight);
      const w = video.videoWidth * scale;
      const h = video.videoHeight * scale;
      ctx.drawImage(video, (size - w) / 2, (size - h) / 2, w, h);
      canvas.toBlob(
        (blob) => (blob ? blob.arrayBuffer().then(resolve) : reject(new Error("Canvas toBlob failed"))),
        "image/jpeg",
        0.8
      );
    };

    video.onerror = () => { URL.revokeObjectURL(url); reject(new Error("Video load failed")); };
    video.src = url;
  });
}

function arrayBufferToBase64(buffer: ArrayBuffer | Uint8Array): string {
  const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

function base64ToArrayBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

/** Return a data URL to preview a thumbnail stored as ArrayBuffer. */
function thumbnailSrc(data: ArrayBuffer): string {
  return URL.createObjectURL(new Blob([data], { type: "image/jpeg" }));
}

/** Get the natural width/height of an image file. */
function getImageDimensions(file: File): Promise<{ width: number; height: number }> {
  return new Promise((resolve) => {
    if (file.type.startsWith("video/")) {
      const video = document.createElement("video");
      const url = URL.createObjectURL(file);
      video.onloadedmetadata = () => {
        URL.revokeObjectURL(url);
        resolve({ width: video.videoWidth, height: video.videoHeight });
      };
      video.onerror = () => { URL.revokeObjectURL(url); resolve({ width: 0, height: 0 }); };
      video.src = url;
    } else {
      const img = new Image();
      const url = URL.createObjectURL(file);
      img.onload = () => { URL.revokeObjectURL(url); resolve({ width: img.naturalWidth, height: img.naturalHeight }); };
      img.onerror = () => { URL.revokeObjectURL(url); resolve({ width: 0, height: 0 }); };
      img.src = url;
    }
  });
}

// ── Paginated blob fetching ───────────────────────────────────────────────────

/** Fetch all pages of a given blob type from the server. */
async function fetchAllPages(blobType: string) {
  const allBlobs: Array<{
    id: string;
    blob_type: string;
    size_bytes: number;
    client_hash: string | null;
    upload_time: string;
  }> = [];
  let cursor: string | undefined;
  do {
    const res = await api.blobs.list({
      blob_type: blobType,
      after: cursor,
      limit: 200,
    });
    allBlobs.push(...res.blobs);
    cursor = res.next_cursor ?? undefined;
  } while (cursor);
  return allBlobs;
}

// ── Component ─────────────────────────────────────────────────────────────────

export default function Gallery() {
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState("");
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement>(null);

  const photos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

  useEffect(() => {
    if (!hasCryptoKey()) {
      navigate("/setup");
      return;
    }
    loadPhotos();
  }, []);

  // ── Load / sync from server ──────────────────────────────────────────────────

  async function loadPhotos() {
    setLoading(true);
    try {
      // Fetch ALL blob types that represent media, with full pagination
      const allMedia = [
        ...(await fetchAllPages("photo")),
        ...(await fetchAllPages("gif")),
        ...(await fetchAllPages("video")),
      ];

      for (const blob of allMedia) {
        const existing = await db.photos.get(blob.id);
        if (existing) continue;

        try {
          const encrypted = await api.blobs.download(blob.id);
          const decrypted = await decrypt(encrypted);
          const payload: PhotoPayload = JSON.parse(new TextDecoder().decode(decrypted));

          let thumbnailData: ArrayBuffer | undefined;
          if (payload.thumbnail_blob_id) {
            try {
              const thumbEnc = await api.blobs.download(payload.thumbnail_blob_id);
              const thumbDec = await decrypt(thumbEnc);
              const thumbPayload: ThumbnailPayload = JSON.parse(new TextDecoder().decode(thumbDec));
              thumbnailData = base64ToArrayBuffer(thumbPayload.data);
            } catch {
              // Thumbnail fetch failed — show placeholder
            }
          }

          await db.photos.put({
            blobId: blob.id,
            thumbnailBlobId: payload.thumbnail_blob_id,
            filename: payload.filename,
            takenAt: new Date(payload.taken_at).getTime(),
            mimeType: payload.mime_type,
            mediaType: payload.media_type ?? mediaTypeFromMime(payload.mime_type),
            width: payload.width,
            height: payload.height,
            duration: payload.duration,
            latitude: payload.latitude,
            longitude: payload.longitude,
            albumIds: payload.album_ids ?? [],
            thumbnailData,
          });
        } catch {
          // Skip items we can't decrypt (wrong key or corrupt blob)
        }
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load");
    } finally {
      setLoading(false);
    }
  }

  // ── Upload ───────────────────────────────────────────────────────────────────

  const handleUpload = useCallback(async (files: FileList) => {
    setUploading(true);
    setError("");

    const fileArray = Array.from(files).filter(
      (f) => f.type.startsWith("image/") || f.type.startsWith("video/")
    );

    setUploadProgress({ done: 0, total: fileArray.length });

    try {
      for (let i = 0; i < fileArray.length; i++) {
        const file = fileArray[i];
        setUploadProgress({ done: i, total: fileArray.length });
        await uploadSingleFile(file);
      }
      setUploadProgress({ done: fileArray.length, total: fileArray.length });
      await loadPhotos();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Upload failed");
    } finally {
      setUploading(false);
      setUploadProgress(null);
    }
  }, []);

  async function uploadSingleFile(file: File) {
    const arrayBuf = await file.arrayBuffer();
    const data = new Uint8Array(arrayBuf);
    const mediaType = mediaTypeFromMime(file.type);
    const serverBlobType = blobTypeFromMime(file.type);

    // Generate thumbnail (JPEG frame for videos, scaled image for photos/GIFs)
    const thumbnailData = await generateThumbnail(file, 256);

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
    await api.blobs.upload(encPhoto, serverBlobType, photoHash);
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

  // ── Drag & Drop ───────────────────────────────────────────────────────────────

  function handleDrop(e: React.DragEvent) {
    e.preventDefault();
    if (e.dataTransfer.files.length > 0) handleUpload(e.dataTransfer.files);
  }

  function handleFileInput(e: React.ChangeEvent<HTMLInputElement>) {
    if (e.target.files && e.target.files.length > 0) handleUpload(e.target.files);
    // Reset input so the same file can be re-selected
    if (inputRef.current) inputRef.current.value = "";
  }

  // ── Render ────────────────────────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader>
        <label className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3.5 py-1.5 rounded-md hover:bg-blue-500 cursor-pointer text-sm font-medium transition-colors select-none shadow-sm shadow-blue-900/20">
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
          {uploading
            ? uploadProgress
              ? `${uploadProgress.done + 1}/${uploadProgress.total}`
              : "Uploading…"
            : "Upload"}
          <input
            ref={inputRef}
            type="file"
            multiple
            accept={ACCEPTED_MIME_TYPES}
            className="hidden"
            onChange={handleFileInput}
            disabled={uploading}
          />
        </label>
      </AppHeader>

      <main className="p-4">
        {error && <p className="text-red-600 dark:text-red-400 text-sm mb-4">{error}</p>}

        <div
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDrop}
          className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2"
        >
        {loading && (!photos || photos.length === 0) && (
          <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-12">Loading…</p>
        )}

        {!loading && (!photos || photos.length === 0) && (
          <div className="col-span-full text-center py-12 border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg">
            <p className="text-gray-500 dark:text-gray-400 mb-2">No media yet</p>
            <p className="text-gray-400 text-sm">
              Drag and drop photos, GIFs, or videos here — or click Upload
            </p>
          </div>
        )}

        {photos?.map((photo) => (
          <MediaTile
            key={photo.blobId}
            photo={photo}
            onClick={() => navigate(`/photo/${photo.blobId}`)}
          />
        ))}
      </div>
      </main>
    </div>
  );
}

// ── MediaTile ─────────────────────────────────────────────────────────────────

interface MediaTileProps {
  photo: CachedPhoto;
  onClick: () => void;
}

function MediaTile({ photo, onClick }: MediaTileProps) {
  const [src, setSrc] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const tileRef = useRef<HTMLDivElement>(null);

  // Lazy-load: only create the object URL when the tile is in the viewport
  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "200px" }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (visible && photo.thumbnailData) {
      const url = thumbnailSrc(photo.thumbnailData);
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    }
  }, [visible, photo.thumbnailData]);

  return (
    <div
      ref={tileRef}
      className="relative aspect-square bg-gray-100 dark:bg-gray-700 rounded overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group"
      onClick={onClick}
    >
      {src ? (
        <img src={src} alt={photo.filename} className="w-full h-full object-cover" loading="lazy" />
      ) : (
        <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center">
          {photo.filename}
        </div>
      )}

      {/* Media type badge */}
      {photo.mediaType === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {photo.duration ? <span>{formatDuration(photo.duration)}</span> : null}
        </div>
      )}
      {photo.mediaType === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}
    </div>
  );
}

function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}
