import { describe, it, expect } from "vitest";
import {
  resolveAlbumDisplayName,
  sourceAlbumId,
  takeoutSettled,
} from "./takeoutAlbums";

// The mangling Google actually applies on export: "&" and "'" collapse to "_".
const MANGLED = { name: "Mum _ Dad_s 40th", title: "Mum & Dad's 40th" };

describe("resolveAlbumDisplayName", () => {
  it("shows the real title instead of the mangled folder name", () => {
    expect(resolveAlbumDisplayName(MANGLED)).toBe("Mum & Dad's 40th");
  });

  it("falls back to the folder name when the export carried no title", () => {
    // Older Takeout exports have no album metadata.json at all.
    expect(resolveAlbumDisplayName({ name: "Trip to Rome", title: null })).toBe(
      "Trip to Rome",
    );
    expect(resolveAlbumDisplayName({ name: "Trip to Rome", title: "   " })).toBe(
      "Trip to Rome",
    );
  });

  it("re-titles an album still carrying the raw folder name", () => {
    // Materialized by an earlier, title-less run — this is the fix landing.
    expect(resolveAlbumDisplayName(MANGLED, "Mum _ Dad_s 40th")).toBe(
      "Mum & Dad's 40th",
    );
  });

  it("leaves a user's own rename alone", () => {
    // Curation wins: only a name we wrote (the raw folder name) is superseded.
    expect(resolveAlbumDisplayName(MANGLED, "The 40th Party")).toBe(
      "The 40th Party",
    );
  });

  it("is stable once applied, so a re-run is a no-op", () => {
    const first = resolveAlbumDisplayName(MANGLED, "Mum _ Dad_s 40th");
    expect(resolveAlbumDisplayName(MANGLED, first)).toBe(first);
  });

  it("does not rename when there is no title to rename to", () => {
    const album = { name: "Trip to Rome", title: null };
    expect(resolveAlbumDisplayName(album, "Trip to Rome")).toBe("Trip to Rome");
  });
});

describe("takeoutSettled", () => {
  // Mirrors Android's `TakeoutSettledTest` case for case — both platforms run
  // this same pass against the same server mapping, so a rule that differs means
  // one device keeps re-uploading manifests the other considers finished.
  const result = (over: Partial<Parameters<typeof takeoutSettled>[0]> = {}) => ({
    albumsCreated: 0,
    albumsUpdated: 0,
    photosAdded: 0,
    photosUnmatched: 0,
    ...over,
  });

  it("settles when every source photo matched", () => {
    expect(
      takeoutSettled(result({ albumsCreated: 3, photosAdded: 40 }), -1),
    ).toBe(true);
  });

  it("keeps running while photos are still arriving", () => {
    expect(
      takeoutSettled(
        result({ albumsUpdated: 1, photosAdded: 5, photosUnmatched: 12 }),
        20,
      ),
    ).toBe(false);
  });

  it("keeps running when the gap shrank, even on a no-op pass", () => {
    // The gap is still closing — more photos are syncing in.
    expect(takeoutSettled(result({ photosUnmatched: 12 }), 20)).toBe(false);
  });

  it("settles on an unchanged pass that left an identical gap", () => {
    // Photos that were trashed or secured never sync, so `photosUnmatched` can
    // never reach 0 — without this the pass re-runs forever.
    expect(takeoutSettled(result({ photosUnmatched: 12 }), 12)).toBe(true);
  });

  it("does not settle on the first pass just because a gap exists", () => {
    // -1 is "no previous pass" — never let that match.
    expect(takeoutSettled(result({ photosUnmatched: 12 }), -1)).toBe(false);
  });

  it("does not settle when the gap is identical but work was done", () => {
    // Photos were added while the same number stayed unmatched: the mirror is
    // still moving, so give the next pass a chance.
    expect(
      takeoutSettled(result({ photosAdded: 3, photosUnmatched: 12 }), 12),
    ).toBe(false);
  });
});

describe("sourceAlbumId", () => {
  it("matches the shared cross-platform formula", async () => {
    // The SAME vector is pinned in server (`source_album_id_matches_the_client_formula`)
    // and Android (`AlbumDisplayNameTest`). Three codebases compute this id
    // independently, and every drift is silent: albums duplicate instead of
    // converging, and a delete tombstone stops matching so the album returns.
    // Reference: `printf 'google_takeout Trip to Rome' | sha256sum`.
    expect(await sourceAlbumId("google_takeout", "Trip to Rome")).toBe(
      "src-03c6bc29608fa7bffdbdd7b46dab34de74aa131875c032e79ab581a44a29e672",
    );
  });

  it("keys on the folder name, not the title", async () => {
    // Identity must not move when an album is retitled — otherwise every device
    // orphans its existing album and builds a duplicate.
    const byFolder = await sourceAlbumId("google_takeout", "Mum _ Dad_s 40th");
    const byTitle = await sourceAlbumId("google_takeout", "Mum & Dad's 40th");
    expect(byFolder).not.toBe(byTitle);
  });
});
