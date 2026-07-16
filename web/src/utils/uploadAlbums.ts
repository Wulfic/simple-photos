/**
 * Takeout album derivation for **browser** uploads (the Import page's "Local
 * Upload" mode).
 *
 * The server-directory import reads album membership straight off the disk
 * (`server/src/import/sidecar.rs`). A browser upload can't — it only ever saw a
 * flat `FileList` — so historically any Takeout imported through the browser
 * lost 100% of its album data. Given each file's path *relative to the picked
 * folder* (`webkitRelativePath`, or a `webkitGetAsEntry` traversal for drag &
 * drop), the same rules can be applied client-side and the result sent along as
 * an upload header.
 *
 * These rules are a deliberate mirror of `crate::import::sidecar`. Keep them in
 * step — a divergence means the same Takeout export produces different albums
 * depending on which import path the user happened to use:
 *
 *  - a directory is a Takeout export only if it contains at least one `.json`
 *    (`TakeoutDirContext::is_takeout`), so a plain folder of holiday snaps never
 *    becomes a bogus album;
 *  - Google's date and container folders are never albums
 *    (`is_non_album_folder`);
 *  - the album is the file's immediate parent folder name
 *    (`derive_album_from_dir`).
 */

/** The album-level metadata file Google writes into each album folder. */
export const ALBUM_METADATA_JSON = "metadata.json";

/**
 * Google's non-album container/date folders (case-insensitive).
 * Mirrors `sidecar::is_non_album_folder`.
 */
export function isNonAlbumFolder(folder: string): boolean {
  const lower = folder.trim().toLowerCase();
  if (lower === "") return true;
  // "Photos from 2023", "Photos from 1998", … — but "Photos from Grandma" is a
  // real album, so only a 4-digit year counts.
  if (lower.startsWith("photos from ")) {
    const rest = lower.slice("photos from ".length);
    if (/^\d{4}/.test(rest)) return true;
  }
  return ["takeout", "google photos", "google fotos", "google foto's"].includes(
    lower,
  );
}

/** The directory portion of a relative path ("" for a top-level file). */
export function dirOfPath(path: string): string {
  const norm = path.replace(/\\/g, "/");
  const i = norm.lastIndexOf("/");
  return i === -1 ? "" : norm.slice(0, i);
}

/** The last path segment ("" for the root). */
function baseOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i === -1 ? path : path.slice(i + 1);
}

/**
 * Decide the Takeout album for every non-JSON file in `paths`, keyed by the
 * path itself. Paths are relative to the picked root, e.g.
 * `"Takeout/Google Photos/Trip to Rome/IMG_1.jpg"`.
 *
 * Files with no album — a plain (non-Takeout) folder, one of Google's date /
 * container folders, or a file picked individually so it has no folder at all —
 * are simply absent from the result.
 */
export function resolveUploadAlbums(paths: string[]): Map<string, string> {
  // Group by directory first: whether a folder is a Takeout export is a property
  // of the whole folder (does any sibling `.json` exist?), which a per-file pass
  // cannot see.
  const jsonDirs = new Set<string>();
  const mediaByDir = new Map<string, string[]>();
  for (const path of paths) {
    const norm = path.replace(/\\/g, "/");
    const dir = dirOfPath(norm);
    if (baseOf(norm).toLowerCase().endsWith(".json")) {
      jsonDirs.add(dir);
    } else {
      const list = mediaByDir.get(dir);
      if (list) list.push(norm);
      else mediaByDir.set(dir, [norm]);
    }
  }

  const albums = new Map<string, string>();
  for (const [dir, mediaPaths] of mediaByDir) {
    // The `is_takeout` gate — no sidecars, not a Takeout folder, not an album.
    if (!jsonDirs.has(dir)) continue;
    const folder = baseOf(dir);
    if (isNonAlbumFolder(folder)) continue;
    for (const p of mediaPaths) albums.set(p, folder);
  }
  return albums;
}

/**
 * The album title from an album folder's `metadata.json`, or null if this isn't
 * usable album metadata. Mirrors `sidecar::parse_album_title`'s guard: a
 * *per-photo* sidecar also has a `title` (the media filename), which would make
 * a terrible album name, so anything carrying photo-level fields is rejected.
 *
 * The value is not sanitised here — the server does that (it must, since it
 * can't trust any client), so there is exactly one sanitisation rule.
 */
export function parseAlbumTitle(json: unknown): string | null {
  if (typeof json !== "object" || json === null) return null;
  const meta = json as Record<string, unknown>;
  if (meta.photoTakenTime !== undefined || meta.googlePhotosOrigin !== undefined) {
    return null;
  }
  const title = meta.title;
  if (typeof title !== "string") return null;
  const trimmed = title.trim();
  return trimmed === "" ? null : trimmed;
}
