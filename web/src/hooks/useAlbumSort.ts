/**
 * useAlbumSort (#52) — per-album sort state, persisted to localStorage.
 *
 * `sort` is `null` until the user picks one, which is what tells
 * {@link useAlbumPhotos} to keep the album's intrinsic order (e.g. "Recently
 * Added"'s add-order) rather than post-sorting. `displaySort` is the concrete
 * value the header control renders — the chosen sort, or the date-desc default.
 */
import { useCallback, useEffect, useState } from "react";
import {
  DEFAULT_ALBUM_SORT,
  defaultDirFor,
  readAlbumSort,
  writeAlbumSort,
  type AlbumSort,
  type SortField,
} from "../gallery/albumSort";

export interface UseAlbumSortResult {
  /** The user's choice, or `null` (intrinsic order). Pass straight to the hook. */
  sort: AlbumSort | null;
  /** Concrete sort for the control's visual state (chosen, or the default). */
  displaySort: AlbumSort;
  /**
   * Handle a click on a field button: toggle its direction if it is already the
   * active field, otherwise switch to it in its natural starting direction.
   */
  selectField: (field: SortField) => void;
}

export function useAlbumSort(albumId: string | undefined): UseAlbumSortResult {
  const [sort, setSort] = useState<AlbumSort | null>(() =>
    albumId ? readAlbumSort(albumId) : null
  );

  // Re-read when navigating between albums (the state initialiser runs once).
  useEffect(() => {
    setSort(albumId ? readAlbumSort(albumId) : null);
  }, [albumId]);

  const selectField = useCallback(
    (field: SortField) => {
      if (!albumId) return;
      setSort((current) => {
        const base = current ?? DEFAULT_ALBUM_SORT;
        const next: AlbumSort =
          base.field === field
            ? { field, dir: base.dir === "asc" ? "desc" : "asc" }
            : { field, dir: defaultDirFor(field) };
        writeAlbumSort(albumId, next);
        return next;
      });
    },
    [albumId]
  );

  return { sort, displaySort: sort ?? DEFAULT_ALBUM_SORT, selectField };
}
