/**
 * SelectablePhotoGrid — a JustifiedGrid of AlbumTiles with built-in multi-select
 * (Add to album + Delete), matching the main Gallery's selection behaviour.
 *
 * Used by every read-only photo grid that previously rendered AlbumTile with a
 * dead `onLongPress`/`isSelectionMode={false}` (smart albums, people, pets,
 * memories, trips), so the select circle now actually works there (#2).
 */
import { useState } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import type { CachedPhoto } from "../../db";
import JustifiedGrid from "./JustifiedGrid";
import AlbumTile from "../AlbumTile";
import AddToAlbumModal from "../AddToAlbumModal";
import { getEffectiveAspectRatio } from "../../utils/thumbnailCss";
import { trashPhotos } from "../../utils/trashPhotos";
import { usePhotoSelection } from "../../hooks/usePhotoSelection";
import { getErrorMessage } from "../../utils/formatters";
import { toast } from "../../store/toast";

interface SelectablePhotoGridProps {
  photos: CachedPhoto[];
  /** albumId threaded into the viewer's location.state for back-nav context. */
  viewerAlbumId?: string;
  /** Notify the parent after items are trashed (so API-backed lists can prune
   *  their local state — live-query grids can ignore it). */
  onDeleted?: (blobIds: string[]) => void;
  /** Hide the Delete action (e.g. read-only contexts). Default: enabled. */
  allowDelete?: boolean;
}

export default function SelectablePhotoGrid({
  photos,
  viewerAlbumId,
  onDeleted,
  allowDelete = true,
}: SelectablePhotoGridProps) {
  const navigate = useAppNavigate();
  const { selectionMode, selectedIds, enter, toggle, setAll, clear: clearSelection } = usePhotoSelection();
  const [showAddToAlbum, setShowAddToAlbum] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const selectAll = () => setAll(photos.map((p) => p.blobId));
  const allSelected = photos.length > 0 && selectedIds.size === photos.length;

  async function deleteSelected() {
    if (selectedIds.size === 0 || deleting) return;
    const ids = [...selectedIds];
    if (!confirm(`Move ${ids.length} item${ids.length !== 1 ? "s" : ""} to trash? You can restore within 30 days.`)) return;
    setDeleting(true);
    try {
      await trashPhotos(ids);
      toast.success(`Moved ${ids.length} item${ids.length !== 1 ? "s" : ""} to trash`);
      onDeleted?.(ids);
      clearSelection();
    } catch (err: unknown) {
      toast.error(getErrorMessage(err));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <>
      {selectionMode && (
        <div className="flex items-center justify-between gap-3 mb-4 p-3 bg-accent-50 dark:bg-accent-900/30 rounded-lg">
          <div className="flex items-center gap-3">
            <button
              onClick={clearSelection}
              className="text-fg-muted hover:text-fg"
              aria-label="Cancel selection"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
            <span className="text-sm font-medium">{selectedIds.size} selected</span>
            <button
              onClick={allSelected ? clearSelection : selectAll}
              className="text-accent-600 dark:text-accent-400 text-sm hover:underline"
            >
              {allSelected ? "Deselect All" : "Select All"}
            </button>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowAddToAlbum(true)}
              disabled={selectedIds.size === 0}
              className="btn btn-primary btn-md inline-flex items-center gap-1.5"
              title="Add to album"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
              </svg>
              Album
            </button>
            {allowDelete && (
              <button
                onClick={deleteSelected}
                disabled={selectedIds.size === 0 || deleting}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium bg-red-600 text-white hover:bg-red-700 shadow-sm disabled:opacity-50"
              >
                {deleting ? "Deleting…" : `Delete (${selectedIds.size})`}
              </button>
            )}
          </div>
        </div>
      )}

      <JustifiedGrid
        items={photos}
        getAspectRatio={(p) => getEffectiveAspectRatio(p.width, p.height, p.cropData)}
        getKey={(p) => p.blobId}
        renderItem={(photo, idx) => (
          <AlbumTile
            photo={photo}
            isSelectionMode={selectionMode}
            isSelected={selectedIds.has(photo.blobId)}
            onClick={() => {
              if (selectionMode) {
                toggle(photo.blobId);
              } else {
                navigate(`/photo/${photo.blobId}`, {
                  state: {
                    photoIds: photos.map((p) => p.blobId),
                    currentIndex: idx,
                    albumId: viewerAlbumId,
                  },
                });
              }
            }}
            onLongPress={() => enter(photo.blobId)}
            onRemove={() => {}}
          />
        )}
      />

      {showAddToAlbum && selectedIds.size > 0 && (
        <AddToAlbumModal
          blobIds={[...selectedIds]}
          onClose={() => setShowAddToAlbum(false)}
          onAdded={(_album, count) => {
            setShowAddToAlbum(false);
            toast.success(`Added ${count} item${count !== 1 ? "s" : ""} to album`);
            clearSelection();
          }}
        />
      )}
    </>
  );
}
