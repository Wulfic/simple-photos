/**
 * Playback-quality selection for the viewer's video player (#49).
 *
 * Owns three things the picker cannot be correct without: which rung is
 * selected, the object URL for a non-source rung, and the lifetime of that URL.
 *
 * ## Three traps this hook exists to avoid
 *
 * **1. The source rung is not a separate download.** `is_source` points at the
 * blob the *photo already owns* — a second reference, not a copy (that is why
 * `037` had to stop the orphan trigger queueing it). So selecting "Original"
 * means "use the `mediaUrl` the viewer already loaded", and this hook fetches
 * nothing. Downloading it again would re-pull a full 4K video the browser has
 * in hand.
 *
 * **2. Rendition bytes must never enter `db.fullPhotos` or the preload cache.**
 * Both are keyed by the *route* blob id — the original. Writing a 1080p
 * rendition under that key means the next open of that video silently serves
 * the downscale as the original, and "Original" in the picker would then play
 * the rendition. Downloads here therefore bypass `loadEncryptedMedia` entirely
 * rather than reusing it with a different id; that reuse is the bug.
 *
 * **3. This hook revokes only URLs it minted.** `mediaUrl` is owned by the
 * preload cache and revoking it from here would blank the video on the next
 * swipe back — the same two-owners-of-one-blob-URL defect that made the
 * thumbnail cache bug permanent in #51. The invariant is simply that
 * `renditionUrl` is the only URL this file ever passes to `revokeObjectURL`.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/client";
import { decryptPhotoBlobToBlob } from "../crypto/blobEnvelope";
import { diagnosticLogger } from "../utils/diagnosticLogger";
import {
  chooseDefaultRendition,
  offerableRenditions,
  readNetworkHint,
  shouldOfferPicker,
  type Rendition,
} from "../gallery/renditionChoice";

interface UseVideoRenditionArgs {
  /** The ladder from the local mirror. Undefined/empty ⇒ no picker. */
  renditions: Rendition[] | undefined;
  /** Route id of the photo being viewed — changing it resets everything. */
  photoId: string | undefined;
  /** The player element, so a quality switch can preserve the playhead. */
  videoRef: React.RefObject<HTMLVideoElement | null>;
  /**
   * False while the viewer is in edit mode.
   *
   * Editing must operate on the original: a crop/trim saved while a 1080p rung
   * is on screen would re-encode the *downscale* over the user's 4K master.
   * Disabling forces the selection back to source rather than merely hiding the
   * gear icon, because hiding a control does not change what `mediaUrl` points
   * at.
   */
  enabled: boolean;
}

interface UseVideoRenditionResult {
  /** Qualities to draw, highest first. Empty when no picker should exist. */
  available: Rendition[];
  /** Whether the gear icon should be rendered at all. */
  hasPicker: boolean;
  /** Currently selected rung, or undefined while playing the photo's own blob. */
  selected: Rendition | undefined;
  /** Object URL for the selected rung; null means "use `mediaUrl`". */
  renditionUrl: string | null;
  /** True while a switch is downloading and decrypting. */
  switching: boolean;
  /** Pick a rung. Passing a source rung reverts to the photo's own blob. */
  select: (r: Rendition) => void;
  /**
   * Call from the player's `onLoadedMetadata`. Restores the playhead and
   * play/pause state captured when the switch began, so changing quality does
   * not restart the video.
   */
  handleLoadedMetadata: () => void;
}

/** Playback state carried across a source swap. */
interface Resume {
  time: number;
  playing: boolean;
}

export default function useVideoRendition({
  renditions,
  photoId,
  videoRef,
  enabled,
}: UseVideoRenditionArgs): UseVideoRenditionResult {
  const [selected, setSelected] = useState<Rendition | undefined>(undefined);
  const [renditionUrl, setRenditionUrl] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);

  // The one URL this hook owns. Kept in a ref as well as state so cleanup can
  // revoke it without depending on a stale closure.
  const ownedUrlRef = useRef<string | null>(null);
  const resumeRef = useRef<Resume | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  // Only rungs this viewer can actually fetch.
  //
  // A null `blob_id` means an unencrypted install, where the bytes live behind
  // `/photos/:id/file?rendition=`. This viewer has no plaintext path at all —
  // it loads every photo through `loadEncryptedMedia` — so offering such a rung
  // would put a menu entry on screen that silently does nothing when clicked.
  // Dropping it makes the picker disappear entirely on a plaintext install,
  // which is honest: the feature is genuinely absent there until the viewer
  // grows a plaintext branch.
  const available = useMemo(
    () => offerableRenditions(renditions).filter((r) => r.blob_id !== null),
    [renditions],
  );
  const hasPicker = enabled && shouldOfferPicker(available);

  /** Drop the URL this hook minted, if any. Never touches `mediaUrl`. */
  const releaseOwnedUrl = useCallback(() => {
    if (ownedUrlRef.current) {
      URL.revokeObjectURL(ownedUrlRef.current);
      ownedUrlRef.current = null;
    }
  }, []);

  // ── Reset on navigation ──────────────────────────────────────────────────
  // A new photo has a different ladder, and the previous rendition's bytes are
  // no longer addressable by anything on screen.
  useEffect(() => {
    abortRef.current?.abort();
    releaseOwnedUrl();
    setRenditionUrl(null);
    setSelected(undefined);
    setSwitching(false);
    resumeRef.current = null;
  }, [photoId, releaseOwnedUrl]);

  // Unmount: the effect above does not run on teardown, so the last URL would
  // otherwise leak a whole video's worth of memory per viewer session.
  useEffect(
    () => () => {
      abortRef.current?.abort();
      releaseOwnedUrl();
    },
    [releaseOwnedUrl],
  );

  // ── Edit mode forces the original ────────────────────────────────────────
  useEffect(() => {
    if (enabled) return;
    abortRef.current?.abort();
    releaseOwnedUrl();
    setRenditionUrl(null);
    setSelected(undefined);
    setSwitching(false);
  }, [enabled, releaseOwnedUrl]);

  /**
   * Capture the playhead before the `<video>` src changes.
   *
   * Read eagerly rather than in `handleLoadedMetadata`, because by then the
   * element has already been pointed at the new source and reports 0.
   */
  const captureResume = useCallback(() => {
    const video = videoRef.current;
    resumeRef.current = video
      ? { time: video.currentTime, playing: !video.paused }
      : null;
  }, [videoRef]);

  const select = useCallback(
    (target: Rendition) => {
      if (!enabled) return;
      if (selected?.short_edge === target.short_edge) return;
      // Already playing the photo's own blob, and the source rung IS that blob.
      // Without this, re-picking "Original" leaves a stale resume snapshot that
      // no `loadedmetadata` will ever consume, because the src does not change.
      if (!selected && target.is_source) return;

      captureResume();
      abortRef.current?.abort();

      // The source rung is the photo's own blob — the viewer already holds a
      // decrypted URL for it. Switching back is therefore a state change, not
      // a download. See the header note on why re-fetching it would be wrong.
      if (target.is_source || !target.blob_id) {
        releaseOwnedUrl();
        setRenditionUrl(null);
        setSelected(undefined);
        setSwitching(false);
        return;
      }

      const controller = new AbortController();
      abortRef.current = controller;
      setSwitching(true);
      setSelected(target);

      void (async () => {
        try {
          const encrypted = await api.blobs.download(target.blob_id!, controller.signal);
          if (controller.signal.aborted) return;
          const { blob } = await decryptPhotoBlobToBlob(encrypted);
          if (controller.signal.aborted) return;

          const url = URL.createObjectURL(blob);
          // Release the *previous* rendition only once the new one exists, so a
          // failed switch never leaves the player with nothing to play.
          releaseOwnedUrl();
          ownedUrlRef.current = url;
          setRenditionUrl(url);

          // Deliberately NOT cached in `db.fullPhotos`: that table is keyed by
          // the photo's own blob id, so a rendition stored there would be
          // replayed as the original on the next open.
        } catch (err) {
          if (err instanceof DOMException && err.name === "AbortError") return;
          // Every failure path logs — a silent revert to the original looks
          // exactly like the picker being ignored, which is unreportable.
          diagnosticLogger.error(
            "VIEWER",
            `Failed to load ${target.short_edge}p rendition for photo ${photoId}`,
            { error: err instanceof Error ? err.message : String(err) },
          );
          releaseOwnedUrl();
          setRenditionUrl(null);
          setSelected(undefined);
        } finally {
          if (!controller.signal.aborted) setSwitching(false);
        }
      })();
    },
    [enabled, selected, captureResume, releaseOwnedUrl, photoId],
  );

  // ── Default selection on a metered link ──────────────────────────────────
  // Runs once per photo, and only when a genuine choice exists. On an
  // unmetered link the default is the source rung, which is what the viewer
  // already loaded — so the common case costs nothing.
  const defaultedFor = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!enabled || !photoId || defaultedFor.current === photoId) return;
    // `available`, not `renditions` — defaulting to a rung the picker refuses
    // to show would leave the menu ticking nothing and no way back.
    if (!shouldOfferPicker(available)) return;
    defaultedFor.current = photoId;

    const preferred = chooseDefaultRendition(available, readNetworkHint());
    // A source default costs nothing: it is the blob the viewer already loaded.
    if (preferred && !preferred.is_source) select(preferred);
  }, [enabled, photoId, available, select]);

  const handleLoadedMetadata = useCallback(() => {
    const video = videoRef.current;
    const resume = resumeRef.current;
    if (!video || !resume) return;
    resumeRef.current = null;

    // A rendition has the same duration as its source, so the playhead
    // transfers directly. Clamped anyway: a salvage re-encode of a corrupt
    // source (#46) is legitimately shorter than the original.
    if (resume.time > 0 && Number.isFinite(video.duration)) {
      video.currentTime = Math.min(resume.time, Math.max(video.duration - 0.1, 0));
    }
    // Restore BOTH directions. The player carries `autoPlay`, so a new src
    // starts playing on its own — meaning a video the user had paused would
    // silently resume on every quality change unless we pause it back.
    if (resume.playing) void video.play().catch(() => { /* autoplay policy */ });
    else video.pause();
  }, [videoRef]);

  return {
    available,
    hasPicker,
    selected,
    renditionUrl,
    switching,
    select,
    handleLoadedMetadata,
  };
}
