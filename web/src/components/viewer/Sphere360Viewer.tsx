/**
 * Sphere360Viewer — interactive viewer for equirectangular 360° photos.
 *
 * Backed by Photo Sphere Viewer (`@photo-sphere-viewer/core`), the de-facto
 * standard three.js photo-sphere library.  Compared to the previous hand-rolled
 * sphere it gives us, for free:
 *   - correct equirectangular projection with no pole distortion,
 *   - GPano XMP pose handling (PoseHeading/Pitch/RollDegrees) and
 *     CroppedArea bounds for partial panoramas (`useXmpData` is on by default),
 *   - drag / wheel / pinch / gyroscope controls and a little-planet intro.
 *
 * The old viewer rendered this content sideways because it textured the sphere
 * straight from `createImageBitmap(blob)` with no EXIF-orientation handling and
 * ignored GPano pose entirely.  We fix orientation here with `buildOrientedSource`
 * (see below) and let PSV handle pose.
 *
 * Controls:
 *   - Drag / touch to look around, wheel / pinch to zoom.
 *   - "360° Full View" pill drops back to the flat equirectangular preview.
 */
import { useEffect, useRef, useState } from "react";
import { Viewer } from "@photo-sphere-viewer/core";
import "@photo-sphere-viewer/core/index.css";

interface Sphere360ViewerProps {
  /** Object URL or data URL of the equirectangular image. */
  mediaUrl: string;
  /** Called when the user clicks the toggle to leave 360° mode. */
  onExitToFull: () => void;
}

/** Largest texture edge we will re-encode to when re-orienting a rotated file.
 *  Keeps the canvas allocation bounded on mobile GPUs. */
const MAX_REORIENT_EDGE = 8192;

/**
 * Read the EXIF `Orientation` tag (1–8) from JPEG bytes.  Returns `1` for
 * non-JPEG data, missing EXIF, or any malformed header — i.e. "no rotation
 * needed".  A minimal SOI → APP1 → TIFF/IFD0 walk; enough to decide whether the
 * pixels must be re-oriented before they go on the sphere, without pulling in a
 * full EXIF parser.
 */
function readJpegOrientation(buf: ArrayBuffer): number {
  const view = new DataView(buf);
  const len = view.byteLength;
  if (len < 2 || view.getUint16(0, false) !== 0xffd8) return 1; // not a JPEG (SOI)

  let offset = 2;
  while (offset + 4 <= len) {
    const marker = view.getUint16(offset, false);
    offset += 2;
    if (marker === 0xffe1) {
      // APP1 — should contain the Exif TIFF header.
      const segStart = offset + 2; // skip the 2-byte segment length
      // "Exif" magic == 0x45786966
      if (segStart + 8 > len || view.getUint32(segStart, false) !== 0x45786966) return 1;
      const tiff = segStart + 6; // skip "Exif\0\0"
      if (tiff + 8 > len) return 1;
      const little = view.getUint16(tiff, false) === 0x4949; // "II" => little-endian
      const firstIFD = view.getUint32(tiff + 4, little);
      const dir = tiff + firstIFD;
      if (dir + 2 > len) return 1;
      const entries = view.getUint16(dir, little);
      for (let i = 0; i < entries; i++) {
        const entry = dir + 2 + i * 12;
        if (entry + 12 > len) break;
        if (view.getUint16(entry, little) === 0x0112) {
          const value = view.getUint16(entry + 8, little);
          return value >= 1 && value <= 8 ? value : 1;
        }
      }
      return 1;
    }
    if ((marker & 0xff00) !== 0xff00) break; // not a marker segment — give up
    if (offset + 2 > len) break;
    offset += view.getUint16(offset, false); // skip this segment's payload
  }
  return 1;
}

/**
 * Resolve a panorama source URL whose pixels are in canonical (orientation = 1)
 * layout, ready to texture a sphere.
 *
 * For the overwhelmingly common orientation-1 case we return `mediaUrl`
 * untouched — this is zero-copy and, crucially, preserves the embedded GPano
 * XMP that PSV reads for pose / cropped-area.  Only when the file actually
 * carries a rotating EXIF orientation (2–8) do we decode with
 * `imageOrientation: "from-image"` and re-encode the upright pixels through a
 * canvas (the re-encode necessarily drops XMP, an acceptable trade for the rare
 * rotated sphere — those almost never also carry GPano pose).
 */
async function buildOrientedSource(
  mediaUrl: string,
  signal: AbortSignal,
): Promise<{ url: string; created: boolean }> {
  const resp = await fetch(mediaUrl, { signal });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  const blob = await resp.blob();
  const buf = await blob.arrayBuffer();

  const orientation = readJpegOrientation(buf);
  if (orientation === 1 || typeof createImageBitmap !== "function") {
    return { url: mediaUrl, created: false };
  }

  // Rotated file: decode applying EXIF orientation, then re-encode upright.
  const bitmap = await createImageBitmap(blob, { imageOrientation: "from-image" });
  try {
    const { width, height } = bitmap;
    const scale =
      width > MAX_REORIENT_EDGE || height > MAX_REORIENT_EDGE
        ? Math.min(MAX_REORIENT_EDGE / width, MAX_REORIENT_EDGE / height)
        : 1;
    const cw = Math.max(1, Math.round(width * scale));
    const ch = Math.max(1, Math.round(height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = cw;
    canvas.height = ch;
    const ctx = canvas.getContext("2d");
    if (!ctx) return { url: mediaUrl, created: false };
    ctx.drawImage(bitmap, 0, 0, cw, ch);
    const outBlob = await new Promise<Blob | null>((res) =>
      canvas.toBlob(res, "image/jpeg", 0.92),
    );
    if (!outBlob) return { url: mediaUrl, created: false };
    return { url: URL.createObjectURL(outBlob), created: true };
  } finally {
    bitmap.close();
  }
}

export default function Sphere360Viewer({ mediaUrl, onExitToFull }: Sphere360ViewerProps) {
  const mountRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    setReady(false);
    setError(null);

    let viewer: Viewer | null = null;
    let createdUrl: string | null = null;
    let cancelled = false;
    const controller = new AbortController();

    (async () => {
      let source: { url: string; created: boolean };
      try {
        source = await buildOrientedSource(mediaUrl, controller.signal);
      } catch (e) {
        if (!cancelled) {
          // eslint-disable-next-line no-console
          console.error("[Sphere360Viewer] source load failed:", e);
          setError("Failed to load panorama image.");
        }
        return;
      }
      if (cancelled) {
        if (source.created) URL.revokeObjectURL(source.url);
        return;
      }
      if (source.created) createdUrl = source.url;

      try {
        // EquirectangularAdapter is the default; useXmpData defaults to true so
        // GPano pose + cropped-area are honoured automatically.
        viewer = new Viewer({
          container: mount,
          panorama: source.url,
          navbar: false,
          loadingTxt: "Loading 360° view…",
          mousewheel: true,
          defaultZoomLvl: 30,
        });
      } catch (e) {
        // PSV throws synchronously when WebGL is unavailable.
        // eslint-disable-next-line no-console
        console.error("[Sphere360Viewer] viewer init failed:", e);
        setError("WebGL is not available in this browser.");
        return;
      }

      viewer.addEventListener(
        "ready",
        () => {
          if (!cancelled) setReady(true);
        },
        { once: true },
      );
      viewer.addEventListener("panorama-error", () => {
        if (!cancelled) setError("Failed to load panorama image.");
      });
    })();

    return () => {
      cancelled = true;
      controller.abort();
      if (viewer) viewer.destroy();
      if (createdUrl) URL.revokeObjectURL(createdUrl);
    };
  }, [mediaUrl]);

  return (
    <div className="relative w-full h-full bg-black select-none">
      {/* PSV mount.  Hidden (not unmounted) on error so the cleanup in the init
          effect still finds the ref and tears the viewer down correctly. */}
      <div
        ref={mountRef}
        className="absolute inset-0"
        style={{ visibility: error ? "hidden" : "visible" }}
      />

      {!ready && !error && (
        <div className="absolute inset-0 flex items-center justify-center text-white text-sm pointer-events-none">
          Loading 360° view…
        </div>
      )}
      {error && (
        <>
          {/* Flat fallback so the user still sees the image when WebGL or the
              texture load fails. */}
          <img
            src={mediaUrl}
            alt="Panorama"
            className="absolute inset-0 w-full h-full object-contain pointer-events-none"
          />
          <div className="absolute top-4 left-1/2 -translate-x-1/2 z-30 px-3 py-1 rounded-full bg-red-600/80 text-white text-xs">
            {error}
          </div>
        </>
      )}

      <button
        onClick={(e) => {
          e.stopPropagation();
          onExitToFull();
        }}
        className="absolute bottom-24 left-1/2 -translate-x-1/2 z-30 flex items-center gap-2 px-4 py-1.5 rounded-full bg-black/60 text-white text-sm font-medium hover:bg-black/80 transition-colors backdrop-blur-sm"
      >
        <span className="text-xs font-bold opacity-70">360°</span>
        <span>Full View</span>
      </button>
    </div>
  );
}
