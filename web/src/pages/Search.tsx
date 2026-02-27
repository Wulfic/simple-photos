import { useState, useEffect, useRef, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";

// ── Types ────────────────────────────────────────────────────────────────────

interface SearchResult {
  id: string;
  filename: string;
  media_type: string;
  mime_type: string;
  thumb_path: string | null;
  created_at: string;
  tags: string[];
}

// ── Search Page ──────────────────────────────────────────────────────────────

export default function Search() {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [allTags, setAllTags] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Load all user tags on mount
  useEffect(() => {
    api.tags.list().then((res) => setAllTags(res.tags)).catch(() => {});
  }, []);

  // Auto-focus the search input
  useEffect(() => {
    inputRef.current?.focus();
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
    try {
      const res = await api.search.query(trimmed);
      setResults(res.results);
    } catch {
      setResults([]);
    } finally {
      setLoading(false);
    }
  }, []);

  // Debounced search
  useEffect(() => {
    const timer = setTimeout(() => doSearch(query), 300);
    return () => clearTimeout(timer);
  }, [query, doSearch]);

  function handleTagClick(tag: string) {
    setQuery(tag);
  }

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
            placeholder="Search by tag or filename…"
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

        {/* Tag cloud — shown when no query */}
        {!query && allTags.length > 0 && (
          <div className="max-w-xl mx-auto mb-8">
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-2">Your tags</p>
            <div className="flex flex-wrap gap-2">
              {allTags.map((tag) => (
                <button
                  key={tag}
                  onClick={() => handleTagClick(tag)}
                  className="px-3 py-1.5 rounded-full bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300 text-sm font-medium hover:bg-blue-200 dark:hover:bg-blue-800/60 transition-colors"
                >
                  {tag}
                </button>
              ))}
            </div>
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
              Try a different tag name or filename
            </p>
          </div>
        )}

        {/* Results grid */}
        {results.length > 0 && (
          <>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
              {results.length} result{results.length !== 1 ? "s" : ""}
            </p>
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2">
              {results.map((result, idx) => (
                <SearchResultTile
                  key={result.id}
                  result={result}
                  onClick={() =>
                    navigate(`/photo/plain/${result.id}`, {
                      state: {
                        photoIds: results.map((r) => r.id),
                        currentIndex: idx,
                      },
                    })
                  }
                />
              ))}
            </div>
          </>
        )}

        {/* Empty state — no tags at all */}
        {!query && allTags.length === 0 && (
          <div className="text-center py-16">
            <AppIcon name="magnify-glass" size="w-12 h-12" className="mx-auto mb-4 opacity-30" />
            <p className="text-gray-500 dark:text-gray-400 mb-1">No tags yet</p>
            <p className="text-sm text-gray-400 dark:text-gray-500">
              Open a photo and add tags to start organizing your library
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
  const [visible, setVisible] = useState(false);
  const [thumbSrc, setThumbSrc] = useState<string | null>(null);
  const tileRef = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    if (!visible) return;
    let cancelled = false;
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
        const url = URL.createObjectURL(blob);
        if (!cancelled) setThumbSrc(url);
      } catch {
        // Thumbnail load failed
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [visible, result.id]);

  return (
    <div
      ref={tileRef}
      className="relative aspect-square bg-gray-100 dark:bg-gray-700 rounded overflow-hidden cursor-pointer hover:opacity-90 transition-opacity group"
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
