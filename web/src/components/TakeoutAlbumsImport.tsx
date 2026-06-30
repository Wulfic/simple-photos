/**
 * "Recreate Google Takeout albums" control for the Import page.
 *
 * Lets the user point at their exported `Google Photos` folder; album folders
 * are matched to already-synced photos by filename and rebuilt as local
 * (encrypted) album manifests. See `utils/takeoutAlbums`.
 */
import { useEffect, useRef, useState } from "react";
import {
  parseTakeoutFolders,
  recreateTakeoutAlbums,
  type RecreateResult,
} from "../utils/takeoutAlbums";

export default function TakeoutAlbumsImport() {
  const inputRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<RecreateResult | null>(null);
  const [error, setError] = useState("");

  // `webkitdirectory`/`directory` aren't in React's JSX input typings, so set
  // them on the element directly. Widely supported folder-picker attributes.
  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.setAttribute("webkitdirectory", "");
      inputRef.current.setAttribute("directory", "");
    }
  }, []);

  async function onPick(files: FileList | null) {
    if (!files || files.length === 0) return;
    setBusy(true);
    setError("");
    setResult(null);
    try {
      const map = parseTakeoutFolders(files);
      if (map.size === 0) {
        setError(
          "No album folders found. Select the 'Google Photos' folder from your Takeout export — it contains one folder per album alongside the 'Photos from YYYY' date folders.",
        );
        return;
      }
      setResult(await recreateTakeoutAlbums(map));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to recreate albums");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card p-4 mb-4">
      <h3 className="text-sm font-semibold mb-1">Recreate Google Takeout albums</h3>
      <p className="text-xs text-fg-muted mb-3">
        Once your Takeout photos have imported and synced, select the same{" "}
        <strong>Google Photos</strong> folder here to rebuild your albums. Albums
        are matched to photos already in your library by filename — nothing is
        re-uploaded.
      </p>
      <label className="btn btn-secondary btn-md inline-block cursor-pointer">
        {busy ? "Rebuilding albums…" : "Select Takeout folder"}
        <input
          ref={inputRef}
          type="file"
          multiple
          className="hidden"
          disabled={busy}
          onChange={(e) => {
            onPick(e.target.files);
            if (inputRef.current) inputRef.current.value = "";
          }}
        />
      </label>
      {error && <p className="text-xs text-red-600 dark:text-red-400 mt-2">{error}</p>}
      {result && (
        <p className="text-xs text-fg-muted mt-2">
          Created {result.albumsCreated}, updated {result.albumsUpdated}, added{" "}
          {result.photosAdded} photo{result.photosAdded === 1 ? "" : "s"}.
          {result.albumsUnmatched > 0 &&
            ` ${result.albumsUnmatched} album folder(s) had no matching synced photos yet — finish importing + syncing, then retry.`}
        </p>
      )}
    </div>
  );
}
