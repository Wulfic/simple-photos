import { describe, expect, it } from "vitest";
import {
  CONSTRAINED_MAX_SHORT_EDGE,
  chooseDefaultRendition,
  formatRenditionLabel,
  isConstrainedNetwork,
  offerableRenditions,
  renditionsEqual,
  shouldOfferPicker,
  type Rendition,
} from "./renditionChoice";

function rung(short_edge: number, over: Partial<Rendition> = {}): Rendition {
  return {
    short_edge,
    // Landscape 16:9 unless a test says otherwise.
    width: Math.round((short_edge * 16) / 9),
    height: short_edge,
    is_source: false,
    blob_id: `b-${short_edge}`,
    codec: "h264",
    size_bytes: short_edge * 1000,
    ...over,
  };
}

describe("offerableRenditions", () => {
  it("is empty for the normal case of a video with one quality", () => {
    expect(offerableRenditions([])).toEqual([]);
    expect(offerableRenditions(undefined)).toEqual([]);
  });

  it("sorts highest first even when the server's order is not trusted", () => {
    const got = offerableRenditions([rung(1080), rung(2160, { is_source: true })]);
    expect(got.map((r) => r.short_edge)).toEqual([2160, 1080]);
  });

  it("never offers the same quality twice", () => {
    const got = offerableRenditions([rung(1080), rung(1080), rung(2160)]);
    expect(got.map((r) => r.short_edge)).toEqual([2160, 1080]);
  });
});

describe("shouldOfferPicker", () => {
  it("draws no gear icon when there is nothing to choose between", () => {
    // The overwhelmingly common case: only videos above the 1080p tier ever
    // get a second rung, so most of the library lands here.
    expect(shouldOfferPicker([])).toBe(false);
    expect(shouldOfferPicker(undefined)).toBe(false);
  });

  it("draws no gear icon for a lone source rung", () => {
    // A one-entry menu implies a choice the user does not have.
    expect(shouldOfferPicker([rung(2160, { is_source: true })])).toBe(false);
  });

  it("draws the gear icon once a real alternative exists", () => {
    expect(shouldOfferPicker([rung(2160, { is_source: true }), rung(1080)])).toBe(true);
  });
});

describe("isConstrainedNetwork", () => {
  it("is false when the browser reports nothing", () => {
    // Safari and Firefox expose no `connection` at all. Defaulting to
    // "constrained" there would cap every desktop viewer at 1080p forever.
    expect(isConstrainedNetwork(undefined)).toBe(false);
    expect(isConstrainedNetwork({})).toBe(false);
  });

  it("honours an explicit Data Saver request over everything else", () => {
    expect(isConstrainedNetwork({ saveData: true, type: "wifi", effectiveType: "4g" })).toBe(
      true,
    );
  });

  it("treats a cellular link as constrained however fast it measures", () => {
    // The issue asks for wifi-vs-cellular, not fast-vs-slow. A 5G link reports
    // effectiveType "4g", so throughput alone would never see this case.
    expect(isConstrainedNetwork({ type: "cellular", effectiveType: "4g" })).toBe(true);
  });

  it("treats a slow link as constrained when the engine hides the link type", () => {
    expect(isConstrainedNetwork({ effectiveType: "2g" })).toBe(true);
    expect(isConstrainedNetwork({ effectiveType: "3g" })).toBe(true);
    expect(isConstrainedNetwork({ effectiveType: "4g" })).toBe(false);
  });

  it("does not constrain wifi", () => {
    expect(isConstrainedNetwork({ type: "wifi", effectiveType: "4g" })).toBe(false);
  });
});

describe("chooseDefaultRendition", () => {
  const ladder4k = [rung(2160, { is_source: true }), rung(1080)];

  it("returns nothing when there are no renditions, meaning 'play the photo's own blob'", () => {
    expect(chooseDefaultRendition([])).toBeUndefined();
    expect(chooseDefaultRendition(undefined)).toBeUndefined();
  });

  it("defaults to the original on an unmetered link", () => {
    expect(chooseDefaultRendition(ladder4k, { type: "wifi" })?.short_edge).toBe(2160);
    expect(chooseDefaultRendition(ladder4k)?.short_edge).toBe(2160);
  });

  it("defaults to 1080p on cellular", () => {
    expect(chooseDefaultRendition(ladder4k, { type: "cellular" })?.short_edge).toBe(1080);
  });

  it("caps at a quality, not at one rung down", () => {
    // An 8K source's next rung down is 4K. "Lower on cellular" does not mean
    // "4K instead of 8K" to anybody, which is why the cap is absolute.
    const ladder8k = [rung(4320, { is_source: true }), rung(2160), rung(1080)];
    expect(chooseDefaultRendition(ladder8k, { saveData: true })?.short_edge).toBe(
      CONSTRAINED_MAX_SHORT_EDGE,
    );
  });

  it("falls back to the smallest rung when nothing sits under the cap", () => {
    // A 4K source whose 1080 rung has not finished encoding yet. Refusing to
    // choose would leave a metered client fetching the 4K anyway.
    const partial = [rung(2160, { is_source: true }), rung(1440)];
    expect(chooseDefaultRendition(partial, { type: "cellular" })?.short_edge).toBe(1440);
  });

  it("picks the highest rung, not the source rung, when they differ", () => {
    // `is_source` marks the original, which is normally also the largest — but
    // nothing in the schema guarantees it, and reading index 0 as "the source"
    // is the kind of assumption that survives until it doesn't.
    const odd = [rung(1080, { is_source: true }), rung(2160)];
    expect(chooseDefaultRendition(odd, { type: "wifi" })?.short_edge).toBe(2160);
  });
});

describe("renditionsEqual", () => {
  it("treats undefined and empty as the same state", () => {
    // Not pedantry. A pre-#49 server (and every row cached before the field
    // existed) yields undefined; a #49 server sends [] for the ~600 videos that
    // need no rung. Calling those different rewrites the whole library on the
    // first pass after an upgrade, and every video on every pass after that.
    expect(renditionsEqual(undefined, [])).toBe(true);
    expect(renditionsEqual([], undefined)).toBe(true);
    expect(renditionsEqual(undefined, undefined)).toBe(true);
  });

  it("sees a ladder appearing", () => {
    expect(renditionsEqual([], [rung(2160, { is_source: true }), rung(1080)])).toBe(false);
  });

  it("sees a ladder being withdrawn", () => {
    expect(renditionsEqual([rung(1080)], [])).toBe(false);
  });

  it("sees a rung's blob being replaced by a re-encode", () => {
    // `upsert_rendition` refreshes a rung in place, so a re-encode changes the
    // blob id under a short_edge that did not move. Comparing lengths or
    // short_edges alone would miss it and leave the viewer fetching bytes the
    // server has already replaced.
    expect(renditionsEqual([rung(1080)], [rung(1080, { blob_id: "b-new" })])).toBe(false);
  });

  it("sees a rung's size changing", () => {
    expect(renditionsEqual([rung(1080)], [rung(1080, { size_bytes: 99 })])).toBe(false);
  });

  it("reports no change for an identical ladder", () => {
    const ladder = [rung(2160, { is_source: true }), rung(1080)];
    expect(renditionsEqual(ladder, [rung(2160, { is_source: true }), rung(1080)])).toBe(true);
  });
});

describe("formatRenditionLabel", () => {
  it("names the original and still shows its resolution", () => {
    expect(formatRenditionLabel(rung(2160, { is_source: true }))).toBe("Original (2160p)");
  });

  it("labels a rung by its short edge", () => {
    expect(formatRenditionLabel(rung(1080))).toBe("1080p");
  });

  it("labels a portrait video by its short edge, not its height", () => {
    // 14 live videos are 1080x1920. Labelling by height would call this
    // "1920p" — a resolution that does not exist — and the whole ladder keys
    // on the short edge precisely so this case is not special.
    const portrait = rung(1080, { width: 1080, height: 1920, is_source: true });
    expect(formatRenditionLabel(portrait)).toBe("Original (1080p)");
  });
});
