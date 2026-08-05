import { describe, it, expect } from "vitest";
import {
  computeFaceCropStyle,
  faceCropRect,
  FACE_MAX_ZOOM,
  FACE_TARGET_FRACTION,
  type FaceBox,
} from "./thumbnailCss";

/**
 * These tests assert the *property* (the face centre lands in the middle of the
 * tile), not the formula. The previous suite asserted the formula's output —
 * `objectPosition: "75.00% 75.00%"` under a test named "centres the crop on the
 * face centre" — which is exactly how #48 shipped: scaling about a
 * transform-origin holds that point still instead of moving it to the centre,
 * so the assertion and the bug agreed with each other.
 */

const pct = (v: string | number | undefined) => parseFloat(String(v));

/** Invert the produced CSS: where in the container (0–1) does image-point `c`
 *  land? Container is 1 unit; `left`/`width` are percentages of it. */
function landsAt(style: Record<string, unknown>, axis: "x" | "y", c: number) {
  const offset = pct(style[axis === "x" ? "left" : "top"] as string) / 100;
  const size = pct(style[axis === "x" ? "width" : "height"] as string) / 100;
  return offset + c * size;
}

const centreOf = (b: FaceBox) => ({ cx: b.x + b.w / 2, cy: b.y + b.h / 2 });

describe("faceCropRect", () => {
  it("returns null for missing/degenerate boxes", () => {
    expect(faceCropRect(null)).toBeNull();
    expect(faceCropRect(undefined)).toBeNull();
    expect(faceCropRect({ x: 0, y: 0, w: 0, h: 0.2 })).toBeNull();
    expect(faceCropRect({ x: 0, y: 0, w: 0.2, h: -1 })).toBeNull();
    expect(faceCropRect({ x: NaN, y: 0, w: 0.2, h: 0.2 })).toBeNull();
  });

  it("expands the box uniformly, so a pixel-square face stays square", () => {
    // w/h differ only because the photo is 4:3; the window must preserve that
    // ratio or the face renders stretched in a square tile.
    const box = { x: 0.3, y: 0.3, w: 0.15, h: 0.2 };
    const r = faceCropRect(box)!;
    expect(r.zx / r.zy).toBeCloseTo(box.w / box.h, 6);
  });

  it("shows the face at the target fraction of the tile", () => {
    const box = { x: 0.4, y: 0.4, w: 0.2, h: 0.2 };
    const r = faceCropRect(box)!;
    // Face occupies box/window of the container on each axis.
    expect(box.w / r.zx).toBeCloseTo(FACE_TARGET_FRACTION, 6);
    expect(box.h / r.zy).toBeCloseTo(FACE_TARGET_FRACTION, 6);
  });

  it("never magnifies a tiny face past FACE_MAX_ZOOM", () => {
    const r = faceCropRect({ x: 0.45, y: 0.45, w: 0.02, h: 0.02 })!;
    // Reaching the target would need 50x; the floor caps the magnification.
    expect(Math.max(r.zx, r.zy)).toBeCloseTo(1 / FACE_MAX_ZOOM, 6);
  });

  it("never opens a window wider than the photo", () => {
    const r = faceCropRect({ x: 0.05, y: 0.05, w: 0.9, h: 0.6 })!;
    expect(r.zx).toBeLessThanOrEqual(1);
    expect(r.zy).toBeLessThanOrEqual(1);
    // The long axis is the binding constraint and should be fully used.
    expect(r.zx).toBeCloseTo(1, 6);
  });

  it("degenerates to the bbox itself at targetFraction 1 (the FaceCrop chip)", () => {
    const box = { x: 0.2, y: 0.1, w: 0.3, h: 0.4 };
    const r = faceCropRect(box, { targetFraction: 1, minVisibleFraction: 0 })!;
    expect(r.zx).toBeCloseTo(box.w, 6);
    expect(r.zy).toBeCloseTo(box.h, 6);
    // Matches the position PhotoInfoPanel's FaceCrop computed by hand.
    expect(r.px).toBeCloseTo(box.x / (1 - box.w), 6);
    expect(r.py).toBeCloseTo(box.y / (1 - box.h), 6);
  });
});

describe("computeFaceCropStyle", () => {
  it("returns {} for missing/degenerate boxes so callers fall back to cover", () => {
    expect(computeFaceCropStyle(null)).toEqual({});
    expect(computeFaceCropStyle({ x: 0, y: 0, w: 0, h: 0.2 })).toEqual({});
  });

  it.each<[string, FaceBox]>([
    ["dead centre", { x: 0.4, y: 0.4, w: 0.2, h: 0.2 }],
    ["up and to the left (#48's report)", { x: 0.25, y: 0.2, w: 0.1, h: 0.1 }],
    ["down and to the right", { x: 0.6, y: 0.65, w: 0.15, h: 0.15 }],
    ["non-square box", { x: 0.3, y: 0.3, w: 0.12, h: 0.18 }],
  ])("puts the face centre in the middle of the tile — %s", (_label, box) => {
    const style = computeFaceCropStyle(box) as Record<string, unknown>;
    const { cx, cy } = centreOf(box);
    expect(landsAt(style, "x", cx)).toBeCloseTo(0.5, 3);
    expect(landsAt(style, "y", cy)).toBeCloseTo(0.5, 3);
  });

  it("never leaves the tile showing background", () => {
    // A face in the corner cannot be centred without panning off the photo, so
    // the window clamps — but the image must still cover the tile completely.
    for (const box of [
      { x: 0, y: 0, w: 0.1, h: 0.1 },
      { x: 0.9, y: 0.9, w: 0.1, h: 0.1 },
      { x: 0.85, y: 0.02, w: 0.13, h: 0.13 },
    ]) {
      const s = computeFaceCropStyle(box) as Record<string, unknown>;
      for (const axis of ["x", "y"] as const) {
        expect(landsAt(s, axis, 0)).toBeLessThanOrEqual(0.0001);
        expect(landsAt(s, axis, 1)).toBeGreaterThanOrEqual(0.9999);
      }
    }
  });

  it("keeps a corner face fully inside the tile", () => {
    const box = { x: 0.9, y: 0.9, w: 0.1, h: 0.1 };
    const s = computeFaceCropStyle(box) as Record<string, unknown>;
    // Both edges of the face must be visible, even though it cannot be centred.
    expect(landsAt(s, "x", box.x)).toBeGreaterThanOrEqual(0);
    expect(landsAt(s, "x", box.x + box.w)).toBeLessThanOrEqual(1);
  });

  it("overrides the container's object-cover sizing", () => {
    const s = computeFaceCropStyle({ x: 0.4, y: 0.4, w: 0.2, h: 0.2 });
    expect(s.position).toBe("absolute");
    expect(s.objectFit).toBe("fill");
    expect(s.maxWidth).toBe("none");
  });
});
