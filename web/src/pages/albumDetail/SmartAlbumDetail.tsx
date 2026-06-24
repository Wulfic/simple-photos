/**
 * SmartAlbumDetail — shared detail view for a single cluster of the
 * auto-generated smart albums (one trip / memory / pet / person). Each of
 * those used to repeat the same scaffold: AppHeader, a back header with title +
 * photo count + slideshow triggers, a loading skeleton, an empty state, the
 * SelectablePhotoGrid, and the SlideshowHost overlay. They differ only in how
 * they load their title + photos and whether the cluster can be renamed.
 */
import { useEffect, useRef, useState } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import { type CachedPhoto } from "../../db";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import AppIcon from "../../components/AppIcon";
import SelectablePhotoGrid from "../../components/gallery/SelectablePhotoGrid";
import { usePhotoSlideshow } from "../../hooks/useSlideshow";
import SlideshowHost from "../../components/viewer/SlideshowHost";
import SlideshowTriggers from "../../components/viewer/SlideshowTriggers";

/**
 * Setters the loader uses to publish header state as soon as it is known —
 * the title typically resolves from a fast list call before the (slower)
 * per-photo lookup completes, so it is surfaced incrementally just like the
 * original hand-rolled views did.
 */
export interface SmartAlbumLoadContext {
  setTitle: (title: string) => void;
  /** Seed the rename input with the cluster's current (raw) label. */
  setRenameValue: (value: string) => void;
}

interface SmartAlbumDetailProps {
  /** Re-runs the loader whenever this changes (the cluster id). */
  reloadKey: string | number;
  /** Title shown until the loader resolves the real one. */
  defaultTitle: string;
  /** Extra classes for the title (e.g. "capitalize" for pets). */
  titleClassName?: string;
  backTo: string;
  /** Back button tooltip reads `Back to {backLabel}`. */
  backLabel: string;
  viewerAlbumId: string;
  emptyMessage: string;
  /** Load the cluster's photos; publish the title via `ctx` once known. */
  load: (ctx: SmartAlbumLoadContext) => Promise<CachedPhoto[]>;
  /** Enables the inline rename UI. Returns the new display title. */
  onRename?: (value: string) => Promise<string>;
}

export default function SmartAlbumDetail({
  reloadKey,
  defaultTitle,
  titleClassName,
  backTo,
  backLabel,
  viewerAlbumId,
  emptyMessage,
  load,
  onRename,
}: SmartAlbumDetailProps) {
  const navigate = useAppNavigate();
  const [title, setTitle] = useState(defaultTitle);
  const [photos, setPhotos] = useState<CachedPhoto[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [nameInput, setNameInput] = useState("");

  // Ref so an inline `load` closure doesn't need to be an effect dependency.
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const found = await loadRef.current({
          setTitle: (t) => { if (!cancelled) setTitle(t); },
          setRenameValue: (v) => { if (!cancelled) setNameInput(v); },
        });
        if (!cancelled) setPhotos(found);
      } catch { /* cluster may not exist */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [reloadKey]);

  const slideshow = usePhotoSlideshow(photos);

  async function saveName() {
    if (!onRename) return;
    try {
      const newTitle = await onRename(nameInput.trim());
      setTitle(newTitle);
      setEditing(false);
    } catch { /* ignore */ }
  }

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />
      <main className="p-4">
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => navigate(backTo)}
            className="text-fg-muted hover:text-fg transition-colors shrink-0"
            title={`Back to ${backLabel}`}
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          {onRename && editing ? (
            <form onSubmit={(e) => { e.preventDefault(); saveName(); }} className="flex items-center gap-2">
              <input
                type="text"
                value={nameInput}
                onChange={(e) => setNameInput(e.target.value)}
                className="input w-auto text-lg font-semibold py-1"
                autoFocus
                maxLength={100}
              />
              <button type="submit" className="text-accent-600 text-sm font-medium">Save</button>
              <button type="button" onClick={() => setEditing(false)} className="text-fg-muted text-sm">Cancel</button>
            </form>
          ) : (
            <>
              <h2 className={`text-xl font-semibold truncate ${titleClassName ?? ""}`}>{title}</h2>
              {onRename && (
                <button
                  onClick={() => setEditing(true)}
                  className="text-fg-muted hover:text-fg text-sm"
                  title="Rename"
                >
                  ✏️
                </button>
              )}
            </>
          )}
          <span className="text-fg-muted text-sm shrink-0">{photos.length} photos</span>
          <SlideshowTriggers slideshow={slideshow} />
        </div>

        {loading ? (
          <GallerySkeleton />
        ) : photos.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">{emptyMessage}</p>
          </div>
        ) : (
          <SelectablePhotoGrid
            photos={photos}
            viewerAlbumId={viewerAlbumId}
            onDeleted={(ids) => setPhotos((prev) => prev.filter((p) => !ids.includes(p.blobId)))}
          />
        )}
      </main>

      <SlideshowHost slideshow={slideshow} />
    </div>
  );
}
