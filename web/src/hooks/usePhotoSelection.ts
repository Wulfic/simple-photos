/**
 * usePhotoSelection — shared multi-select state for any photo grid.
 *
 * Mirrors the behaviour the main Gallery had inlined so selection works
 * identically across smart albums, people/pets, memories/trips, and search:
 * long-press (or the tile's select circle) enters selection mode; tapping
 * toggles; emptying the set exits selection mode.
 */
import { useState, useCallback } from "react";

export interface PhotoSelection {
  selectionMode: boolean;
  selectedIds: Set<string>;
  /** Enter selection mode seeded with one id. */
  enter: (id: string) => void;
  /** Toggle one id; exits selection mode when the set becomes empty. */
  toggle: (id: string) => void;
  /** Replace the selection with the given ids (enters mode if non-empty). */
  setAll: (ids: string[]) => void;
  /** Clear the selection and exit selection mode. */
  clear: () => void;
}

export function usePhotoSelection(): PhotoSelection {
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  const enter = useCallback((id: string) => {
    setSelectionMode(true);
    setSelectedIds(new Set([id]));
  }, []);

  const toggle = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      if (next.size === 0) setSelectionMode(false);
      return next;
    });
  }, []);

  const setAll = useCallback((ids: string[]) => {
    setSelectedIds(new Set(ids));
    setSelectionMode(ids.length > 0);
  }, []);

  const clear = useCallback(() => {
    setSelectionMode(false);
    setSelectedIds(new Set());
  }, []);

  return { selectionMode, selectedIds, enter, toggle, setAll, clear };
}
