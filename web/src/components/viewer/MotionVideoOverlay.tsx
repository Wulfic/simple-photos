/**
 * MotionVideoOverlay — auto-plays the embedded motion video for motion photos.
 *
 * Behaviour mirrors GIF autoplay: the motion video plays in a loop on top of
 * the still image. A toggle button lets the user switch between still and video.
 * The video auto-plays when the viewer opens the motion photo.
 */
import { useEffect, useState, useRef } from "react";
import { api } from "../../api/client";
import { useAuthStore } from "../../store/auth";

interface MotionVideoOverlayProps {
  /** Server-side photo ID (needed for the motion-video endpoint) */
  serverPhotoId: string;
  /** Whether the overlay should be visible */
  visible: boolean;
}

export default function MotionVideoOverlay({ serverPhotoId, visible }: MotionVideoOverlayProps) {
  const [videoUrl, setVideoUrl] = useState<string | null>(null);
  const [playing, setPlaying] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);

  // Fetch the motion video on mount
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(false);

    (async () => {
      try {
        const url = api.photos.motionVideoUrl(serverPhotoId);
        const token = useAuthStore.getState().accessToken;
        const resp = await fetch(url, {
          credentials: "include",
          headers: token ? { Authorization: `Bearer ${token}` } : {},
        });
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        const blob = await resp.blob();
        if (cancelled) return;
        const objUrl = URL.createObjectURL(blob);
        setVideoUrl(objUrl);
        setLoading(false);
      } catch {
        if (!cancelled) {
          setError(true);
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [serverPhotoId]);

  // Cleanup URL on unmount
  useEffect(() => {
    return () => {
      if (videoUrl) URL.revokeObjectURL(videoUrl);
    };
  }, [videoUrl]);

  // Auto-play/pause based on `playing` state
  useEffect(() => {
    const v = videoRef.current;
    if (!v || !videoUrl) return;
    if (playing) {
      v.play().catch(() => {});
    } else {
      v.pause();
    }
  }, [playing, videoUrl]);

  if (!visible || error) return null;

  return (
    <>
      {/* Video overlay — positioned on top of the still image */}
      {videoUrl && playing && (
        <video
          ref={videoRef}
          src={videoUrl}
          className="absolute inset-0 w-full h-full object-contain z-10"
          autoPlay
          loop
          muted
          playsInline
        />
      )}

      {/* Loading spinner */}
      {loading && (
        <div className="absolute top-16 left-1/2 -translate-x-1/2 z-20">
          <div className="w-5 h-5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
        </div>
      )}

      {/* LIVE toggle button */}
      {videoUrl && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            setPlaying((p) => !p);
          }}
          className={`absolute bottom-24 left-1/2 -translate-x-1/2 z-30 px-4 py-1.5 rounded-full text-sm font-bold transition-all ${
            playing
              ? "bg-white text-black shadow-lg"
              : "bg-black/60 text-white/80 hover:bg-black/80"
          }`}
        >
          {playing ? "LIVE ●" : "LIVE ○"}
        </button>
      )}
    </>
  );
}
