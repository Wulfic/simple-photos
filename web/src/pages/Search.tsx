/**
 * Search page — tag-based and text search across the encrypted photo library
 * (local IndexedDB) and server, with unified results.
 */
import { useState, useEffect, useRef, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import JustifiedGrid from "../components/gallery/JustifiedGrid";

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
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [searchError, setSearchError] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

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

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 text-gray-900 dark:text-gray-100">
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
            className="w-full pl-10 pr-4 py-3 rounded-xl border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent text-base"
          />
          {query && (
            <button
              onClick={() => { setQuery(""); setResults([]); setSearched(false); inputRef.current?.focus(); }}
              className="absolute inset-y-0 right-3 flex items-center text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
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
        {loading && (
          <div className="flex justify-center py-12">
            <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
          </div>
        )}

        {/* No results */}
        {searched && !loading && results.length === 0 && (
          <div className="text-center py-12">
            <p className="text-gray-500 dark:text-gray-400">No results found for "{query}"</p>
            <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
              Try a tag, filename, date (e.g. "2024"), or type (e.g. "video")
            </p>
          </div>
        )}

        {/* Results grid */}
        {results.length > 0 && (
          <>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
              {results.length} result{results.length !== 1 ? "s" : ""}
            </p>
            <JustifiedGrid
              items={results}
              getAspectRatio={(r) => (r.width && r.height) ? r.width / r.height : 1}
              getKey={(r) => r.id}
              renderItem={(result, idx) => (
                <SearchResultTile
                  result={result}
                  onClick={() =>
                    navigate(`/photo/${result.id}`, {
                      state: {
                        photoIds: results.map((r) => r.id),
                        currentIndex: idx,
                      },
                    })
                  }
                />
              )}
            />
          </>
        )}

        {/* Empty state */}
        {!query && (
          <div className="text-center py-16">
            <AppIcon name="magnify-glass" size="w-12 h-12" className="mx-auto mb-4 opacity-30" />
            <p className="text-gray-500 dark:text-gray-400 mb-1">Search your library</p>
            <p className="text-sm text-gray-400 dark:text-gray-500">
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
}: {
  result: SearchResult;
  onClick: () => void;
}) {
  const isGif = result.media_type === "gif";
  const [visible, setVisible] = useState(false);
  const [thumbSrc, setThumbSrc] = useState<string | null>(null);
  const tileRef = useRef<HTMLDivElement>(null);

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
      className="relative w-full h-full bg-gray-100 dark:bg-gray-700 overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group"
      onClick={onClick}
    >
      {thumbSrc ? (
        <img
          src={thumbSrc}
          alt={result.filename}
          className="w-full h-full object-cover"
          loading="lazy"
        />
      ) : (
        <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center">
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
