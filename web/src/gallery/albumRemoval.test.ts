import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  albumRemovalPrompt,
  secureRemovalPrompt,
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
    const p = secureRemovalPrompt(1, "Private", 0);
    expect(p.kind).toBe("confirm");
    expect(p.body).toMatch(/visible in your regular gallery/i);
    expect(p.body).not.toMatch(/stay secured/i);
  });

  it("promises the OPPOSITE when another secure album still holds it", () => {
    // The Z1 behaviour change: the server drops only the membership row and the
    // photo stays hidden. Saying "returns to your gallery" here would tell the
    // user they had un-secured something they had not.
    const p = secureRemovalPrompt(1, "Private", 1);
    expect(p.kind).toBe("confirm");
    expect(p.body).toMatch(/stay secured/i);
    expect(p.body).toMatch(/NOT return to your regular gallery/);
  });

  it("counts the other albums so the user knows where it still lives", () => {
    expect(secureRemovalPrompt(2, "Private", 2).body).toContain("2 other secure albums");
    expect(secureRemovalPrompt(2, "Private", 1).body).toContain("1 other secure album");
  });

  it("REFUSES rather than guessing when membership is unknown", () => {
    // Replaces the old "treats 0 other albums as the default" test, which
    // asserted the defect: it pinned `secureRemovalPrompt(1, "Private")` as
    // equal to the 0 case, i.e. it required that an omitted argument promise a
    // return to the regular gallery. Same shape as the #48 face-centering suite
    // and B3b's rung_queue test — a test that locks in the bug.
    const p = secureRemovalPrompt(1, "Private", undefined);
    expect(p.kind).toBe("blocked");
    expect(p.body).not.toMatch(/will be unsecured/i);
    expect(p.body).not.toMatch(/will stay secured/i);
    expect(p.body).toMatch(/refresh/i);
  });

  it("keeps the three verdicts genuinely distinct", () => {
    // Vacuity guard. Every assertion above still passes if two branches collapse
    // into one another; this is what notices.
    const none = secureRemovalPrompt(1, "Private", 0);
    const some = secureRemovalPrompt(1, "Private", 1);
    const unknown = secureRemovalPrompt(1, "Private", undefined);
    expect(new Set([none.body, some.body, unknown.body]).size).toBe(3);
    expect(new Set([none.kind, some.kind, unknown.kind]).size).toBe(2);
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

  it("calls secureRemovalPrompt", () => {
    expect(secureGallery).toContain("secureRemovalPrompt(");
  });

  it("resolves membership through otherSecureAlbumCount", () => {
    // Without this the call site can pass a hardcoded 0 and re-create the bug
    // while satisfying the assertion above.
    expect(secureGallery).toContain("otherSecureAlbumCount(");
  });

  it("no longer promises a return to the regular gallery in the UI text", () => {
    // The three places that carried the unconditional claim: the confirm(), the
    // success toast, and the tile tooltip.
    expect(secureGallery).not.toMatch(/It will return to your regular gallery/);
    expect(secureGallery).not.toMatch(/returns to regular gallery/);
  });

  it("does not ask with window.confirm", () => {
    // `confirm()` cannot render a conditional body, cannot be themed, and is
    // suppressible by the browser — see the ConfirmDialog module doc.
    expect(secureGallery).not.toMatch(/(?<![.\w])confirm\(["'`]/);
  });
});
