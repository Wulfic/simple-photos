import { describe, it, expect } from "vitest";
import {
  isNonAlbumFolder,
  parseAlbumTitle,
  resolveUploadAlbums,
} from "./uploadAlbums";

// These cases mirror the Rust tests in `server/src/import/sidecar.rs`. The two
// implementations must agree: the same Takeout export has to produce the same
// albums whether it's imported from a server directory or through the browser.

describe("isNonAlbumFolder", () => {
  it("rejects Google's container and date folders", () => {
    expect(isNonAlbumFolder("Takeout")).toBe(true);
    expect(isNonAlbumFolder("TAKEOUT")).toBe(true);
    expect(isNonAlbumFolder("Google Photos")).toBe(true);
    expect(isNonAlbumFolder("Google Fotos")).toBe(true);
    expect(isNonAlbumFolder("Photos from 2020")).toBe(true);
    expect(isNonAlbumFolder("")).toBe(true);
  });

  it("keeps real albums", () => {
    expect(isNonAlbumFolder("Vacation")).toBe(false);
    // Only a 4-digit year is a date folder — this one is a real album.
    expect(isNonAlbumFolder("Photos from Grandma")).toBe(false);
  });
});

describe("resolveUploadAlbums", () => {
  it("assigns the parent folder as the album", () => {
    const albums = resolveUploadAlbums([
      "Takeout/Google Photos/Trip to Rome/IMG_1.jpg",
      "Takeout/Google Photos/Trip to Rome/IMG_1.jpg.supplemental-metadata.json",
    ]);
    expect(albums.get("Takeout/Google Photos/Trip to Rome/IMG_1.jpg")).toBe(
      "Trip to Rome",
    );
  });

  it("never turns a plain user folder into an album (the is_takeout gate)", () => {
    // Media but no Google sidecars — just someone's folder of photos.
    const albums = resolveUploadAlbums([
      "Vacation Photos/IMG_1.jpg",
      "Vacation Photos/IMG_2.jpg",
    ]);
    expect(albums.size).toBe(0);
  });

  it("skips Google's date and container folders", () => {
    const albums = resolveUploadAlbums([
      "Takeout/Google Photos/Photos from 2021/IMG_1.jpg",
      "Takeout/Google Photos/Photos from 2021/IMG_1.jpg.json",
      "Takeout/Google Photos/IMG_loose.jpg",
      "Takeout/Google Photos/IMG_loose.jpg.json",
    ]);
    expect(albums.size).toBe(0);
  });

  it("gives individually-picked files no album", () => {
    // No webkitRelativePath → bare filenames → no folder structure at all.
    expect(resolveUploadAlbums(["IMG_1.jpg", "IMG_1.jpg.json"]).size).toBe(0);
  });

  it("handles a photo living in several albums plus the date folder", () => {
    // Takeout duplicates the same bytes into every album AND the date folder.
    const albums = resolveUploadAlbums([
      "Takeout/Google Photos/Trip to Rome/IMG_1.jpg",
      "Takeout/Google Photos/Trip to Rome/IMG_1.jpg.json",
      "Takeout/Google Photos/Best of 2021/IMG_1.jpg",
      "Takeout/Google Photos/Best of 2021/IMG_1.jpg.json",
      "Takeout/Google Photos/Photos from 2021/IMG_1.jpg",
      "Takeout/Google Photos/Photos from 2021/IMG_1.jpg.json",
    ]);
    expect(albums.get("Takeout/Google Photos/Trip to Rome/IMG_1.jpg")).toBe(
      "Trip to Rome",
    );
    expect(albums.get("Takeout/Google Photos/Best of 2021/IMG_1.jpg")).toBe(
      "Best of 2021",
    );
    expect(
      albums.has("Takeout/Google Photos/Photos from 2021/IMG_1.jpg"),
    ).toBe(false);
    expect(albums.size).toBe(2);
  });

  it("treats an album metadata.json as evidence the folder is Takeout", () => {
    const albums = resolveUploadAlbums([
      "Takeout/Google Photos/Trip to Rome/IMG_1.jpg",
      "Takeout/Google Photos/Trip to Rome/metadata.json",
    ]);
    expect(albums.get("Takeout/Google Photos/Trip to Rome/IMG_1.jpg")).toBe(
      "Trip to Rome",
    );
  });

  it("normalises backslash separators", () => {
    const albums = resolveUploadAlbums([
      "Takeout\\Google Photos\\Trip to Rome\\IMG_1.jpg",
      "Takeout\\Google Photos\\Trip to Rome\\IMG_1.jpg.json",
    ]);
    expect(albums.get("Takeout/Google Photos/Trip to Rome/IMG_1.jpg")).toBe(
      "Trip to Rome",
    );
  });
});

describe("parseAlbumTitle", () => {
  it("reads the real title from album metadata", () => {
    expect(
      parseAlbumTitle({ title: "Mum & Dad's 40th", access: "protected" }),
    ).toBe("Mum & Dad's 40th");
  });

  it("rejects a per-photo sidecar, whose title is the media filename", () => {
    expect(
      parseAlbumTitle({
        title: "IMG_1.jpg",
        photoTakenTime: { timestamp: "1494963474" },
      }),
    ).toBeNull();
    expect(
      parseAlbumTitle({ title: "IMG_1.jpg", googlePhotosOrigin: {} }),
    ).toBeNull();
  });

  it("rejects unusable input", () => {
    expect(parseAlbumTitle(null)).toBeNull();
    expect(parseAlbumTitle("not an object")).toBeNull();
    expect(parseAlbumTitle({ description: "no title" })).toBeNull();
    expect(parseAlbumTitle({ title: "   " })).toBeNull();
  });
});
