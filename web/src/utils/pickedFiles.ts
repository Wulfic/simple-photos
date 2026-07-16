/**
 * Turning a browser file pick / drop into files **with their folder paths**.
 *
 * A plain `<input type="file" multiple>` and `DataTransfer.files` both hand back
 * a flat list with no folder structure at all, which is why Takeout imported
 * through the browser used to lose 100% of its album data — the album is the
 * folder name, and the folder name was never in the payload.
 *
 * Two APIs recover it, both non-standard but universally supported:
 *  - `<input webkitdirectory>` populates `File.webkitRelativePath`;
 *  - `DataTransferItem.webkitGetAsEntry()` exposes dropped directories, which
 *    can be walked.
 *
 * The resulting relative paths feed `utils/uploadAlbums.ts`.
 */

/** A picked file plus its path relative to the picked/dropped root. */
export interface PickedFile {
  file: File;
  /** e.g. "Takeout/Google Photos/Trip to Rome/IMG_1.jpg", or just "IMG_1.jpg". */
  path: string;
}

/**
 * Files from an `<input type="file">`. `webkitRelativePath` is populated only
 * when the input had `webkitdirectory`; otherwise the path is the bare filename
 * and the file simply gets no album.
 */
export function pickedFromInput(files: FileList): PickedFile[] {
  return Array.from(files).map((file) => ({
    file,
    path: file.webkitRelativePath || file.name,
  }));
}

/**
 * Files from a drag & drop, expanding any dropped directories.
 *
 * Falls back to the flat `DataTransfer.files` when the entries API isn't
 * available — same behaviour as before this existed, just without albums.
 */
export async function pickedFromDrop(dt: DataTransfer): Promise<PickedFile[]> {
  // Capture the entries synchronously: a DataTransfer is neutered once the drop
  // handler yields, so touching dt.items after an await returns nothing.
  const entries: FileSystemEntry[] = [];
  for (const item of Array.from(dt.items ?? [])) {
    const entry = item.webkitGetAsEntry?.();
    if (entry) entries.push(entry);
  }
  if (entries.length === 0) return pickedFromInput(dt.files);

  const out: PickedFile[] = [];
  for (const entry of entries) await walkEntry(entry, "", out);
  return out;
}

/** Recursively collect an entry's files, prefixing each with its folder path. */
async function walkEntry(
  entry: FileSystemEntry,
  prefix: string,
  out: PickedFile[],
): Promise<void> {
  if (entry.isFile) {
    try {
      const file = await new Promise<File>((resolve, reject) =>
        (entry as FileSystemFileEntry).file(resolve, reject),
      );
      out.push({ file, path: prefix + entry.name });
    } catch (e) {
      // One unreadable file must not abandon the whole drop.
      console.error(`[pickedFiles] could not read dropped file "${prefix}${entry.name}"`, e); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
    }
    return;
  }
  if (!entry.isDirectory) return;

  const reader = (entry as FileSystemDirectoryEntry).createReader();
  const childPrefix = `${prefix}${entry.name}/`;
  // readEntries yields at most ~100 entries per call and signals the end with an
  // empty batch — a single call silently truncates any real Takeout folder.
  for (;;) {
    let batch: FileSystemEntry[];
    try {
      batch = await new Promise<FileSystemEntry[]>((resolve, reject) =>
        reader.readEntries(resolve, reject),
      );
    } catch (e) {
      console.error(`[pickedFiles] could not read dropped folder "${childPrefix}"`, e); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
      return;
    }
    if (batch.length === 0) return;
    for (const child of batch) await walkEntry(child, childPrefix, out);
  }
}
