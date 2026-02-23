import { useEffect, useRef, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { decrypt } from "../crypto/crypto";
import { db, type MediaType } from "../db";

// ── Payload shape ─────────────────────────────────────────────────────────────
interface MediaPayload {
  v: number;
  filename: string;
  taken_at: string;
  mime_type: string;
  media_type?: MediaType;
  width: number;
  height: number;
  duration?: number;
  album_ids: string[];
  thumbnail_blob_id: string;
  data: string; // base64-encoded raw file bytes
}

// ── Viewer ────────────────────────────────────────────────────────────────────

export default function Viewer() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  const [mediaUrl, setMediaUrl] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [mimeType, setMimeType] = useState("image/jpeg");
  const [mediaType, setMediaType] = useState<MediaType>("photo");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  // For live preview: show the cached thumbnail while the full media is loading
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  const videoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    if (!id) return;

    // Show cached thumbnail immediately for a live-preview feel
    db.photos.get(id).then((cached) => {
      if (cached?.thumbnailData) {
        const url = URL.createObjectURL(new Blob([cached.thumbnailData], { type: "image/jpeg" }));
        setPreviewUrl(url);
      }
    });

    loadMedia(id);

    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") navigate("/gallery");
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [id]);

  // Revoke object URLs on unmount to avoid memory leaks
  useEffect(() => {
    return () => {
      if (mediaUrl) URL.revokeObjectURL(mediaUrl);
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  }, [mediaUrl, previewUrl]);

  async function loadMedia(blobId: string) {
    setLoading(true);
    setError("");
    try {
      const encrypted = await api.blobs.download(blobId);
      const decrypted = await decrypt(encrypted);
      const payload: MediaPayload = JSON.parse(new TextDecoder().decode(decrypted));

      setFilename(payload.filename);
      setMimeType(payload.mime_type);

      // Derive media type from payload, then MIME, then default to photo
      const resolvedType: MediaType =
        payload.media_type ??
        (payload.mime_type === "image/gif"
          ? "gif"
          : payload.mime_type.startsWith("video/")
          ? "video"
          : "photo");
      setMediaType(resolvedType);

      // Decode base64 → Blob → Object URL
      const bytes = base64ToUint8Array(payload.data).buffer as ArrayBuffer;
      const blob = new Blob([bytes], { type: payload.mime_type });
      const url = URL.createObjectURL(blob);
      setMediaUrl(url);

      // Revoke the preview now that full media is ready
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
        setPreviewUrl(null);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to load media");
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete() {
    if (!id || !confirm("Permanently delete this item?")) return;
    try {
      await api.blobs.delete(id);
      const cached = await db.photos.get(id);
      if (cached?.thumbnailBlobId) {
        await api.blobs.delete(cached.thumbnailBlobId).catch(() => {});
      }
      await db.photos.delete(id);
      navigate("/gallery");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Delete failed");
    }
  }

  function handleDownload() {
    if (!mediaUrl) return;
    const a = document.createElement("a");
    a.href = mediaUrl;
    a.download = filename || "media";
    a.click();
  }

  // ── Render ────────────────────────────────────────────────────────────────────
  return (
    <div className="fixed inset-0 bg-black flex flex-col select-none">
      {/* Top bar */}
      <div className="flex items-center justify-between px-4 py-3 bg-black/80 z-10">
        <button
          onClick={() => navigate("/gallery")}
          className="text-white hover:text-gray-300 text-sm flex items-center gap-1"
        >
          ← Back
        </button>
        <span className="text-white text-sm truncate mx-4 max-w-xs">{filename}</span>
        <div className="flex gap-3">
          <button
            onClick={handleDownload}
            className="text-white hover:text-gray-300 text-sm"
            disabled={!mediaUrl}
          >
            Download
          </button>
          <button
            onClick={handleDelete}
            className="text-red-400 hover:text-red-300 text-sm"
          >
            Delete
          </button>
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 flex items-center justify-center overflow-hidden relative">
        {/* Live preview: blurred thumbnail shown while full media loads */}
        {previewUrl && loading && (
          <img
            src={previewUrl}
            alt="preview"
            className="absolute inset-0 w-full h-full object-contain blur-sm opacity-60 pointer-events-none"
          />
        )}

        {loading && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="text-white text-sm bg-black/50 px-4 py-2 rounded-full">
              Decrypting…
            </div>
          </div>
        )}

        {error && (
          <p className="text-red-400 text-sm z-10">{error}</p>
        )}

        {/* ── Photo / GIF viewer ─────────────────────────────────────────── */}
        {mediaUrl && (mediaType === "photo" || mediaType === "gif") && (
          <img
            src={mediaUrl}
            alt={filename}
            className="max-w-full max-h-full object-contain"
            style={{ imageRendering: mediaType === "gif" ? "auto" : undefined }}
          />
        )}

        {/* ── Video player ───────────────────────────────────────────────── */}
        {mediaUrl && mediaType === "video" && (
          <video
            ref={videoRef}
            src={mediaUrl}
            controls
            playsInline
            autoPlay={false}
            className="max-w-full max-h-full"
            style={{ background: "black" }}
          >
            <p className="text-white text-sm">
              Your browser doesn't support this video format.
            </p>
          </video>
        )}
      </div>

      {/* Bottom meta bar (shown when media is loaded) */}
      {mediaUrl && (
        <div className="px-4 py-2 bg-black/60 text-gray-400 text-xs flex items-center gap-4">
          <span className="uppercase tracking-wide font-mono">
            {mediaType === "video" ? "VIDEO" : mediaType === "gif" ? "GIF" : "PHOTO"}
          </span>
          <span className="truncate">{mimeType}</span>
        </div>
      )}
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function base64ToUint8Array(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
