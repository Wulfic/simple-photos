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
    let transform = "";

    const rot = ((c.rotate || 0) % 360 + 360) % 360;
    
    // Scale and translate
    if (c.width && c.width < 1 && c.height && c.height < 1) {
      const zoom = Math.max(1 / c.width, 1 / c.height);
      transform += `scale(${zoom}) `;
      
      const cx = (c.x || 0) + c.width / 2;
      const cy = (c.y || 0) + c.height / 2;
      
      const tx = (0.5 - cx) * 100;
      const ty = (0.5 - cy) * 100;
      transform += `translate(${tx}%, ${ty}%) `;
    }

    if (rot) {
      transform += `rotate(${rot}deg) `;
      // 90°/270° rotations swap the visual width/height but the layout box
      // stays unchanged.  This is fine because tile wrappers use
      // overflow-hidden + object-cover which clip and fill correctly.
    }

    if (transform) {
      styles.transform = transform.trim();
    }
    
    if (c.brightness) {
      styles.filter = `brightness(${1 + c.brightness / 100})`;
    }

    return styles;
  } catch {
    return {};
  }
}
