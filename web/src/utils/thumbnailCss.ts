import type { CSSProperties } from "react";

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
      w *= cropW;
      h *= cropH;
      const rot = ((c.rotate || 0) % 360 + 360) % 360;
      if (rot % 180 !== 0) {
        [w, h] = [h, w];
      }
    } catch { /* malformed JSON — ignore */ }
  }
  return w / h;
}

export function getThumbnailStyle(cropJson?: string | null): CSSProperties {
  if (!cropJson) return {};
  try {
    const c = JSON.parse(cropJson);
    const styles: CSSProperties = {};
    const parts: string[] = [];

    const rot = ((c.rotate || 0) % 360 + 360) % 360;
    const cw = typeof c.width === "number" ? c.width : 1;
    const ch = typeof c.height === "number" ? c.height : 1;

    // The gallery tile is sized to the *cropped* aspect ratio
    // (getEffectiveAspectRatio) and the <img> uses object-cover, so the image
    // is pre-scaled by object-cover's `s = max(tileW/W, tileH/H)`. To make the
    // crop rectangle exactly fill the tile:
    //
    //   scale = 1 / max(cw, ch)
    //
    // — NOT max(1/cw, 1/ch), which over-zooms every non-square crop (the bug
    // behind "the thumbnail doesn't match the editor", #4). The crop centre is
    // then translated to the tile centre. Because object-cover only letterboxes
    // along the *minor* axis, the translate on that axis is amplified by the
    // cw:ch ratio (factor `fx`/`fy` below). Derivation verified against the
    // Viewer's computeCropZoom for centred, wide, and off-centre strips.
    const cropped = cw < 0.999 || ch < 0.999;
    if (cropped) {
      const scale = 1 / Math.max(cw, ch);
      const cx = (c.x || 0) + cw / 2;
      const cy = (c.y || 0) + ch / 2;
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

    return styles;
  } catch {
    return {};
  }
}
