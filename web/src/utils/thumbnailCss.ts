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
      if (rot === 90 || rot === 270) {
          // If the thumbnail wrapper is non-square, rotating will mess up aspect,
          // but our wrappers are `aspect-square`, `overflow-hidden`.
          // If the image itself is object-cover, it should look right.
      }
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
