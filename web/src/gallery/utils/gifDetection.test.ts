import { describe, it, expect } from "vitest";
import {
  isGifMime,
  isAnimatedGifThumbnail,
  needsFullGifLoad,
  mediaTypeFromMime,
  supportsInPlaceEditSave,
} from "./gifDetection";

describe("gifDetection", () => {
  it("isGifMime detects the GIF mime", () => {
    expect(isGifMime("image/gif")).toBe(true);
    expect(isGifMime("image/jpeg")).toBe(false);
    expect(isGifMime("video/mp4")).toBe(false);
  });

  it("isAnimatedGifThumbnail is true only for GIF thumbnails", () => {
    expect(isAnimatedGifThumbnail("image/gif")).toBe(true);
    expect(isAnimatedGifThumbnail("image/jpeg")).toBe(false);
    expect(isAnimatedGifThumbnail(null)).toBe(false);
    expect(isAnimatedGifThumbnail(undefined)).toBe(false);
  });

  it("needsFullGifLoad only for GIFs whose thumbnail is a static JPEG", () => {
    expect(needsFullGifLoad("gif", "image/jpeg")).toBe(true);
    expect(needsFullGifLoad("gif", "image/gif")).toBe(false); // animated thumb
    expect(needsFullGifLoad("photo", "image/jpeg")).toBe(false);
  });

  it("mediaTypeFromMime classifies each family", () => {
    expect(mediaTypeFromMime("image/gif")).toBe("gif");
    expect(mediaTypeFromMime("video/webm")).toBe("video");
    expect(mediaTypeFromMime("audio/mpeg")).toBe("audio");
    expect(mediaTypeFromMime("image/png")).toBe("photo");
  });

  // Issue #18: GIFs must not use in-place metadata "Save" — the animated-GIF
  // thumbnail can't be re-baked that way, so edits must go through Save Copy.
  describe("supportsInPlaceEditSave", () => {
    it("is false for GIFs (Save Copy only)", () => {
      expect(supportsInPlaceEditSave("gif")).toBe(false);
    });

    it("is true for photos, videos, and audio", () => {
      expect(supportsInPlaceEditSave("photo")).toBe(true);
      expect(supportsInPlaceEditSave("video")).toBe(true);
      expect(supportsInPlaceEditSave("audio")).toBe(true);
    });
  });
});
