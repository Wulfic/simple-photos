import type { CSSProperties } from "react";

/** TEMP crop-thumbnail tracing. Flip to false (or delete the gated logs) once
 *  the metadata-crop thumbnail bug is root-caused. */
export const CROP_DEBUG = true;

/**
 * TEMP: log to console AND stash into `window.__CROPLOG` so the whole trace can
 * be retrieved in one shot via `copy(JSON.stringify(window.__CROPLOG, null, 2))`
 * instead of hand-scraping the console. Remove with the rest of the CROP_DEBUG
 * instrumentation once the bug is fixed.
 */
export function croplog(tag: string, payload: Record<string, unknown>): void {
  if (!CROP_DEBUG) return;
  try { console.log(tag, payload); } catch { /* ignore */ }
  try {
    const w = window as unknown as { __CROPLOG?: Array<Record<string, unknown>> };
    if (!w.__CROPLOG) w.__CROPLOG = [];
    w.__CROPLOG.push({ tag, ...payload });
    if (w.__CROPLOG.length > 600) w.__CROPLOG.shift();
  } catch { /* SSR / no window — ignore */ }
}

/**
 * Aspect-ratio clamp applied to every tile by {@link JustifiedGrid}. Extreme
 * ratios (very wide / very tall crops) are clamped so a single item can't
 * produce a degenerate row. `getThumbnailStyle` MUST use the same clamp so the
 * crop transform is computed for the tile that is actually rendered — otherwise
 * an over-wide crop is positioned for a 10:1 tile but drawn into a clamped 4:1
 * tile, translating the image clean out of the box (blank tile + a sliver).
 */
export const TILE_ASPECT_MIN = 0.3;
export const TILE_ASPECT_MAX = 4;

export function clampTileAspect(ar: number): number {
  return Math.max(TILE_ASPECT_MIN, Math.min(ar, TILE_ASPECT_MAX));
}

/**
 * Return the effective visual aspect ratio (width / height) for a photo,
 * accounting for crop and rotation in `cropData`.  When a 90°/270° rotation
 * is stored in cropData, the visual aspect ratio is the inverse of the raw
 * pixel aspect ratio.  Returns 1 when dimensions are unknown.
 */
export function getEffectiveAspectRatio(
  width: number | undefined | null,
  height: number | undefined | null,
  cropData?: string | null,
): number {
  if (!width || !height) return 1;
  let w = width;
  let h = height;
  if (cropData) {
    try {
      const c = JSON.parse(cropData);
      const cropW = c.width || 1;
      const cropH = c.height || 1;
      const rot = ((c.rotate || 0) % 360 + 360) % 360;
      // The crop fractions are expressed in the ROTATED frame (the frame the
      // user draws on — see useViewerEdit.getMediaRect). So rotate the stored
      // dims into that frame FIRST, then apply the crop fractions. Doing it the
      // other way (multiply then swap) pairs cropW with the un-rotated width and
      // sizes the tile to the wrong aspect for any non-square rotated crop,
      // which then stretches the object-fit:fill thumbnail.
      if (rot % 180 !== 0) {
        [w, h] = [h, w];
      }
      w *= cropW;
      h *= cropH;
      croplog("[CROPDBG:gridAspect]", {
        storedW: width, storedH: height, storedAR: +(width / height).toFixed(3),
        crop: { w: cropW, h: cropH, rot },
        effectiveAR: +(w / h).toFixed(3),
        clampedTileAR: +clampTileAspect(w / h).toFixed(3),
      });
    } catch { /* malformed JSON — ignore */ }
  }
  return w / h;
}

/**
 * CSS transform that makes an `object-cover` <img> of the *full* (uncropped)
 * thumbnail display only the crop rectangle, filling the tile.
 *
 * The tile is laid out by {@link JustifiedGrid} at the *clamped* effective crop
 * aspect ratio. When `naturalW`/`naturalH` are known (and there's no rotation)
 * we compute the exact object-cover transform of the crop rect into a tile of
 * that clamped aspect — correct for ANY crop, including the extreme ones the
 * grid clamps. This general form reduces *exactly* to the legacy formula when
 * the effective aspect is within the clamp range (verified algebraically).
 *
 * The legacy path (no natural dims, or a rotation is present) keeps the old
 * behaviour, which is correct only while tile aspect == crop aspect.
 */
/**
 * Object-cover crop transform computed from **measured** ground-truth sizes:
 * the rendered tile's pixel size and the loaded image's natural size. This is
 * the robust form — it does not trust the photo's stored `width/height`, which
 * can be wrong/swapped and would otherwise mis-size the crop vertically (a gap
 * at the top of the tile with the crop sitting low). The tile size already
 * reflects whatever aspect the grid laid out (including the [0.3,4] clamp), so
 * covering the crop into the measured tile always fills it with no gap.
 *
 * Falls back to {@link getThumbnailStyle} when a rotation is present or any
 * measurement is missing (e.g. before the image has decoded).
 */
export function computeCropCoverTransform(
  cropJson: string | null | undefined,
  tileW: number,
  tileH: number,
  imgW: number,
  imgH: number,
): CSSProperties {
  if (!cropJson) return {};
  try {
    const c = JSON.parse(cropJson);
    const rot = ((c.rotate || 0) % 360 + 360) % 360;
    const cw = typeof c.width === "number" ? c.width : 1;
    const ch = typeof c.height === "number" ? c.height : 1;
    const x0 = typeof c.x === "number" ? c.x : 0;
    const y0 = typeof c.y === "number" ? c.y : 0;
    const cropped = cw < 0.999 || ch < 0.999;
    const measured = tileW > 0 && tileH > 0 && imgW > 0 && imgH > 0;

    if (cropped && rot === 0 && measured) {
      const arTile = tileW / tileH;
      const arImg = imgW / imgH;
      const s = Math.max(1, arTile / arImg);
      const cx = x0 + cw / 2;
      const cy = y0 + ch / 2;
      const scale = Math.max(arTile / (cw * arImg * s), 1 / (ch * s));
      const tx = (-(cx - 0.5) * arImg * s) / arTile * 100;
      const ty = -(cy - 0.5) * s * 100;
      const styles: CSSProperties = {
        transform: `scale(${scale}) translate(${tx}%, ${ty}%)`,
      };
      if (c.brightness) styles.filter = `brightness(${1 + c.brightness / 100})`;
      croplog("[CROPDBG:cover] MEASURED branch", {
        tileW, tileH, imgW, imgH, arTile: +arTile.toFixed(3), arImg: +arImg.toFixed(3),
        crop: { x: x0, y: y0, w: cw, h: ch }, scale: +scale.toFixed(3),
        tx: +tx.toFixed(2), ty: +ty.toFixed(2), transform: styles.transform,
      });
      return styles;
    }

    // ── Rotated path (90/180/270, with or without a crop) ──────────────────
    // The crop fractions (x, y, w, h) are stored in the *rotated* frame's 0–1
    // space (see useViewerEdit). The object-cover + rotate() legacy path is
    // wrong for two reasons: it computes the crop offset in the UN-rotated
    // frame, and object-cover clips the image to the tile *before* the rotate
    // runs (so the wrong axis is clipped, and any crop reaching into the
    // clipped overflow is lost). Instead, reproduce the fill technique used for
    // rot===0 crops, but in the rotated frame, using the measured pixel sizes.
    //
    // Model: the full image (rotated by `rot`) has a visual footprint of
    // Fw × Fh, where Fw = tileW/cw and Fh = tileH/ch (so the crop sub-rect,
    // which is `cw × ch` of that footprint, comes out to exactly tileW × tileH).
    // The footprint's top-left must sit at (Ox, Oy) relative to the tile so the
    // crop rect lands at the tile origin. We render the *un-rotated* <img> at
    // object-fit:fill into a box whose dimensions are swapped for 90/270, then
    // translate + rotate about the box centre to plant the footprint exactly.
    if (rot !== 0 && measured) {
      const fw = tileW / cw;            // rotated full-image footprint width
      const fh = tileH / ch;            // rotated full-image footprint height
      const ox = -(x0 / cw) * tileW;    // footprint top-left X (tile coords)
      const oy = -(y0 / ch) * tileH;    // footprint top-left Y (tile coords)
      const swapped = rot === 90 || rot === 270;
      // The un-rotated <img> box: rotating a (boxW × boxH) box by 90/270 yields
      // a (boxH × boxW) footprint, so swap to land on (fw × fh).
      const boxW = swapped ? fh : fw;
      const boxH = swapped ? fw : fh;
      // Translate the box centre so the rotated footprint centre = (ox+fw/2,
      // oy+fh/2). Rotation is about the box centre (default transform-origin),
      // which is translation-invariant for the centre point.
      const dx = swapped ? ox + fw / 2 - fh / 2 : ox;
      const dy = swapped ? oy + fh / 2 - fw / 2 : oy;
      const styles: CSSProperties = {
        position: "absolute",
        left: 0,
        top: 0,
        width: `${boxW}px`,
        height: `${boxH}px`,
        maxWidth: "none",
        objectFit: "fill",
        transform: `translate(${dx}px, ${dy}px) rotate(${rot}deg)`,
      };
      if (c.brightness) styles.filter = `brightness(${1 + c.brightness / 100})`;
      croplog("[CROPDBG:cover] ROTATED fill branch", {
        tileW, tileH, imgW, imgH, rot, cropped, crop: { x: x0, y: y0, w: cw, h: ch },
        fw: +fw.toFixed(2), fh: +fh.toFixed(2), boxW: +boxW.toFixed(2), boxH: +boxH.toFixed(2),
        dx: +dx.toFixed(2), dy: +dy.toFixed(2), transform: styles.transform,
      });
      return styles;
    }

    // Not yet measured — best-effort pure fallback.
    croplog("[CROPDBG:cover] FALLBACK → getThumbnailStyle (tile/img 0)", {
      tileW, tileH, imgW, imgH, rot, cropped, crop: { x: x0, y: y0, w: cw, h: ch },
    });
    return getThumbnailStyle(cropJson, imgW || undefined, imgH || undefined);
  } catch {
    return {};
  }
}

/**
 * Crop a thumbnail by sizing the *full* image and offsetting it, instead of
 * transforming an `object-cover` image.
 *
 * `object-cover` scales the image to cover the tile and **clips the overflow
 * before any transform runs**. A crop that reaches into that clipped overflow
 * (e.g. an edge strip of a landscape photo) loses those pixels, so scaling the
 * crop up to fill the tile leaves blank tile background — the long-standing
 * "metadata crop renders with a gap, Save Copy is fine" bug. Save Copy worked
 * because the server bakes pixels from the whole image; no CSS transform can
 * recover pixels `object-cover` already discarded.
 *
 * Here the `<img>` is sized so the full image is `(tileW/cw) x (tileH/ch)` and
 * shifted left/up by the crop origin, so the crop rectangle exactly fills the
 * tile while the *entire* image remains the paint source — only the tile
 * wrapper (`overflow-hidden`) clips. `object-fit: fill` keeps it exact (no
 * distortion while the tile aspect matches the crop's effective aspect, which
 * it does except for the rare clamped extreme crop).
 *
 * Returns `null` for uncropped or rotated photos — those keep the existing
 * `object-cover` (+ `rotate()`) path, which has no clipping problem.
 */
export function getCropFillStyle(cropJson?: string | null): CSSProperties | null {
  if (!cropJson) return null;
  try {
    const c = JSON.parse(cropJson);
    const rot = ((c.rotate || 0) % 360 + 360) % 360;
    const cw = typeof c.width === "number" ? c.width : 1;
    const ch = typeof c.height === "number" ? c.height : 1;
    const x0 = typeof c.x === "number" ? c.x : 0;
    const y0 = typeof c.y === "number" ? c.y : 0;
    const cropped = cw < 0.999 || ch < 0.999;
    if (!cropped || rot !== 0 || cw <= 0 || ch <= 0) return null;

    const style: CSSProperties = {
      position: "absolute",
      width: `${100 / cw}%`,
      height: `${100 / ch}%`,
      left: `${-(x0 / cw) * 100}%`,
      top: `${-(y0 / ch) * 100}%`,
      maxWidth: "none",
      objectFit: "fill",
    };
    if (c.brightness) style.filter = `brightness(${1 + c.brightness / 100})`;
    if (CROP_DEBUG) croplog("[CROPDBG:fill]", {
      cw, ch, x0, y0, width: style.width as string, height: style.height as string,
      left: style.left as string, top: style.top as string,
    });
    return style;
  } catch {
    return null;
  }
}

export function getThumbnailStyle(
  cropJson?: string | null,
  naturalW?: number | null,
  naturalH?: number | null,
): CSSProperties {
  if (!cropJson) return {};
  try {
    const c = JSON.parse(cropJson);
    const styles: CSSProperties = {};

    const rot = ((c.rotate || 0) % 360 + 360) % 360;
    const cw = typeof c.width === "number" ? c.width : 1;
    const ch = typeof c.height === "number" ? c.height : 1;
    const x0 = typeof c.x === "number" ? c.x : 0;
    const y0 = typeof c.y === "number" ? c.y : 0;
    const cropped = cw < 0.999 || ch < 0.999;

    // ── General, clamp-aware path ─────────────────────────────────────────
    if (cropped && rot === 0 && naturalW && naturalH && naturalW > 0 && naturalH > 0) {
      const arImg = naturalW / naturalH;
      const effective = (naturalW * cw) / (naturalH * ch);
      const arTile = clampTileAspect(effective);
      // object-cover base scale of the full image into the (clamped) tile.
      const s = Math.max(1, arTile / arImg);
      const cx = x0 + cw / 2;
      const cy = y0 + ch / 2;
      // Scale the crop rect so it *covers* the tile, then recentre it.
      const scale = Math.max(arTile / (cw * arImg * s), 1 / (ch * s));
      const tx = (-(cx - 0.5) * arImg * s) / arTile * 100;
      const ty = -(cy - 0.5) * s * 100;
      styles.transform = `scale(${scale}) translate(${tx}%, ${ty}%)`;
      if (c.brightness) styles.filter = `brightness(${1 + c.brightness / 100})`;
      croplog("[CROPDBG:thumbStyle] GENERAL branch (natural dims)", {
        naturalW, naturalH, arImg: +arImg.toFixed(3), effective: +effective.toFixed(3),
        arTile: +arTile.toFixed(3), crop: { x: x0, y: y0, w: cw, h: ch },
        scale: +scale.toFixed(3), tx: +tx.toFixed(2), ty: +ty.toFixed(2), transform: styles.transform,
      });
      return styles;
    }

    // ── Legacy path (rotation present, or natural dims unknown) ────────────
    // Assumes the tile is sized to the crop's own aspect ratio. See the
    // function doc — only correct while tile aspect == crop aspect.
    const parts: string[] = [];
    if (cropped) {
      const scale = 1 / Math.max(cw, ch);
      const cx = x0 + cw / 2;
      const cy = y0 + ch / 2;
      const fx = Math.max(1, ch / cw);
      const fy = Math.max(1, cw / ch);
      const tx = (0.5 - cx) * fx * 100;
      const ty = (0.5 - cy) * fy * 100;
      parts.push(`scale(${scale})`, `translate(${tx}%, ${ty}%)`);
    }

    if (rot) {
      // 90°/270° rotations swap the visual width/height but the layout box
      // stays unchanged. object-cover + overflow-hidden clip and fill correctly.
      parts.push(`rotate(${rot}deg)`);
    }

    if (parts.length) {
      styles.transform = parts.join(" ");
    }

    if (c.brightness) {
      styles.filter = `brightness(${1 + c.brightness / 100})`;
    }

    croplog("[CROPDBG:thumbStyle] LEGACY branch (no natural dims or rotated)", {
      naturalW: naturalW ?? null, naturalH: naturalH ?? null, rot, crop: { x: x0, y: y0, w: cw, h: ch }, transform: styles.transform,
    });
    return styles;
  } catch {
    return {};
  }
}
