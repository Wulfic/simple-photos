import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  albumRemovalPrompt,
  secureRemovalPrompt,
  secureRemovalToast,
  secureAlbumDeletePrompt,
  otherSecureAlbumCount,
} from "./albumRemoval";

describe("albumRemovalPrompt", () => {
  it("names the album it is removing from", () => {
    const p = albumRemovalPrompt(3, "Holiday");
    expect(p.title).toContain("3 photos");
    expect(p.title).toContain("Holiday");
  });

  it("says nothing is deleted — the whole reason the prompt exists", () => {
    // The action sits behind a TRASH icon after Z1c. Without this sentence the
    // icon change alone makes an un-filing action read as a deletion.
    const p = albumRemovalPrompt(2, "Holiday");
    expect(p.body).toMatch(/nothing is deleted/i);
    expect(p.body).toMatch(/stay in your gallery/i);
  });

  it("uses singular wording for one photo", () => {
    const one = albumRemovalPrompt(1, "Holiday");
    expect(one.title).toContain("1 photo from");
    expect(one.title).not.toContain("1 photos");
    expect(one.body).toContain("It stays");
  });

  it("falls back to a generic label with no album name", () => {
    expect(albumRemovalPrompt(1).title).toContain("this album");
  });
});

describe("otherSecureAlbumCount", () => {
  it("counts every membership except the one being removed from", () => {
    expect(
      otherSecureAlbumCount([{ id: "a" }, { id: "b" }, { id: "c" }], "a"),
    ).toBe(2);
  });

  it("returns 0 when the open album is the only one holding it", () => {
    expect(otherSecureAlbumCount([{ id: "a" }], "a")).toBe(0);
  });

  it("returns UNKNOWN — not 0 — for an absent or empty list", () => {
    // The load-bearing case, and the one the previous two-way API got wrong.
    // An empty array can only mean a server that does not publish memberships;
    // reading it as "no other album" promises the photo returns to the regular
    // gallery, which is the privacy-shaped lie this whole item exists to kill.
    expect(otherSecureAlbumCount([], "a")).toBeUndefined();
    expect(otherSecureAlbumCount(undefined, "a")).toBeUndefined();
    expect(otherSecureAlbumCount(null, "a")).toBeUndefined();
  });

  it("returns UNKNOWN when the owning album is missing from the list", () => {
    // Not off-by-one: the owner is the one membership that MUST be present, so
    // its absence means the array is not what we think it is. Counting it as an
    // "other" would turn a last-membership removal into the "stays secured"
    // branch — wrong in the direction that surprises the user.
    expect(otherSecureAlbumCount([{ id: "b" }, { id: "c" }], "a")).toBeUndefined();
  });
});

describe("secureRemovalPrompt", () => {
  it("promises a return to the regular gallery when no other secure album holds it", () => {
    const p = secureRemovalPrompt([0], "Private");
    expect(p.kind).toBe("confirm");
    expect(p.title).toContain("1 photo from");
    expect(p.body).toMatch(/visible in your regular gallery/i);
    expect(p.body).not.toMatch(/stay secured/i);
  });

  it("promises the OPPOSITE when another secure album still holds it", () => {
    // The Z1 behaviour change: the server drops only the membership row and the
    // photo stays hidden. Saying "returns to your gallery" here would tell the
    // user they had un-secured something they had not.
    const p = secureRemovalPrompt([1], "Private");
    expect(p.kind).toBe("confirm");
    expect(p.body).toMatch(/stay secured/i);
    expect(p.body).toMatch(/NOT return to your regular gallery/);
  });

  it("counts the other albums so the user knows where a single photo still lives", () => {
    expect(secureRemovalPrompt([2], "Private").body).toContain("2 other secure albums");
    expect(secureRemovalPrompt([1], "Private").body).toContain("1 other secure album");
  });

  it("REFUSES rather than guessing when membership is unknown", () => {
    // Replaces the old "treats 0 other albums as the default" test, which
    // asserted the defect: it pinned an omitted argument as equal to the 0 case.
    const p = secureRemovalPrompt([undefined], "Private");
    expect(p.kind).toBe("blocked");
    expect(p.body).not.toMatch(/will be unsecured/i);
    expect(p.body).not.toMatch(/will stay secured/i);
    expect(p.body).toMatch(/refresh/i);
  });

  it("keeps the three verdicts genuinely distinct", () => {
    // Vacuity guard. Every assertion above still passes if two branches collapse
    // into one another; this is what notices.
    const none = secureRemovalPrompt([0], "Private");
    const some = secureRemovalPrompt([1], "Private");
    const unknown = secureRemovalPrompt([undefined], "Private");
    expect(new Set([none.body, some.body, unknown.body]).size).toBe(3);
    expect(new Set([none.kind, some.kind, unknown.kind]).size).toBe(2);
  });

  // ── Batches (TODO.md S3) — the cases the scalar form could not express ──

  it("one unknown member blocks the whole batch", () => {
    expect(secureRemovalPrompt([0, 0, undefined], "Private").kind).toBe("blocked");
  });

  it("an empty list is unknown, not 'nothing to worry about'", () => {
    expect(secureRemovalPrompt([], "Private").kind).toBe("blocked");
  });

  it("a mixed batch says how many go each way", () => {
    const p = secureRemovalPrompt([0, 2, 0], "Private");
    expect(p.kind).toBe("confirm");
    expect(p.title).toContain("3 photos");
    expect(p.body).toContain("2 will return to your regular gallery");
    expect(p.body).toContain("other 1 will stay secured");
  });

  it("a batch that all stays secured says so without inventing a count", () => {
    const p = secureRemovalPrompt([1, 3], "Private");
    expect(p.body).toMatch(/each is also in another secure album/);
    expect(p.body).toMatch(/NOT return/);
  });

  it("names the generic album when the batch spans albums", () => {
    expect(secureRemovalPrompt([0, 0], undefined).title).toContain("this secure album");
  });
});

describe("secureAlbumDeletePrompt (S4)", () => {
  it("says exactly what Android says — the two strings are asserted verbatim on both sides", () => {
    const p = secureAlbumDeletePrompt("Private");
    expect(p.title).toBe("Delete secure album “Private”?");
    expect(p.body).toBe(
      "Every photo in this album will be removed from it. Photos that are also in " +
        "another secure album stay secured there; the rest return to your regular " +
        "gallery. The album itself is deleted.",
    );
  });
});

describe("secureRemovalToast", () => {
  it("reports what actually happened, from the same counts as the prompt", () => {
    expect(secureRemovalToast([0])).toBe("Photo returned to your gallery.");
    expect(secureRemovalToast([0, 0])).toBe("2 photos returned to your gallery.");
    expect(secureRemovalToast([1])).toMatch(/stays secured/);
    expect(secureRemovalToast([1, 2])).toMatch(/They stay secured/);
    expect(secureRemovalToast([0, 1, 0])).toBe(
      "Removed 3. 2 returned to your gallery; 1 stay secured in other secure albums.",
    );
  });

  it("never claims a return for a photo that stayed secured (the Z1d toast bug)", () => {
    expect(secureRemovalToast([3])).not.toMatch(/returned to your gallery/);
  });
});

/**
 * The wiring guard.
 *
 * This is the test that would have caught Z1 shipping half-done. The helper
 * above was written, fully unit-tested, documented at length — and called by
 * NOTHING, while the page it was written for kept a `window.confirm` containing
 * the exact sentence it exists to prevent. Every test in this file was green
 * over a UI that lied. **A tested helper with no call site is worse than no
 * helper**, because the green suite is what stops anyone looking.
 *
 * There is no jsdom in this repo, so the dialog cannot be rendered and asserted.
 * What IS checkable is that the call site exists and that the false sentence is
 * gone — read from source with `node:fs`, following `safeArea.test.ts`, and for
 * the same reason: the failure mode is invisible to every other kind of test.
 */
describe("the secure removal prompt is actually wired up", () => {
  const HERE = dirname(fileURLToPath(import.meta.url));
  const secureGallery = readFileSync(
    join(resolve(HERE, ".."), "pages", "SecureGallery.tsx"),
    "utf8",
  );

  it("reads a non-empty source file", () => {
    // Vacuity guard with teeth: a bad path or a moved file would make every
    // assertion below pass against an empty string.
    expect(secureGallery.length).toBeGreaterThan(1000);
    expect(secureGallery).toContain("export default function SecureGallery");
  });

  // Since F4 the grid and the viewer share one plan → execute → report pipeline
  // in `secureRemoval.ts`, so the calls below are asserted where they now live:
  // the page must route through the pipeline, and the pipeline must call the
  // prompt/membership/expansion/toast helpers. Its behaviour is pinned by
  // `secureRemoval.test.ts`; these keep the wiring from silently going dead.
  const pipeline = readFileSync(join(HERE, "secureRemoval.ts"), "utf8");

  it("routes removal through the shared pipeline", () => {
    expect(pipeline.length).toBeGreaterThan(1000);
    expect(secureGallery).toContain("planSecureRemoval(");
    expect(secureGallery).toContain("executeSecureRemoval(");
  });

  it("calls secureRemovalPrompt", () => {
    expect(pipeline).toContain("secureRemovalPrompt(");
  });

  it("resolves membership through otherSecureAlbumCount", () => {
    // Without this the call site can pass a hardcoded 0 and re-create the bug
    // while satisfying the assertion above.
    expect(pipeline).toContain("otherSecureAlbumCount(");
  });

  it("no longer promises a return to the regular gallery in the UI text", () => {
    // The three places that carried the unconditional claim: the confirm(), the
    // success toast, and the tile tooltip.
    expect(secureGallery).not.toMatch(/It will return to your regular gallery/);
    expect(secureGallery).not.toMatch(/returns to regular gallery/);
  });

  it("expands bursts before asking, so the prompt counts what is removed (S2)", () => {
    expect(pipeline).toContain("expandBurstStacks(");
  });

  it("derives the success toast from the prompt's own counts", () => {
    expect(pipeline).toContain("secureRemovalToast(");
    expect(secureGallery).toContain("secureRemovalOutcome(");
  });

  it("offers Remove on a multi-select, not only one tile at a time (S3)", () => {
    // The selection bar is the block between the push-selection comment and the
    // move-from picker; it must route the selected tiles into the same request.
    const bar = secureGallery.slice(
      secureGallery.indexOf("{pushSelect.selectionMode && ("),
      secureGallery.indexOf("Cross-secure-album move picker"),
    );
    expect(bar.length).toBeGreaterThan(200);
    expect(bar).toContain("requestRemoval(");
  });

  it("does not hide Select behind having a second album to add to", () => {
    // Selecting to REMOVE needs no other album; gating Select on moveTargets
    // made multi-select removal unreachable with a single secure album.
    const header = secureGallery.slice(
      secureGallery.indexOf("onClick={() => pushSelect.enterEmpty()}") - 300,
      secureGallery.indexOf("onClick={() => pushSelect.enterEmpty()}"),
    );
    expect(header).not.toContain("moveTargets.length > 0");
  });

  it("the pull picker draws one tile per stack (S5)", () => {
    // resolveSecureMoves expands a picked tile to its stack; drawing every frame
    // instead shows one burst as N tiles, unlike every other secure grid and
    // unlike Android's picker.
    expect(secureGallery).toContain("collapseBurstStacks(otherSecureItems)");
    expect(secureGallery).toContain("otherSecureTiles.map(");
  });

  it("a pull-picker tile toggles its selection exactly once per click", () => {
    // Found in a real browser (2026-11-24): the wrapper AND the tile both called
    // toggleMoveSelect, the tile's click bubbled into the wrapper, and every
    // click selected-then-deselected — "Move here (0)" forever, so nothing could
    // ever be pulled in from another secure album on web.
    const start = secureGallery.indexOf("otherSecureTiles.map(");
    const tile = secureGallery.slice(start, secureGallery.indexOf("{selected && (", start));
    expect(start).toBeGreaterThan(0);
    expect(tile.match(/toggleMoveSelect\(it\.id\)/g) ?? []).toHaveLength(1);
  });

  it("does not ask with window.confirm", () => {
    // `confirm()` cannot render a conditional body, cannot be themed, and is
    // suppressible by the browser — see the ConfirmDialog module doc.
    expect(secureGallery).not.toMatch(/(?<![.\w])confirm\(["'`]/);
  });
});
