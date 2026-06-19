/**
 * ThumbnailTile — unified gallery tile component.
 *
 * Replaces MediaTile (main gallery), SecureGalleryTile.ItemTile (secure gallery),
 * and PhotoThumbnail (secure gallery helper) with a single component that handles
 * all view contexts: main gallery, secure gallery, albums, shared albums.
 *
 * Uses the `useThumbnailLoader` hook for decrypted/server thumbnail resolution
 * and `useGifAutoplay` for large GIF full-blob loading.
 */
import { useRef, useEffect, useState, useCallback, type CSSProperties } from "react";
import useLongPress from "../../hooks/useLongPress";
import { useThumbnailLoader } from "../hooks/useThumbnailLoader";
import { useGifAutoplay } from "../hooks/useGifAutoplay";
import { getThumbnailStyle, computeCropCoverTransform, getCropFillStyle, CROP_DEBUG, croplog } from "../../utils/thumbnailCss";
import { formatDuration } from "../../utils/gallery";
import type { ThumbnailTileProps } from "../types";

export default function ThumbnailTile({
  source,
  mediaType,
  filename,
  cropData,
  width,
  height,
  duration,
  photoSubtype,
  burstCount,
  onClick,
  onLongPress,
  selectionMode,
  isSelected,
  onDimensionMismatch,
}: ThumbnailTileProps) {
  const tileRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const [visible, setVisible] = useState(false);
  // Blur-up / fade-in: the image starts transparent over the neutral tile
  // background and fades in once decoded, instead of popping in.
  const [decoded, setDecoded] = useState(false);
  // Crop transform computed from MEASURED tile + image sizes (ground truth),
  // so the crop fills the tile regardless of stored-dimension errors or the
  // grid's aspect clamp. Null until measured → falls back to the pure
  // stored-dims transform for the first paint.
  const [measuredCropStyle, setMeasuredCropStyle] = useState<CSSProperties | null>(null);

  const isGif = mediaType === "gif";
  const hasAnimatedThumb = isGif && source.thumbnailMimeType === "image/gif";
  const needsFullGifLoad = isGif && !hasAnimatedThumb;

  // Lazy visibility gate — only start loading when the tile enters the viewport
  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          if (!needsFullGifLoad) observer.disconnect(); // One-shot for non-GIF
        }
      },
      { rootMargin: `${Math.max(200, Math.round(window.innerHeight * 0.5))}px` },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [needsFullGifLoad]);

  // Thumbnail loading (cache → IDB → encrypted blob → server fallback)
  const thumb = useThumbnailLoader(source, visible);

  // GIF autoplay (large GIFs only — small GIFs play directly from thumbnail)
  const gif = useGifAutoplay(
    tileRef,
    source.storageBlobId ?? source.blobId,
    source.serverSide ? source.serverPhotoId : undefined,
    needsFullGifLoad && visible,
  );

  // The displayed URL: full GIF blob when in view, otherwise the thumbnail
  const displayUrl = gif.fullGifUrl ?? thumb.url;

  // Reset the fade when the source URL changes. If the new image is already
  // cached/complete (blob URLs often are), the onLoad event may not fire, so
  // mark it decoded straight away to avoid a tile that stays blank.
  useEffect(() => {
    if (!displayUrl) {
      setDecoded(false);
      return;
    }
    const img = imgRef.current;
    if (img && img.complete && img.naturalWidth > 0) {
      setDecoded(true);
    } else {
      setDecoded(false);
    }
  }, [displayUrl]);

  // ── Measured crop transform ─────────────────────────────────────────────
  // Compute the crop transform from the REAL tile + image sizes so a crop
  // always fills its tile, even when the photo's stored dimensions are wrong
  // (which would otherwise leave a gap / mis-position the crop vertically).
  const measureCrop = useCallback(() => {
    if (!cropData || isGif) { setMeasuredCropStyle(null); return; }
    const tile = tileRef.current;
    const img = imgRef.current;
    croplog("[CROPDBG:measure]", {
      filename,
      tileNull: !tile, imgNull: !img,
      tileW: tile?.clientWidth, tileH: tile?.clientHeight,
      natW: img?.naturalWidth, natH: img?.naturalHeight,
      storedW: width, storedH: height,
      storedAR: width && height ? +(width / height).toFixed(3) : null,
      natAR: img?.naturalWidth ? +(img.naturalWidth / img.naturalHeight).toFixed(3) : null,
      cropData,
    });
    if (!tile || !img || img.naturalWidth === 0) {
      croplog("[CROPDBG:measure] EARLY RETURN", {
        filename,
        reason: !tile ? "no tile ref" : !img ? "no img ref" : "img.naturalWidth===0 (not decoded)",
      });
      return;
    }
    if (tile.clientWidth === 0 || tile.clientHeight === 0) {
      croplog("[CROPDBG:measure] ⚠️ TILE NOT LAID OUT (0 size) — cover falls back to natural-dims getThumbnailStyle", {
        filename, tileW: tile.clientWidth, tileH: tile.clientHeight,
      });
    }
    const style = computeCropCoverTransform(
      cropData,
      tile.clientWidth,
      tile.clientHeight,
      img.naturalWidth,
      img.naturalHeight,
    );
    croplog("[CROPDBG:measure] → setMeasuredCropStyle", { filename, style: style as Record<string, unknown> });
    setMeasuredCropStyle(style);
  }, [cropData, isGif, filename, width, height]);

  // Reset + re-measure whenever the crop or the displayed image changes.
  useEffect(() => { setMeasuredCropStyle(null); measureCrop(); }, [cropData, displayUrl, measureCrop]);

  // Re-measure on tile resize (window resize / layout changes re-flow the grid,
  // changing the tile's clamped aspect). Only cropped, non-GIF tiles need this.
  useEffect(() => {
    if (!cropData || isGif) return;
    const tile = tileRef.current;
    if (!tile) return;
    const ro = new ResizeObserver(() => measureCrop());
    ro.observe(tile);
    return () => ro.disconnect();
  }, [cropData, isGif, measureCrop]);

  // ── [CROPDBG] Trace which style actually wins (measured vs stored fallback) ──
  useEffect(() => {
    if (!CROP_DEBUG || !cropData || isGif) return;
    const tile = tileRef.current;
    const img = imgRef.current;
    const fallback = getThumbnailStyle(cropData, width, height);
    croplog("[CROPDBG:applied]", {
      filename,
      usingMeasured: !!measuredCropStyle,
      applied: (measuredCropStyle ?? fallback) as Record<string, unknown>,
      measuredCropStyle: measuredCropStyle as Record<string, unknown> | null,
      storedFallback: fallback as Record<string, unknown>,
      storedW: width, storedH: height,
      liveTileW: tile?.clientWidth, liveTileH: tile?.clientHeight,
      liveNatW: img?.naturalWidth, liveNatH: img?.naturalHeight,
    });
  }, [measuredCropStyle, cropData, isGif, filename, width, height]);

  // Crop rendering: size the FULL image and offset it so the crop fills the
  // tile, instead of transforming an object-cover image (which clips the crop's
  // overflow pixels before the transform — the metadata-crop "gap" bug). Null
  // for uncropped/rotated/GIF → keep the object-cover path.
  const cropFill = isGif ? null : getCropFillStyle(cropData);

  // Final style applied to the <img>. cropFill (pure rot=0 crop) wins first,
  // then the measured transform (rot=0 cover crop OR the rotated fill branch),
  // then the stored-dims fallback for the first paint. Any style that sizes &
  // positions the image absolutely is a "fill" style — those must NOT also get
  // object-cover (which would re-clip and fight the manual sizing).
  const appliedCropStyle: CSSProperties | undefined = isGif
    ? undefined
    : (cropFill ?? measuredCropStyle ?? getThumbnailStyle(cropData, width, height));
  const usesFill = appliedCropStyle?.position === "absolute";

  // Long press for selection mode
  const longPress = useLongPress(() => onLongPress?.(), 500);

  return (
    <div
      ref={tileRef}
      className={`relative w-full h-full bg-surface-raised overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group ${isSelected ? "ring-2 ring-accent-500" : ""}`}
      onClick={(e) => {
        if (longPress.wasLongPress()) { e.preventDefault(); return; }
        onClick();
      }}
      onTouchStart={longPress.onTouchStart}
      onTouchEnd={longPress.onTouchEnd}
      onTouchMove={longPress.onTouchMove}
      onContextMenu={(e) => e.preventDefault()}
    >
      {displayUrl ? (
        <>
          {/* GIFs use object-cover without crop transforms (breaks animation).
              JustifiedGrid already sizes the tile to match the GIF's aspect ratio. */}
          <img
            ref={imgRef}
            src={displayUrl}
            alt={filename}
            className={`${usesFill ? "" : "w-full h-full object-cover"} transition-opacity duration-500 ease-out motion-reduce:transition-none ${decoded ? "opacity-100" : "opacity-0"}`}
            loading="lazy"
            style={appliedCropStyle}
            onLoad={(e) => {
              setDecoded(true);
              measureCrop();
              if (onDimensionMismatch) {
                const img = e.currentTarget;
                const nw = img.naturalWidth;
                const nh = img.naturalHeight;
                // Self-heal: if the thumbnail's orientation disagrees with the
                // stored dimensions, push the corrected dims (the parent only
                // applies them on an *exact* w↔h swap, so false positives are
                // impossible). This MUST also run for cropped photos: a
                // metadata-only crop never regenerates the thumbnail, so the
                // thumbnail is still the full image and its natural dims are
                // exactly what getEffectiveAspectRatio needs to size the tile.
                // Skipping it left a crop on a photo with swapped stored dims
                // sized to the wrong tile aspect, so the (otherwise correct)
                // getThumbnailStyle transform mismatched the tile and produced
                // a zoomed/garbled thumbnail.
                if (nw > 0 && nh > 0 && nw !== nh) {
                  onDimensionMismatch(nw, nh);
                }
              }
            }}
          />

          {/* Filename overlay for audio files */}
          {mediaType === "audio" && (
            <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/60 to-transparent px-1 pb-0.5 pt-3 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
              <span className="text-white text-[10px] leading-tight line-clamp-1 break-all drop-shadow">{filename}</span>
            </div>
          )}
        </>
      ) : (
        <div className="w-full h-full flex flex-col items-center justify-center gap-1.5 px-1 text-center">
          {thumb.state === "loading" || thumb.state === "placeholder" ? (
            !source.thumbnailData ? (
              <>
                <div className="w-5 h-5 border-2 border-edge-strong border-t-accent-500 dark:border-t-accent-400 rounded-full animate-spin" />
                <span className="text-[10px] font-medium text-fg-muted uppercase tracking-wide">Queued</span>
              </>
            ) : (
              <span className="text-fg-muted text-xs">{filename}</span>
            )
          ) : thumb.state === "error" ? (
            <div className="text-center text-fg-muted">
              <span className="text-2xl block mb-1">🔐</span>
              <span className="text-xs">Encrypted</span>
            </div>
          ) : (
            <span className="text-fg-muted text-xs">{filename}</span>
          )}
        </div>
      )}

      {/* Media type badges */}
      {mediaType === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
          {duration ? <span>{formatDuration(duration)}</span> : null}
        </div>
      )}
      {mediaType === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}
      {mediaType === "audio" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>♫</span>
          {duration ? <span>{formatDuration(duration)}</span> : null}
        </div>
      )}

      {/* Photo subtype badges */}
      {photoSubtype === "burst" && (
        <div className="absolute top-1 left-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M4 6h16M4 10h16M4 14h16" />
          </svg>
          {burstCount && burstCount > 1 ? <span>{burstCount}</span> : null}
        </div>
      )}
      {photoSubtype === "motion" && (
        <div className="absolute top-1 left-1 bg-black/60 text-white text-[10px] font-bold px-1.5 py-0.5 rounded">
          LIVE
        </div>
      )}
      {(photoSubtype === "panorama" || photoSubtype === "equirectangular") && (
        <div className="absolute top-1 left-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6.115 5.19l.319 1.913A6 6 0 008.11 10.36L9.75 12l-.387.775c-.217.433-.132.956.21 1.298l1.348 1.348c.21.21.329.497.329.795v1.089c0 .426.24.815.622 1.006l.153.076c.433.217.956.132 1.298-.21l.723-.723a8.7 8.7 0 002.288-4.042 1.087 1.087 0 00-.358-1.099l-1.33-1.108c-.251-.21-.582-.299-.905-.245l-1.17.195a1.125 1.125 0 01-.98-.314l-.295-.295a1.125 1.125 0 010-1.591l.13-.132a1.125 1.125 0 011.3-.21l.603.302a.809.809 0 001.086-1.086L14.25 7.5l1.256-.837a4.5 4.5 0 001.528-1.732l.146-.292M6.115 5.19A9 9 0 1017.18 4.64M6.115 5.19A8.965 8.965 0 0112 3c1.929 0 3.716.607 5.18 1.64" />
          </svg>
          {photoSubtype === "equirectangular" ? "360°" : "PANO"}
        </div>
      )}

      {/* Selection indicator — hidden by default; appears on hover (desktop)
          or once selection mode is active (e.g. via long-press on touch).
          Selected items always show the green check. */}
      {onLongPress && (
        <button
          type="button"
          aria-label={isSelected ? "Deselect" : "Select"}
          onClick={(e) => {
            e.stopPropagation();
            e.preventDefault();
            // The parent's onLongPress handler is what flips selectionMode on,
            // and onClick toggles when already in selectionMode. Reuse both.
            if (selectionMode) {
              onClick();
            } else {
              onLongPress();
            }
          }}
          onPointerDown={(e) => e.stopPropagation()}
          onTouchStart={(e) => e.stopPropagation()}
          className={`absolute top-1.5 right-1.5 w-5 h-5 rounded-full border-2 flex items-center justify-center transition-all z-10 ${
            isSelected
              ? "bg-green-500 border-green-500 shadow opacity-100"
              : selectionMode
                ? "bg-white/80 border-gray-400 hover:bg-white opacity-100"
                : "bg-white/40 border-white/70 shadow-sm opacity-0 group-hover:opacity-100 hover:bg-white/80"
          }`}
        >
          {isSelected && (
            <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          )}
        </button>
      )}
    </div>
  );
}
