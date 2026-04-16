import type { CSSProperties } from "react";

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
