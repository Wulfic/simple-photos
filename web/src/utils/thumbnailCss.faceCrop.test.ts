import { describe, it, expect } from "vitest";
import {
  computeFaceCropStyle,
  FACE_MAX_ZOOM,
  FACE_TARGET_FRACTION,
} from "./thumbnailCss";

describe("computeFaceCropStyle", () => {
  it("returns empty style for missing/degenerate boxes", () => {
    expect(computeFaceCropStyle(null)).toEqual({});
    expect(computeFaceCropStyle(undefined)).toEqual({});
    expect(computeFaceCropStyle({ x: 0, y: 0, w: 0, h: 0.2 })).toEqual({});
    expect(computeFaceCropStyle({ x: 0, y: 0, w: 0.2, h: -1 })).toEqual({});
    expect(computeFaceCropStyle({ x: NaN, y: 0, w: 0.2, h: 0.2 })).toEqual({});
  });

  it("centres the crop on the face centre", () => {
    // Face fills right-bottom quadrant → centre at (0.75, 0.75).
    const s = computeFaceCropStyle({ x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
    expect(s.objectPosition).toBe("75.00% 75.00%");
    expect(s.transformOrigin).toBe("75.00% 75.00%");
  });

  it("zooms small faces up to the clamp, big faces stay at 1", () => {
    // Tiny face (0.1) → target/0.1 = 6 → clamped to FACE_MAX_ZOOM.
    const tiny = computeFaceCropStyle({ x: 0.4, y: 0.4, w: 0.1, h: 0.1 });
    expect(tiny.transform).toBe(`scale(${FACE_MAX_ZOOM.toFixed(3)})`);

    // Large face already bigger than target → no extra zoom.
    const big = computeFaceCropStyle({ x: 0.1, y: 0.1, w: 0.8, h: 0.8 });
    expect(big.transform).toBe("scale(1.000)");
  });

  it("scales proportionally between the bounds", () => {
    // max(w,h)=0.3 → target/0.3 = 2.0, within [1, MAX].
    const s = computeFaceCropStyle({ x: 0.3, y: 0.3, w: 0.3, h: 0.2 });
    const expected = (FACE_TARGET_FRACTION / 0.3).toFixed(3);
    expect(s.transform).toBe(`scale(${expected})`);
  });

  it("clamps the centre to the tile for a face bleeding off-frame", () => {
    const s = computeFaceCropStyle({ x: 0.9, y: 0.9, w: 0.4, h: 0.4 });
    // centre = 1.1 → clamped to 1.0
    expect(s.objectPosition).toBe("100.00% 100.00%");
  });
});
