/**
 * Password-protected secure gallery page.
 *
 * Users can create PIN/password-gated galleries, add photos from the main
 * library, and unlock them with a password. Photos inside secure galleries
 * are hidden from the main gallery view.
 */
import { useState, useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { useAppNavigate } from "../hooks/useAppNavigate";
import { useScrollMemory } from "../hooks/useScrollMemory";
import { api } from "../api/client";
import { db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import JustifiedGrid from "../components/gallery/JustifiedGrid";
import { getErrorMessage } from "../utils/formatters";
import { useIsBackupServer } from "../hooks/useIsBackupServer";
import { useAuthStore } from "../store/auth";
import {
  setGalleryToken as persistGalleryToken,
  getGalleryToken,
  clearGalleryToken,
  hasFreshGalleryToken,
  isGalleryTokenRejection,
} from "../utils/galleryToken";
import { useSecureAdd } from "../store/secureAdd";
import { SecureGalleryItem, SecureAlbumCover } from "../gallery";
import { GallerySkeleton, AlbumGridSkeleton } from "../components/skeletons";

interface Gallery {
  id: string;
  name: string;
  created_at: string;
  item_count: number;
}

interface GalleryItem {
  id: string;
  blob_id: string;
  added_at: string;
  encrypted_thumb_blob_id?: string | null;
  width?: number | null;
  height?: number | null;
  media_type?: string | null;
  photo_subtype?: string | null;
  burst_id?: string | null;
  duration_secs?: number | null;
  motion_video_blob_id?: string | null;
}

/**
 * Secure Albums page.
 *
 * Flow: password gate → album list → album detail with items.
 * Uses the user's account password (not a per-album password).
 */
export default function SecureGallery() {
  const navigate = useAppNavigate();
  const [searchParams] = useSearchParams();
  const isBackupServer = useIsBackupServer();
  const startSecureAdd = useSecureAdd((s) => s.start);

  // Auth gate state. Restore from the session unlock token so returning from
  // the photo viewer (which remounts this page) lands back IN the secure album
  // instead of the password gate — and, combined with the ?album auto-select
  // effect below, restores the exact album you were viewing (#6: closing a
  // secure photo dumped you out of the secure gallery).
  //
  // CRITICAL: gate on token *freshness*, not mere presence. The token lives in
  // sessionStorage (whole tab lifetime) but the server only honours it for one
  // hour. Keying `authenticated` off `!!token` meant an expired token skipped
  // the password gate yet every secure request 401'd → "no password prompt AND
  // nothing loads". `hasFreshGalleryToken()` re-prompts once the token is stale.
  const persistedFresh = hasFreshGalleryToken();
  const persistedToken = persistedFresh ? (getGalleryToken() ?? "") : "";
  const [authenticated, setAuthenticated] = useState(persistedFresh);
  const [galleryToken, setGalleryToken] = useState(persistedToken);
  const [password, setPassword] = useState("");
  const [authError, setAuthError] = useState("");
  const [authLoading, setAuthLoading] = useState(false);

  // Gallery list state
  const [galleries, setGalleries] = useState<Gallery[]>([]);
  const [galleriesLoading, setGalleriesLoading] = useState(false);
  const [selectedGallery, setSelectedGallery] = useState<Gallery | null>(null);

  // Gallery items state
  const [items, setItems] = useState<GalleryItem[]>([]);
  const [itemsLoading, setItemsLoading] = useState(false);

  // Preserve scroll position per secure album when opening a photo and
  // returning. Keyed by the selected gallery so each album restores its own.
  useScrollMemory(`secure-gallery:${selectedGallery?.id ?? ""}`, items.length > 0);

  // Create album state
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  // Error / success
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  // A ref so the URL-sync effect can read the current gallery without being
  // in its dependency array (avoids infinite-loop risk).
  const selectedGalleryRef = useRef(selectedGallery);
  useEffect(() => { selectedGalleryRef.current = selectedGallery; }, [selectedGallery]);

  // Drop a stale token from sessionStorage on mount so the central API client
  // (api/core.ts) stops attaching a dead X-Gallery-Token to every request.
  // Runs once; fresh tokens are left untouched so #6 (return-from-viewer)
  // still works.
  useEffect(() => {
    if (!hasFreshGalleryToken()) clearGalleryToken();
  }, []);

  // Return to the password gate when the unlock token is no longer accepted
  // (expired past its 1-hour TTL, or invalidated by a server restart that
  // rotated the JWT secret). Without this the user is stranded: "unlocked" per
  // the UI but unable to load anything and with no way to re-enter the password.
  const lock = useCallback((message?: string) => {
    clearGalleryToken();
    setGalleryToken("");
    setAuthenticated(false);
    setSelectedGallery(null);
    setItems([]);
    setAuthError(message ?? "");
  }, []);

  // When the browser Back button removes the ?album param, return to the
  // album list without navigating away from the page entirely.
  useEffect(() => {
    if (!searchParams.get("album") && selectedGalleryRef.current !== null) {
      setSelectedGallery(null);
      setItems([]);
    }
  }, [searchParams]); // eslint-disable-line react-hooks/exhaustive-deps

  // Re-select the album named in ?album=… once galleries are loaded. This
  // restores the album detail view when returning from the photo viewer (the
  // page remounted with selectedGallery=null but the URL still points at the
  // album).
  useEffect(() => {
    const albumId = searchParams.get("album");
    if (authenticated && albumId && !selectedGallery && galleries.length > 0) {
      const g = galleries.find((x) => x.id === albumId);
      if (g) setSelectedGallery(g);
    }
  }, [authenticated, searchParams, galleries, selectedGallery]);

  // Load galleries after auth
  const loadGalleries = useCallback(async () => {
    setGalleriesLoading(true);
    try {
      const res = await api.secureGalleries.list();
      setGalleries(res.galleries);
    } catch {
      setError("Failed to load albums.");
    } finally {
      setGalleriesLoading(false);
    }
  }, []);

  useEffect(() => {
    if (authenticated) loadGalleries();
  }, [authenticated, loadGalleries]);

  // Load items for selected gallery
  const loadItems = useCallback(
    async (galleryId: string) => {
      setItemsLoading(true);
      try {
        const res = await api.secureGalleries.listItems(galleryId, galleryToken);
        setItems(res.items);
      } catch (err: unknown) {
        // A rejected token means the session lapsed — send the user back to the
        // gate to re-unlock instead of stranding them on a permanently empty
        // album with a generic error.
        if (isGalleryTokenRejection(err)) {
          lock("Your secure session expired. Enter your password to continue.");
        } else {
          setError("Failed to load album items.");
        }
      } finally {
        setItemsLoading(false);
      }
    },
    [galleryToken, lock]
  );

  useEffect(() => {
    if (selectedGallery) loadItems(selectedGallery.id);
  }, [selectedGallery, loadItems]);

  // Handle password auth
  async function handleUnlock(e: React.FormEvent) {
    e.preventDefault();
    setAuthError("");
    setAuthLoading(true);
    try {
      const res = await api.secureGalleries.unlock(password);
      setGalleryToken(res.gallery_token);
      // Persist to sessionStorage so media requests (thumbnails in this grid,
      // and the full Viewer opened on a separate route) can present the token
      // to the server's secure-album gate.
      persistGalleryToken(res.gallery_token);
      setAuthenticated(true);
      setPassword("");
    } catch (err: unknown) {
      setAuthError(getErrorMessage(err, "Invalid password"));
    } finally {
      setAuthLoading(false);
    }
  }

  // Create new gallery
  async function handleCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!newName.trim()) return;
    setCreating(true);
    setError("");
    try {
      await api.secureGalleries.create(newName.trim());
      setSuccess(`Album "${newName.trim()}" created.`);
      setNewName("");
      setShowCreate(false);
      await loadGalleries();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setCreating(false);
    }
  }

  // Delete album
  async function handleDelete(gallery: Gallery) {
    if (!confirm(`Delete secure album "${gallery.name}"? All items inside will be removed.`))
      return;
    try {
      await api.secureGalleries.delete(gallery.id);
      setSuccess(`Album "${gallery.name}" deleted.`);
      if (selectedGallery?.id === gallery.id) {
        setSelectedGallery(null);
        setItems([]);
      }
      await loadGalleries();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  // Remove a single item from the current secure album.
  // The cloned blob is deleted server-side and the original photo becomes
  // visible again in the regular gallery (the server's
  // `/galleries/secure/blob-ids` endpoint will stop reporting its id, so
  // the next gallery refresh unhides it automatically).
  async function handleRemoveItem(item: GalleryItem) {
    if (!selectedGallery) return;
    if (!confirm("Remove this photo from the secure album? It will return to your regular gallery."))
      return;
    try {
      await api.secureGalleries.removeItem(selectedGallery.id, item.id);
      // Drop the local IDB clone entry that `handleAddSelectedPhotos`
      // created at add time, so the secure album view stays consistent
      // even before the next reload.
      try { await db.photos.delete(item.blob_id); } catch { /* non-fatal */ }
      setSuccess("Photo returned to your gallery.");
      await loadItems(selectedGallery.id);
      await loadGalleries();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  // Collapse burst stacks → one tile / viewer page per burst (the album still
  // physically holds every frame). Counts come from the full list for the badge.
  const secureBurstCounts = new Map<string, number>();
  for (const it of items) {
    if (it.burst_id) secureBurstCounts.set(it.burst_id, (secureBurstCounts.get(it.burst_id) ?? 0) + 1);
  }
  const seenBursts = new Set<string>();
  const displayItems = items.filter((it) => {
    if (!it.burst_id) return true;
    if (seenBursts.has(it.burst_id)) return false;
    seenBursts.add(it.burst_id);
    return true;
  });

  // ── Password Gate ───────────────────────────────────────────────────────────

  if (!authenticated) {
    return (
      <div className="min-h-screen bg-canvas">
        <AppHeader />
        <main className="max-w-md mx-auto p-4 mt-16">
          <div className="card shadow-card-hover p-8">
            <div className="text-center mb-6">
              <div className="w-16 h-16 mx-auto mb-4 bg-accent-100 dark:bg-accent-900/30 rounded-full flex items-center justify-center">
                <AppIcon name="locks" size="w-8 h-8" />
              </div>
              <h2 className="text-xl font-bold text-fg">
                Secure Albums
              </h2>
              <p className="text-sm text-fg-muted mt-2">
                Enter your account password to access your secure albums.
              </p>
            </div>

            <form onSubmit={handleUnlock} className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-fg-muted mb-1">
                  Password
                </label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="input"
                  required
                  autoFocus
                  autoComplete="current-password"
                  placeholder="Enter your password"
                />
              </div>

              {authError && (
                <p className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/30 rounded p-2">
                  {authError}
                </p>
              )}

              <button
                type="submit"
                disabled={authLoading || !password}
                className="btn btn-primary btn-md w-full"
              >
                {authLoading ? (
                  <span className="flex items-center justify-center gap-2">
                    <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                    Verifying…
                  </span>
                ) : (
                  "Unlock"
                )}
              </button>
            </form>
          </div>
        </main>
      </div>
    );
  }

  // ── Album Detail View ────────────────────────────────────────────────────

  if (selectedGallery) {
    return (
      <div className="min-h-screen bg-canvas">
        <AppHeader />
        <main className="p-4">
          {/* Back + title + actions */}
          <div className="flex items-center justify-between mb-6">
            <div className="flex items-center gap-3">
              <button
                onClick={() => {
                  setSelectedGallery(null);
                  setItems([]);
                  // Replace the current history entry so the browser Back
                  // button returns to the album list, not an orphaned URL.
                  navigate("/secure-gallery", { replace: true });
                }}
                className="text-accent-600 hover:text-accent-700 text-sm font-medium flex items-center gap-1"
              >
                <AppIcon name="back-arrow" />
                Back
              </button>
              <h2 className="text-xl font-semibold dark:text-white flex items-center gap-2">
                <span>🔒</span> {selectedGallery.name}
              </h2>
              <span className="text-fg-muted text-sm">{displayItems.length} items</span>
            </div>
            {!isBackupServer && (
              <div className="flex gap-2">
                <button
                  onClick={() => {
                    // Browse your regular/smart albums to pick photos, instead
                    // of scrolling one giant flat master list. The secure-add
                    // session lets every album grid offer an "Add to 🔒" action.
                    startSecureAdd(selectedGallery.id, selectedGallery.name);
                    navigate("/albums");
                  }}
                  className="btn btn-primary btn-md inline-flex items-center"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
                  </svg>
                  Add Photos
                </button>
              </div>
            )}
          </div>

          {error && (
            <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">
              {error}
            </p>
          )}
          {success && (
            <p className="text-green-600 dark:text-green-400 text-sm mb-4 p-3 bg-green-50 dark:bg-green-900/30 rounded">
              {success}
            </p>
          )}

          {itemsLoading ? (
            <GallerySkeleton />
          ) : items.length === 0 ? (
            <div className="text-center py-16 border-2 border-dashed border-edge rounded-lg">
              <span className="text-4xl mb-3 block">🖼️</span>
              <p className="text-fg-muted text-sm mb-3">This album is empty.</p>
              {!isBackupServer && (
              <button
                onClick={() => {
                  startSecureAdd(selectedGallery.id, selectedGallery.name);
                  navigate("/albums");
                }}
                className="btn btn-primary btn-md"
              >
                Add Photos from Gallery
              </button>
              )}
            </div>
          ) : (
            <JustifiedGrid
              items={displayItems}
              getAspectRatio={(item) => (item.width && item.height) ? item.width / item.height : 1}
              getKey={(item) => item.id}
              renderItem={(item, idx) => (
                <div className="group relative w-full h-full">
                  <SecureGalleryItem
                    item={item}
                    burstCount={item.burst_id ? secureBurstCounts.get(item.burst_id) : undefined}
                    onClick={() =>
                      navigate(`/photo/${item.blob_id}`, {
                        state: {
                          photoIds: displayItems.map((i) => i.blob_id),
                          currentIndex: idx,
                          secureGallery: true,
                          secureAlbumId: selectedGallery?.id,
                          // Full (un-collapsed) item list, including every frame of
                          // every burst — the Viewer's BurstStrip needs this to show
                          // burst frames, since secured photos never sync into the
                          // local IDB photo cache it normally reads subtype/burst
                          // info from (they're intentionally excluded from main
                          // gallery sync).
                          secureItems: items,
                        },
                      })
                    }
                  />
                  {!isBackupServer && (
                    <button
                      onClick={(e) => { e.stopPropagation(); handleRemoveItem(item); }}
                      className="absolute top-1 right-1 hidden group-hover:flex items-center justify-center w-7 h-7 bg-black/60 hover:bg-red-600 text-white rounded-full transition-colors z-10"
                      title="Remove from secure album (returns to regular gallery)"
                      aria-label="Remove from secure album"
                    >
                      <AppIcon name="trashcan" size="w-4 h-4" />
                    </button>
                  )}
                </div>
              )}
            />
          )}
        </main>
      </div>
    );
  }

  // ── Album List View ─────────────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />
      <main className="p-4">
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <div>
            <h2 className="text-xl font-semibold dark:text-white flex items-center gap-2">
              <span>🔒</span> Secure Albums
            </h2>
            <p className="text-fg-muted text-sm mt-1">
              End-to-end encrypted albums for your most private photos.
            </p>
          </div>
          {!showCreate && !isBackupServer && (
            <button
              onClick={() => {
                setShowCreate(true);
                setError("");
                setSuccess("");
              }}
              className="btn btn-primary btn-md"
            >
              + New Album
            </button>
          )}
        </div>

        {/* Messages */}
        {error && (
          <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">
            {error}
          </p>
        )}
        {success && (
          <p className="text-green-600 dark:text-green-400 text-sm mb-4 p-3 bg-green-50 dark:bg-green-900/30 rounded">
            {success}
          </p>
        )}

        {/* Create album form */}
        {showCreate && (
          <form
            onSubmit={handleCreate}
            className="card p-5 mb-6 space-y-3"
          >
            <h3 className="text-sm font-semibold text-fg-muted">
              Create New Album
            </h3>
            <div>
              <label className="block text-sm font-medium text-fg-muted mb-1">
                Album Name
              </label>
              <input
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="e.g. Private Photos"
                className="input"
                required
                maxLength={100}
                autoFocus
              />
            </div>
            <div className="flex gap-2">
              <button
                type="submit"
                disabled={creating || !newName.trim()}
                className="btn btn-primary btn-md"
              >
                {creating ? "Creating…" : "Create Album"}
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowCreate(false);
                  setNewName("");
                }}
                className="btn btn-secondary btn-md"
              >
                Cancel
              </button>
            </div>
          </form>
        )}

        {/* Album list */}
        {galleriesLoading ? (
          <AlbumGridSkeleton />
        ) : galleries.length === 0 ? (
          <div className="card text-center py-16">
            <span className="text-4xl mb-3 block">🔒</span>
            <p className="text-fg-muted font-medium">
              No secure albums yet
            </p>
            <p className="text-sm text-fg-muted mt-1">
              Create an album to store your most private photos securely.
            </p>
            {!showCreate && !isBackupServer && (
              <button
                onClick={() => setShowCreate(true)}
                className="btn btn-primary btn-md mt-4"
              >
                + Create your first album
              </button>
            )}
          </div>
        ) : (
          // Card grid mirroring the regular Albums page, with the delete button
          // tucked inside each card (hover-revealed) the way shared albums do.
          <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 gap-3">
            {galleries.map((g) => (
              <div
                key={g.id}
                className="card card-interactive p-2 cursor-pointer relative group"
                onClick={() => {
                  setSelectedGallery(g);
                  // Push a history entry so the browser Back button returns
                  // here to the album list rather than jumping to the
                  // previous page (e.g. the main gallery).
                  navigate(`/secure-gallery?album=${g.id}`);
                }}
              >
                <div className="aspect-square bg-surface-raised rounded mb-1.5 flex items-center justify-center overflow-hidden">
                  <SecureAlbumCover
                    galleryId={g.id}
                    galleryToken={galleryToken}
                    itemCount={g.item_count}
                  />
                </div>
                <p className="font-medium text-sm truncate flex items-center gap-1">
                  <span className="shrink-0">🔒</span>
                  <span className="truncate">{g.name}</span>
                </p>
                <p className="text-xs text-fg-muted">
                  {g.item_count} item{g.item_count !== 1 ? "s" : ""}
                </p>

                {!isBackupServer && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDelete(g);
                    }}
                    className="absolute top-2 right-2 hidden group-hover:flex items-center justify-center p-1 bg-white dark:bg-gray-700 rounded shadow text-red-500 hover:text-red-700"
                    title="Delete album"
                    aria-label="Delete secure album"
                  >
                    <AppIcon name="trashcan" />
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}
