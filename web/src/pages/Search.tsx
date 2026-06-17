/**
 * Search page — tag-based and text search across the encrypted photo library
 * (local IndexedDB) and server, with unified results.
 */
import { useState, useEffect, useRef, useCallback } from "react";
import { useAppNavigate } from "../hooks/useAppNavigate";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import JustifiedGrid from "../components/gallery/JustifiedGrid";
import AddToAlbumModal from "../components/AddToAlbumModal";
import { usePhotoSelection } from "../hooks/usePhotoSelection";
import { trashPhotos } from "../utils/trashPhotos";
import { getErrorMessage } from "../utils/formatters";
import { toast } from "../store/toast";
import { GallerySkeleton } from "../components/skeletons";

// ── Types ────────────────────────────────────────────────────────────────────

interface SearchResult {
  id: string;
  filename: string;
  media_type: string;
  mime_type: string;
  thumb_path: string | null;
  created_at: string;
  taken_at: string | null;
  latitude: number | null;
  longitude: number | null;
  width: number | null;
  height: number | null;
  tags: string[];
  /** For encrypted results, a local object URL for the thumbnail */
  _localThumbUrl?: string;
}

/**
 * True when two strings are within Levenshtein edit distance 1 (a single
 * substitution, insertion, or deletion). O(n) single pass — no DP matrix.
 *
 * Used to gate typo tolerance in search. The previous inline check only
 * compared index-aligned characters (a Hamming distance), which both
 * mis-handled insert/delete typos AND fired on any same-length single
 * substitution regardless of word length — that is what made "house" match
 * "horse" and "mouse".
 */
function withinEditDistance1(a: string, b: string): boolean {
  if (a === b) return true;
  const la = a.length;
  const lb = b.length;
  if (Math.abs(la - lb) > 1) return false;

  let i = 0;
  let j = 0;
  let edits = 0;
  while (i < la && j < lb) {
    if (a[i] === b[j]) {
      i++;
      j++;
      continue;
    }
    if (++edits > 1) return false;
    if (la > lb) i++; // deletion from a
    else if (lb > la) j++; // insertion into a
    else { i++; j++; } // substitution
  }
  // A leftover trailing character in either string is one more edit.
  if (i < la || j < lb) edits++;
  return edits <= 1;
}

// ── Search Page ──────────────────────────────────────────────────────────────

export default function Search() {
  const navigate = useAppNavigate();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [searchError, setSearchError] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // ── Multi-select (parity with the main gallery; #2) ─────────────────────
  const { selectionMode, selectedIds, enter, toggle, setAll, clear: clearSelection } = usePhotoSelection();
  const [showAddToAlbum, setShowAddToAlbum] = useState(false);
  const [deleting, setDeleting] = useState(false);

  // Auto-focus the search input
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // ── Fuzzy matching helper ──────────────────────────────────────────────
  // Tokenizes the query, stems each token, and checks if all tokens match
  // somewhere in the searchable text (filename, media type, date, etc.)
  const fuzzyMatch = useCallback((searchText: string, queryStr: string): boolean => {
    const lower = searchText.toLowerCase();
    const tokens = queryStr.toLowerCase().split(/\s+/).filter(Boolean);
    if (tokens.length === 0) return false;

    return tokens.every((token) => {
      // Direct substring match
      if (lower.includes(token)) return true;

      // Basic stemming variants
      const variants: string[] = [];
      if (token.length > 4) {
        if (token.endsWith("ing")) {
          const stem = token.slice(0, -3);
          variants.push(stem);
          // e.g. "running" -> "run"
          if (stem.length > 2 && stem[stem.length - 1] === stem[stem.length - 2]) {
            variants.push(stem.slice(0, -1));
          }
        }
        if (token.endsWith("ed")) variants.push(token.slice(0, -2));
        if (token.endsWith("es")) variants.push(token.slice(0, -2));
        else if (token.endsWith("s")) variants.push(token.slice(0, -1));
        if (token.endsWith("ly")) variants.push(token.slice(0, -2));
      }

      // Check variants
      if (variants.some((v) => lower.includes(v))) return true;

      // Per-word matching against the searchable text.
      const words = lower.split(/[\s_\-./]+/).filter(Boolean);
      return words.some((word) => {
        if (word.length === 0 || token.length === 0) return false;
        // Prefix match (e.g. "house" → "household").
        if (token.length >= 3 && word.startsWith(token)) return true;
        // Typo tolerance (edit distance ≤ 1) — deliberately gated hard.
        // The old code allowed a single substitution on words of ANY length,
        // so 5-letter queries matched all their neighbours: "house" pulled in
        // "horse" and "mouse". Now we only fuzzy-match when the token is long
        // enough that a 1-char change is unlikely to be a different real word
        // (≥ 7 chars) AND the first character matches (kills "house"→"mouse").
        if (token.length < 7) return false;
        if (word[0] !== token[0]) return false;
        return withinEditDistance1(word, token);
      });
    });
  }, []);

  const doSearch = useCallback(async (q: string) => {
    const trimmed = q.trim();
    if (!trimmed) {
      setResults([]);
      setSearched(false);
      return;
    }
    setLoading(true);
    setSearched(true);
    setSearchError("");
    try {
      // Search server-side photos
      let serverSearchFailed = false;
      const serverPromise = api.search.query(trimmed).catch((err) => {
        console.warn("Server search failed:", err);
        serverSearchFailed = true;
        return { results: [] as SearchResult[] };
      });

      // Search local encrypted photos in IndexedDB
      const localPromise = (async (): Promise<SearchResult[]> => {
        try {
          const allPhotos = await db.photos.toArray();
          if (allPhotos.length === 0) return [];

          // Load album names for album-based searching
          const allAlbums = await db.albums.toArray();
          const albumNameMap = new Map<string, string>();
          for (const album of allAlbums) {
            albumNameMap.set(album.albumId, album.name);
          }

          const matches: SearchResult[] = [];
          for (const photo of allPhotos) {
            // Resolve album names for this photo
            const albumNames = (photo.albumIds ?? [])
              .map((id) => albumNameMap.get(id))
              .filter(Boolean)
              .join(" ");

            const searchableText = [
              photo.filename,
              photo.mediaType,
              photo.mimeType,
              photo.takenAt ? new Date(photo.takenAt).toISOString() : "",
              photo.latitude?.toString() ?? "",
              photo.longitude?.toString() ?? "",
              albumNames,
            ].join(" ");

            if (fuzzyMatch(searchableText, trimmed)) {
              let localThumbUrl: string | undefined;
              if (photo.thumbnailData) {
                const mime = photo.thumbnailMimeType || (photo.mediaType === "gif" ? "image/gif" : "image/jpeg");
                const blob = new Blob([photo.thumbnailData], { type: mime });
                localThumbUrl = URL.createObjectURL(blob);
              }
              matches.push({
                id: photo.blobId,
                filename: photo.filename,
                media_type: photo.mediaType,
                mime_type: photo.mimeType,
                thumb_path: null,
                created_at: photo.takenAt ? new Date(photo.takenAt).toISOString() : "",
                taken_at: photo.takenAt ? new Date(photo.takenAt).toISOString() : null,
                latitude: photo.latitude ?? null,
                longitude: photo.longitude ?? null,
                width: photo.width ?? null,
                height: photo.height ?? null,
                tags: [],
                _localThumbUrl: localThumbUrl,
              });
            }
          }
          return matches;
        } catch {
          return [];
        }
      })();

      const [serverRes, localResults] = await Promise.all([serverPromise, localPromise]);

      // Merge and deduplicate (server results take priority)
      const serverIds = new Set(serverRes.results.map((r) => r.id));
      // Local matches that are shadowed by a server result get dropped from the
      // merged list — revoke their thumbnail object URLs now, since they'll
      // never make it into `results` (and thus never hit the cleanup effect).
      for (const r of localResults) {
        if (serverIds.has(r.id) && r._localThumbUrl) {
          URL.revokeObjectURL(r._localThumbUrl);
        }
      }
      const combined = [
        ...serverRes.results,
        ...localResults.filter((r) => !serverIds.has(r.id)),
      ];

      setResults(combined);
      if (serverSearchFailed) {
        setSearchError("Server search unavailable — showing local results only.");
      }
    } catch {
      setResults([]);
    } finally {
      setLoading(false);
    }
  }, [fuzzyMatch]);

  // Debounced search
  useEffect(() => {
    const timer = setTimeout(() => doSearch(query), 300);
    return () => clearTimeout(timer);
  }, [query, doSearch]);

  // Free the thumbnail object URLs of the current result batch when results
  // are replaced (new search / clear) or on unmount. Each search creates a
  // fresh set of `_localThumbUrl`s, so revoking the prior batch is safe.
  // The cleanup runs before the next effect, after the new results render.
  useEffect(() => {
    const urls = results
      .map((r) => r._localThumbUrl)
      .filter((u): u is string => !!u);
    return () => {
      for (const u of urls) URL.revokeObjectURL(u);
    };
  }, [results]);

  const allSelected = results.length > 0 && selectedIds.size === results.length;

  async function deleteSelected() {
    if (selectedIds.size === 0 || deleting) return;
    const ids = [...selectedIds];
    if (!confirm(`Move ${ids.length} item${ids.length !== 1 ? "s" : ""} to trash? You can restore within 30 days.`)) return;
    setDeleting(true);
    try {
      await trashPhotos(ids);
      toast.success(`Moved ${ids.length} item${ids.length !== 1 ? "s" : ""} to trash`);
      clearSelection();
      // Re-run the search so results (and their thumbnail object URLs) rebuild
      // cleanly from the now-pruned local DB instead of leaving stale tiles.
      await doSearch(query);
    } catch (err: unknown) {
      toast.error(getErrorMessage(err));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="min-h-screen bg-canvas text-fg">
      <AppHeader />

      <main className="max-w-screen-2xl mx-auto px-4 py-6">
        {/* Search input */}
        <div className="relative max-w-xl mx-auto mb-6">
          <div className="absolute inset-y-0 left-3 flex items-center pointer-events-none">
            <AppIcon name="magnify-glass" size="w-5 h-5" />
          </div>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search tags, filenames, dates, media types…"
            maxLength={500}
            className="w-full pl-10 pr-4 py-3 rounded-xl border border-edge-strong bg-surface text-fg placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-accent-500 focus:border-transparent text-base"
          />
          {query && (
            <button
              onClick={() => { setQuery(""); setResults([]); setSearched(false); inputRef.current?.focus(); }}
              className="absolute inset-y-0 right-3 flex items-center text-fg-muted hover:text-fg"
            >
              ✕
            </button>
          )}
        </div>

        {/* Server search error banner */}
        {searchError && (
          <div className="max-w-xl mx-auto mb-4 px-4 py-3 rounded-lg bg-yellow-100 dark:bg-yellow-900/30 text-yellow-800 dark:text-yellow-200 text-sm">
            {searchError}
          </div>
        )}

        {/* Loading */}
        {loading && <GallerySkeleton />}

        {/* No results */}
        {searched && !loading && results.length === 0 && (
          <div className="text-center py-12">
            <p className="text-fg-muted">No results found for "{query}"</p>
            <p className="text-sm text-fg-muted mt-1">
              Try a tag, filename, date (e.g. "2024"), or type (e.g. "video")
            </p>
          </div>
        )}

        {/* Results grid */}
        {results.length > 0 && (
          <>
            {selectionMode ? (
              <div className="flex items-center justify-between gap-3 mb-3 p-3 bg-accent-50 dark:bg-accent-900/30 rounded-lg">
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
                    onClick={() => (allSelected ? clearSelection() : setAll(results.map((r) => r.id)))}
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
                  <button
                    onClick={deleteSelected}
                    disabled={selectedIds.size === 0 || deleting}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium bg-red-600 text-white hover:bg-red-700 shadow-sm disabled:opacity-50"
                  >
                    {deleting ? "Deleting…" : `Delete (${selectedIds.size})`}
                  </button>
                </div>
              </div>
            ) : (
              <p className="text-sm text-fg-muted mb-3">
                {results.length} result{results.length !== 1 ? "s" : ""}
              </p>
            )}
            <JustifiedGrid
              items={results}
              getAspectRatio={(r) => (r.width && r.height) ? r.width / r.height : 1}
              getKey={(r) => r.id}
              renderItem={(result, idx) => (
                <SearchResultTile
                  result={result}
                  selectionMode={selectionMode}
                  isSelected={selectedIds.has(result.id)}
                  onToggleSelect={() => (selectionMode ? toggle(result.id) : enter(result.id))}
                  onClick={() => {
                    if (selectionMode) {
                      toggle(result.id);
                    } else {
                      navigate(`/photo/${result.id}`, {
                        state: {
                          photoIds: results.map((r) => r.id),
                          currentIndex: idx,
                        },
                      });
                    }
                  }}
                  onLongPress={() => enter(result.id)}
                />
              )}
            />
          </>
        )}

        {/* Add-to-album picker for the current selection */}
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

        {/* Empty state */}
        {!query && (
          <div className="text-center py-16">
            <AppIcon name="magnify-glass" size="w-12 h-12" className="mx-auto mb-4 opacity-30" />
            <p className="text-fg-muted mb-1">Search your library</p>
            <p className="text-sm text-fg-muted">
              Search by tags, filenames, dates, or media types
            </p>
          </div>
        )}
      </main>
    </div>
  );
}

// ── Search Result Tile ───────────────────────────────────────────────────────

function SearchResultTile({
  result,
  onClick,
  selectionMode,
  isSelected,
  onToggleSelect,
  onLongPress,
}: {
  result: SearchResult;
  onClick: () => void;
  selectionMode: boolean;
  isSelected: boolean;
  onToggleSelect: () => void;
  onLongPress: () => void;
}) {
  const isGif = result.media_type === "gif";
  const [visible, setVisible] = useState(false);
  const [thumbSrc, setThumbSrc] = useState<string | null>(null);
  const tileRef = useRef<HTMLDivElement>(null);
  const longPressRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const didLongPress = useRef(false);

  function handlePointerDown() {
    didLongPress.current = false;
    longPressRef.current = setTimeout(() => {
      didLongPress.current = true;
      onLongPress();
      longPressRef.current = null;
    }, 500);
  }
  function handlePointerUp() {
    if (longPressRef.current) {
      clearTimeout(longPressRef.current);
      longPressRef.current = null;
    }
    if (!didLongPress.current) onClick();
  }
  function handlePointerLeave() {
    if (longPressRef.current) {
      clearTimeout(longPressRef.current);
      longPressRef.current = null;
    }
  }

  // One-shot viewport observer for all tiles — thumbnail IS the animated GIF for GIFs.
  useEffect(() => {
    const el = tileRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "200px" }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Fetch and display thumbnail when visible.
  // For GIFs: the server thumbnail endpoint returns an animated GIF, and
  // _localThumbUrl (encrypted gallery) points to the full animated blob.
  // Either way the thumbnail itself plays the animation — no extra load needed.
  useEffect(() => {
    if (!visible) return;
    if (result._localThumbUrl) {
      setThumbSrc(result._localThumbUrl);
      return;
    }
    let cancelled = false;
    // Object URL this tile creates for the fetched server thumbnail, so the
    // cleanup can revoke it (otherwise it leaks every time the tile unmounts
    // or re-fetches). Note: _localThumbUrl is owned by the parent results and
    // is revoked there, not here.
    let objectUrl: string | null = null;
    (async () => {
      try {
        const { accessToken } = useAuthStore.getState();
        const headers: Record<string, string> = {
          "X-Requested-With": "SimplePhotos",
        };
        if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;
        const res = await fetch(api.photos.thumbUrl(result.id), { headers });
        if (!res.ok || cancelled) return;
        const blob = await res.blob();
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        setThumbSrc(objectUrl);
      } catch {
        // Thumbnail load failed
      }
    })();
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [visible, result.id, result._localThumbUrl]);

  return (
    <div
      ref={tileRef}
      className={`relative w-full h-full bg-surface-raised overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group ${
        isSelected ? "ring-2 ring-accent-500" : ""
      }`}
      onPointerDown={handlePointerDown}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerLeave}
      onContextMenu={(e) => e.preventDefault()}
    >
      {/* Selection circle — always visible (top-right); tapping selects and
          enters selection mode, mirroring the gallery's AlbumTile. */}
      <button
        type="button"
        aria-label={isSelected ? "Deselect" : "Select"}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          onToggleSelect();
        }}
        onPointerDown={(e) => e.stopPropagation()}
        className={`absolute top-1.5 right-1.5 z-10 w-5 h-5 rounded-full border-2 flex items-center justify-center transition-all ${
          isSelected
            ? "bg-green-500 border-green-500 shadow"
            : selectionMode
              ? "bg-white/80 border-gray-400 hover:bg-white"
              : "bg-white/40 border-white/70 opacity-70 hover:opacity-100 hover:bg-white/80 shadow-sm"
        }`}
      >
        {isSelected && (
          <svg className="w-3 h-3 text-white" fill="currentColor" viewBox="0 0 20 20">
            <path fillRule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clipRule="evenodd" />
          </svg>
        )}
      </button>
      {thumbSrc ? (
        <img
          src={thumbSrc}
          alt={result.filename}
          className="w-full h-full object-cover"
          loading="lazy"
        />
      ) : (
        <div className="w-full h-full flex items-center justify-center text-fg-muted text-xs px-1 text-center">
          {result.filename}
        </div>
      )}

      {/* Media type badge */}
      {result.media_type === "video" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>▶</span>
        </div>
      )}
      {result.media_type === "gif" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded">
          GIF
        </div>
      )}
      {result.media_type === "audio" && (
        <div className="absolute bottom-1 right-1 bg-black/60 text-white text-xs px-1.5 py-0.5 rounded flex items-center gap-1">
          <span>♫</span>
        </div>
      )}

      {/* Metadata overlay on hover (date + location) */}
      <div className="absolute inset-x-0 bottom-0 p-1 opacity-0 group-hover:opacity-100 transition-opacity bg-gradient-to-t from-black/60 to-transparent">
        {result.taken_at && (
          <p className="text-white text-[10px] truncate">
            {new Date(result.taken_at).toLocaleDateString()}
          </p>
        )}
        {result.latitude != null && result.longitude != null && (
          <p className="text-white/80 text-[10px] truncate">
            📍 {result.latitude.toFixed(4)}, {result.longitude.toFixed(4)}
          </p>
        )}
      </div>

      {/* Tag chips overlay on hover */}
      {result.tags.length > 0 && (
        <div className="absolute inset-x-0 top-0 p-1 opacity-0 group-hover:opacity-100 transition-opacity">
          <div className="flex flex-wrap gap-0.5">
            {result.tags.slice(0, 3).map((tag) => (
              <span
                key={tag}
                className="bg-black/60 text-white text-[10px] px-1.5 py-0.5 rounded-full"
              >
                {tag}
              </span>
            ))}
            {result.tags.length > 3 && (
              <span className="bg-black/60 text-white text-[10px] px-1.5 py-0.5 rounded-full">
                +{result.tags.length - 3}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
