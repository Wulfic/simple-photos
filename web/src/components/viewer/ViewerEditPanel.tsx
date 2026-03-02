import { useRef, useEffect, useCallback } from "react";
import type { MediaType } from "../../db";

// ── Types ────────────────────────────────────────────────────────────────────

/** Which editing tab is active */
export type EditTab = "crop" | "brightness" | "trim";

interface ViewerEditPanelProps {
  /** Which tab is currently active */
  editTab: EditTab;
  setEditTab: (tab: EditTab) => void;

  /** Current media type being edited */
  mediaType: MediaType;

  /** Brightness value (-100 to 100) */
  brightness: number;
  setBrightness: (v: number) => void;

  /** Existing crop/edit data (used to show Reset button) */
  cropData: {
    x: number;
    y: number;
    width: number;
    height: number;
    rotate: number;
    brightness?: number;
    trimStart?: number;
    trimEnd?: number;
  } | null;

  /** Trim start/end in seconds */
  trimStart: number;
  trimEnd: number;
  setTrimStart: (v: number) => void;
  setTrimEnd: (v: number) => void;

  /** Total duration of the media in seconds (for trim slider bounds) */
  duration: number;

  /** Save overwrites the current metadata */
  onSave: () => void;
  /** Save Copy creates a new metadata-only version */
  onSaveCopy: () => void;
  /** Reset clears all edits */
  onClear: () => void;
  /** Cancel exits edit mode without saving */
  onCancel: () => void;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Format seconds as MM:SS or HH:MM:SS */
function formatTime(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  return `${m}:${String(sec).padStart(2, "0")}`;
}

// ── Component ────────────────────────────────────────────────────────────────

export default function ViewerEditPanel({
  editTab,
  setEditTab,
  mediaType,
  brightness,
  setBrightness,
  cropData,
  trimStart,
  trimEnd,
  setTrimStart,
  setTrimEnd,
  duration,
  onSave,
  onSaveCopy,
  onClear,
  onCancel,
}: ViewerEditPanelProps) {
  // Determine which tabs are available for this media type
  const isPhoto = mediaType === "photo";
  const isVideo = mediaType === "video";
  const isAudio = mediaType === "audio";
  const showCrop = isPhoto || isVideo;
  const showBrightness = isPhoto || isVideo;
  const showTrim = isVideo || isAudio;

  // ── Trim range slider refs for dual-thumb control ──────────────────────
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingThumb = useRef<"start" | "end" | null>(null);

  // Clamp trim values to valid range
  const clampTrimStart = useCallback(
    (v: number) => Math.max(0, Math.min(v, trimEnd - 0.5)),
    [trimEnd],
  );
  const clampTrimEnd = useCallback(
    (v: number) => Math.max(trimStart + 0.5, Math.min(v, duration)),
    [trimStart, duration],
  );

  // Pointer handlers for dual-thumb trim slider
  const handleTrackPointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (!trackRef.current || duration <= 0) return;
      const rect = trackRef.current.getBoundingClientRect();
      const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
      const secs = pct * duration;
      // Determine which thumb is closer
      const distToStart = Math.abs(secs - trimStart);
      const distToEnd = Math.abs(secs - trimEnd);
      if (distToStart <= distToEnd) {
        draggingThumb.current = "start";
        setTrimStart(clampTrimStart(secs));
      } else {
        draggingThumb.current = "end";
        setTrimEnd(clampTrimEnd(secs));
      }
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    },
    [duration, trimStart, trimEnd, setTrimStart, setTrimEnd, clampTrimStart, clampTrimEnd],
  );

  const handleTrackPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!draggingThumb.current || !trackRef.current || duration <= 0) return;
      const rect = trackRef.current.getBoundingClientRect();
      const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
      const secs = pct * duration;
      if (draggingThumb.current === "start") {
        setTrimStart(clampTrimStart(secs));
      } else {
        setTrimEnd(clampTrimEnd(secs));
      }
    },
    [duration, setTrimStart, setTrimEnd, clampTrimStart, clampTrimEnd],
  );

  const handleTrackPointerUp = useCallback(() => {
    draggingThumb.current = null;
  }, []);

  // Auto-select first available tab when entering edit for audio
  useEffect(() => {
    if (isAudio && editTab !== "trim") {
      setEditTab("trim");
    }
  }, [isAudio, editTab, setEditTab]);

  return (
    <div className="absolute bottom-0 left-0 right-0 z-30 bg-black/90 border-t border-white/10 px-4 py-3 space-y-3">
      {/* Tab switcher */}
      <div className="flex items-center justify-center gap-2">
        {showCrop && (
          <button
            onClick={() => setEditTab("crop")}
            className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
              editTab === "crop"
                ? "bg-white text-black"
                : "bg-white/10 text-white hover:bg-white/20"
            }`}
          >
            Crop
          </button>
        )}
        {showBrightness && (
          <button
            onClick={() => setEditTab("brightness")}
            className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
              editTab === "brightness"
                ? "bg-white text-black"
                : "bg-white/10 text-white hover:bg-white/20"
            }`}
          >
            Brightness
          </button>
        )}
        {showTrim && (
          <button
            onClick={() => setEditTab("trim")}
            className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
              editTab === "trim"
                ? "bg-white text-black"
                : "bg-white/10 text-white hover:bg-white/20"
            }`}
          >
            Trim
          </button>
        )}
      </div>

      {/* ── Brightness slider ────────────────────────────────────────── */}
      {editTab === "brightness" && (
        <div className="flex items-center gap-3 max-w-sm mx-auto">
          <svg className="w-5 h-5 text-gray-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <circle cx="12" cy="12" r="4" />
          </svg>
          <input
            type="range"
            min={-100}
            max={100}
            value={brightness}
            onChange={(e) => setBrightness(Number(e.target.value))}
            className="flex-1 h-1.5 rounded-full appearance-none bg-white/20 accent-white cursor-pointer"
          />
          <svg className="w-5 h-5 text-yellow-300 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <circle cx="12" cy="12" r="4" />
            <path strokeLinecap="round" d="M12 2v2m0 16v2m-7.07-3.93l1.41-1.41m9.9-9.9l1.41-1.41M2 12h2m16 0h2M4.93 4.93l1.41 1.41m9.9 9.9l1.41 1.41" />
          </svg>
        </div>
      )}

      {/* ── Trim dual-thumb slider ───────────────────────────────────── */}
      {editTab === "trim" && duration > 0 && (
        <div className="max-w-lg mx-auto space-y-2">
          {/* Time labels */}
          <div className="flex justify-between text-xs text-gray-400 tabular-nums">
            <span>{formatTime(trimStart)}</span>
            <span className="text-white font-medium">
              {formatTime(trimEnd - trimStart)} selected
            </span>
            <span>{formatTime(trimEnd)}</span>
          </div>
          {/* Track */}
          <div
            ref={trackRef}
            className="relative h-8 cursor-pointer select-none touch-none"
            onPointerDown={handleTrackPointerDown}
            onPointerMove={handleTrackPointerMove}
            onPointerUp={handleTrackPointerUp}
          >
            {/* Background track */}
            <div className="absolute top-1/2 -translate-y-1/2 left-0 right-0 h-2 rounded-full bg-white/10" />
            {/* Selected range highlight */}
            <div
              className="absolute top-1/2 -translate-y-1/2 h-2 rounded-full bg-blue-500/60"
              style={{
                left: `${(trimStart / duration) * 100}%`,
                right: `${100 - (trimEnd / duration) * 100}%`,
              }}
            />
            {/* Start thumb */}
            <div
              className="absolute top-1/2 -translate-y-1/2 w-4 h-4 rounded-full bg-white border-2 border-blue-500 shadow-lg"
              style={{ left: `calc(${(trimStart / duration) * 100}% - 8px)` }}
            />
            {/* End thumb */}
            <div
              className="absolute top-1/2 -translate-y-1/2 w-4 h-4 rounded-full bg-white border-2 border-blue-500 shadow-lg"
              style={{ left: `calc(${(trimEnd / duration) * 100}% - 8px)` }}
            />
          </div>
          {/* Full duration label */}
          <div className="text-center text-xs text-gray-500">
            Full duration: {formatTime(duration)}
          </div>
        </div>
      )}

      {/* Trim loading state — duration not yet determined */}
      {editTab === "trim" && duration <= 0 && (
        <div className="max-w-lg mx-auto text-center py-3">
          <div className="flex items-center justify-center gap-2 text-gray-400 text-sm">
            <div className="w-4 h-4 border-2 border-gray-400 border-t-transparent rounded-full animate-spin" />
            Loading duration…
          </div>
          <p className="text-xs text-gray-500 mt-1">
            Play the media briefly if the duration doesn't appear.
          </p>
        </div>
      )}

      {/* ── Action buttons ───────────────────────────────────────────── */}
      <div className="flex items-center justify-center gap-2">
        <button
          onClick={onSave}
          className="px-5 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700 transition-colors"
        >
          Save
        </button>
        <button
          onClick={onSaveCopy}
          className="px-4 py-2 bg-green-600 text-white rounded-lg text-sm font-medium hover:bg-green-700 transition-colors"
          title="Save as a new copy — keeps the original unchanged"
        >
          Save Copy
        </button>
        {cropData && (
          <button
            onClick={onClear}
            className="px-4 py-2 bg-gray-600 text-white rounded-lg text-sm font-medium hover:bg-gray-500 transition-colors"
          >
            Reset
          </button>
        )}
        <button
          onClick={onCancel}
          className="px-4 py-2 bg-gray-700 text-white rounded-lg text-sm font-medium hover:bg-gray-600 transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
