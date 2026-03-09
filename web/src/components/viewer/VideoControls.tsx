import { useState, useEffect, useRef, useCallback } from "react";

interface VideoControlsProps {
  videoRef: React.RefObject<HTMLVideoElement | null>;
  /** Controls visibility — tied to the viewer's overlay toggle */
  visible: boolean;
}

function formatTime(seconds: number): string {
  if (!isFinite(seconds) || seconds < 0) return "0:00";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0)
    return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/**
 * Custom video playback controls that render as a positioned overlay.
 *
 * Used instead of native `<video controls>` so that CSS rotation applied
 * to the video element doesn't rotate the UI controls along with it.
 * These controls sit outside the rotation wrapper.
 */
export default function VideoControls({ videoRef, visible }: VideoControlsProps) {
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [muted, setMuted] = useState(false);
  const seekBarRef = useRef<HTMLDivElement>(null);
  const [seeking, setSeeking] = useState(false);
  const rafRef = useRef(0);

  // ── Sync state with the <video> element ──────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onMeta = () => {
      if (video.duration && isFinite(video.duration)) setDuration(video.duration);
    };
    const onDur = () => {
      if (video.duration && isFinite(video.duration)) setDuration(video.duration);
    };
    const onVolumeChange = () => setMuted(video.muted);

    video.addEventListener("play", onPlay);
    video.addEventListener("pause", onPause);
    video.addEventListener("loadedmetadata", onMeta);
    video.addEventListener("durationchange", onDur);
    video.addEventListener("volumechange", onVolumeChange);

    // rAF-based time tracking for smooth seek bar updates
    let running = true;
    const tick = () => {
      if (!running) return;
      if (!seeking) setCurrentTime(video.currentTime);
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);

    // Sync initial state
    setPlaying(!video.paused);
    setMuted(video.muted);
    if (video.duration && isFinite(video.duration)) setDuration(video.duration);
    setCurrentTime(video.currentTime);

    return () => {
      running = false;
      cancelAnimationFrame(rafRef.current);
      video.removeEventListener("play", onPlay);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("loadedmetadata", onMeta);
      video.removeEventListener("durationchange", onDur);
      video.removeEventListener("volumechange", onVolumeChange);
    };
  }, [videoRef, seeking]);

  // ── Actions ──────────────────────────────────────────────────────────────
  const togglePlay = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) video.play();
    else video.pause();
  }, [videoRef]);

  const toggleMute = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
  }, [videoRef]);

  const handleSeek = useCallback(
    (clientX: number) => {
      const bar = seekBarRef.current;
      const video = videoRef.current;
      if (!bar || !video || !duration) return;
      const rect = bar.getBoundingClientRect();
      const fraction = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
      video.currentTime = fraction * duration;
      setCurrentTime(fraction * duration);
    },
    [videoRef, duration],
  );

  const progress = duration > 0 ? (currentTime / duration) * 100 : 0;

  return (
    <div
      className={`absolute bottom-0 left-0 right-0 z-30 transition-opacity duration-300 ${
        visible ? "opacity-100" : "opacity-0 pointer-events-none"
      }`}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="bg-gradient-to-t from-black/80 via-black/50 to-transparent pt-10 pb-3 px-4">
        {/* Seek bar */}
        <div
          ref={seekBarRef}
          className="w-full h-1.5 bg-white/20 rounded-full cursor-pointer mb-3 group relative"
          onPointerDown={(e) => {
            setSeeking(true);
            handleSeek(e.clientX);
            (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
          }}
          onPointerMove={(e) => {
            if (seeking) handleSeek(e.clientX);
          }}
          onPointerUp={() => setSeeking(false)}
        >
          <div
            className="h-full bg-blue-500 rounded-full transition-[width] duration-75"
            style={{ width: `${progress}%` }}
          />
          <div
            className="absolute top-1/2 -translate-y-1/2 w-3.5 h-3.5 bg-white rounded-full shadow-md opacity-0 group-hover:opacity-100 transition-opacity"
            style={{ left: `calc(${progress}% - 7px)` }}
          />
        </div>

        {/* Controls row */}
        <div className="flex items-center gap-3">
          {/* Play / Pause */}
          <button
            onClick={togglePlay}
            className="text-white hover:text-blue-400 transition-colors"
            aria-label={playing ? "Pause" : "Play"}
          >
            {playing ? (
              <svg className="w-7 h-7" fill="currentColor" viewBox="0 0 24 24">
                <rect x="6" y="4" width="4" height="16" rx="1" />
                <rect x="14" y="4" width="4" height="16" rx="1" />
              </svg>
            ) : (
              <svg className="w-7 h-7" fill="currentColor" viewBox="0 0 24 24">
                <path d="M8 5.14v13.72a1 1 0 0 0 1.5.86l11.04-6.86a1 1 0 0 0 0-1.72L9.5 4.28a1 1 0 0 0-1.5.86Z" />
              </svg>
            )}
          </button>

          {/* Time */}
          <span className="text-white/80 text-xs font-mono select-none min-w-[5.5rem]">
            {formatTime(currentTime)} / {formatTime(duration)}
          </span>

          <div className="flex-1" />

          {/* Mute / Unmute */}
          <button
            onClick={toggleMute}
            className="text-white/70 hover:text-white transition-colors"
            aria-label={muted ? "Unmute" : "Mute"}
          >
            {muted ? (
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M5.586 15H4a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1h1.586l4.707-4.707C10.923 3.663 12 4.109 12 5v14c0 .891-1.077 1.337-1.707.707L5.586 15Z"
                />
                <path strokeLinecap="round" strokeLinejoin="round" d="m17 9-4 6m4 0-4-6" />
              </svg>
            ) : (
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M19.114 5.636a9 9 0 0 1 0 12.728M16.463 8.288a5.25 5.25 0 0 1 0 7.424M5.586 15H4a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1h1.586l4.707-4.707C10.923 3.663 12 4.109 12 5v14c0 .891-1.077 1.337-1.707.707L5.586 15Z"
                />
              </svg>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
