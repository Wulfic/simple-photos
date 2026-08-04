import { describe, it, expect } from "vitest";
import { albumRemovalPrompt, secureRemovalPrompt } from "./albumRemoval";

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

describe("secureRemovalPrompt", () => {
  it("promises a return to the regular gallery when no other secure album holds it", () => {
    const p = secureRemovalPrompt(1, "Private");
    expect(p.body).toMatch(/visible in your regular gallery/i);
    expect(p.body).not.toMatch(/stay secured/i);
  });

  it("promises the OPPOSITE when another secure album still holds it", () => {
    // The Z1 behaviour change: the server drops only the membership row and the
    // photo stays hidden. Saying "returns to your gallery" here would tell the
    // user they had un-secured something they had not.
    const p = secureRemovalPrompt(1, "Private", 1);
    expect(p.body).toMatch(/stay secured/i);
    expect(p.body).toMatch(/NOT return to your regular gallery/);
  });

  it("counts the other albums so the user knows where it still lives", () => {
    expect(secureRemovalPrompt(2, "Private", 2).body).toContain("2 other secure albums");
    expect(secureRemovalPrompt(2, "Private", 1).body).toContain("1 other secure album");
  });

  it("treats 0 other albums as the default", () => {
    // Vacuity guard: if the two branches ever collapse into one, the two
    // assertions above pass while the message becomes unconditional again.
    expect(secureRemovalPrompt(1, "Private").body).toEqual(
      secureRemovalPrompt(1, "Private", 0).body,
    );
    expect(secureRemovalPrompt(1, "Private", 0).body).not.toEqual(
      secureRemovalPrompt(1, "Private", 1).body,
    );
  });
});
