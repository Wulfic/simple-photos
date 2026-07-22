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
import { usePhotoSelection } from "../hooks/usePhotoSelection";
import { api } from "../api/client";
import { db } from "../db";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import JustifiedGrid from "../components/gallery/JustifiedGrid";
import { getErrorMessage } from "../utils/formatters";
import { getEffectiveAspectRatio } from "../utils/thumbnailCss";
import {
  otherSecureAlbumItems,
  resolveSecureMoves,
  expandSecureSelection,
  planSecureMovesToTarget,
  secureMoveTargets,
} from "../gallery/secureMovePicker";
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
import {
  SecureGalleryItem,
  SecureAlbumCover,
  SecureSmartAlbumCover,
} from "../gallery";
import {
  computeSecureSmartAlbums,
  filterSecureSmartAlbum,
  isSecureSmartAlbum,
  type SecureSmartAlbum,
} from "../gallery/secureSmartAlbums";
import type { SecureGalleryItem as SecureItem } from "../api/galleries";
import { GallerySkeleton, AlbumGridSkeleton } from "../components/skeletons";

interface Gallery {
  id: string;
  name: string;
  created_at: string;
  item_count: number;
}

// Item shape from the secure-gallery API (per-album and aggregate). `gallery_id`
// is always present; `gallery_name` only on the aggregate feed.
type GalleryItem = SecureItem;

/** Synthesize a Gallery card from a computed secure smart album. */
function smartToGallery(sa: SecureSmartAlbum): Gallery {
  return { id: sa.id, name: sa.label, created_at: "", item_count: sa.count };
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

  // Aggregate feed across ALL secure galleries — drives the built-in secure
  // smart albums (Secure Gallery / Photos / GIFs / Videos / Audio). Fetched
  // once after unlock and refreshed on every mutation (create/delete/remove).
  const [allItems, setAllItems] = useState<GalleryItem[]>([]);
  const [allItemsLoading, setAllItemsLoading] = useState(false);

  // Visible secure smart albums (non-empty types only), recomputed as the feed
  // changes. Cheap pure derivation — no memo needed. Declared here so the
  // ?album restore effect (below) can reference it.
  const smartAlbums = computeSecureSmartAlbums(allItems);

  // Preserve scroll position per secure album when opening a photo and
  // returning. Keyed by the selected gallery so each album restores its own.
  useScrollMemory(`secure-gallery:${selectedGallery?.id ?? ""}`, items.length > 0);

  // Create album state
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  // Cross-secure-album picker (#31): move items INTO the open album from the
  // user's OTHER secure albums. A photo can live in only one secure album, so
  // this is a move (server reassigns membership), not a copy.
  const [showMovePicker, setShowMovePicker] = useState(false);
  const [moveSelected, setMoveSelected] = useState<Set<string>>(new Set());
  const [moving, setMoving] = useState(false);

  // Push direction (#43): select items in the OPEN album and move them OUT to
  // another secure album. Same server op as the pull picker, opposite framing.
  // Reuses the shared multi-select hook so behaviour matches every other grid.
  const pushSelect = usePhotoSelection();
  const [showMoveTarget, setShowMoveTarget] = useState(false);
  const [movingPush, setMovingPush] = useState(false);

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
    if (!authenticated || !albumId || selectedGallery) return;
    // Smart album: synthesize the selection from the aggregate feed once loaded.
    // Fixes return-from-viewer landing back INSIDE a smart album.
    if (isSecureSmartAlbum(albumId)) {
      const sa = smartAlbums.find((s) => s.id === albumId);
      if (sa) setSelectedGallery(smartToGallery(sa));
      return;
    }
    if (galleries.length > 0) {
      const g = galleries.find((x) => x.id === albumId);
      if (g) setSelectedGallery(g);
    }
  }, [authenticated, searchParams, galleries, selectedGallery, smartAlbums]);

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

  // Load the aggregate item feed for the secure smart albums. Token-gated like
  // per-album items — a rejected token means the session lapsed → back to gate.
  const loadAllItems = useCallback(async () => {
    if (!galleryToken) return;
    setAllItemsLoading(true);
    try {
      const res = await api.secureGalleries.listAllItems(galleryToken);
      setAllItems(res.items);
    } catch (err: unknown) {
      if (isGalleryTokenRejection(err)) {
        lock("Your secure session expired. Enter your password to continue.");
      } else {
        console.error("[SecureGallery] Failed to load aggregate items", err);
        setError("Failed to load secure albums.");
      }
    } finally {
      setAllItemsLoading(false);
    }
  }, [galleryToken, lock]);

  useEffect(() => {
    if (authenticated) loadAllItems();
  }, [authenticated, loadAllItems]);

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

  // Real album → fetch its items with the gallery token. Smart album → derive
  // items from the already-loaded aggregate feed (no per-gallery request).
  useEffect(() => {
    if (selectedGallery && !isSecureSmartAlbum(selectedGallery.id)) {
      loadItems(selectedGallery.id);
    }
  }, [selectedGallery, loadItems]);

  useEffect(() => {
    if (selectedGallery && isSecureSmartAlbum(selectedGallery.id)) {
      setItems(filterSecureSmartAlbum(allItems, selectedGallery.id));
    }
  }, [selectedGallery, allItems]);

  // Reset the push selection whenever the open album changes (or we return to
  // the list) so a selection never carries across albums or lingers on Back.
  useEffect(() => {
    pushSelect.clear();
    setShowMoveTarget(false);
  }, [selectedGallery?.id]); // eslint-disable-line react-hooks/exhaustive-deps

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
      await loadAllItems();
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
      await loadAllItems();
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
    // In a smart view the selected "gallery" is synthetic — route removal to
    // the item's REAL owning album. `gallery_id` is always present on both the
    // per-album and aggregate feeds.
    const smartView = isSecureSmartAlbum(selectedGallery.id);
    const owningGalleryId = smartView ? item.gallery_id : selectedGallery.id;
    if (!owningGalleryId) {
      setError("Could not determine which album this photo belongs to.");
      return;
    }
    if (!confirm("Remove this photo from the secure album? It will return to your regular gallery."))
      return;
    try {
      await api.secureGalleries.removeItem(owningGalleryId, item.id);
      // Drop the local IDB clone entry that `handleAddSelectedPhotos`
      // created at add time, so the secure album view stays consistent
      // even before the next reload.
      try { await db.photos.delete(item.blob_id); } catch { /* non-fatal */ }
      setSuccess("Photo returned to your gallery.");
      // Refresh the aggregate feed (smart items re-derive from it via effect)
      // and the album list; real albums also re-fetch their own items.
      await loadAllItems();
      await loadGalleries();
      if (!smartView) await loadItems(selectedGallery.id);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  // Items in the user's OTHER secure albums — the source pool for the
  // cross-secure-album move picker. Excludes the open album's own items.
  const otherSecureItems = selectedGallery
    ? otherSecureAlbumItems(allItems, selectedGallery.id)
    : [];

  // Real secure albums the current selection can be pushed INTO (#43). Excludes
  // the open album; for a smart view its synthetic id matches nothing, so every
  // real album is offered.
  const moveTargets = selectedGallery
    ? secureMoveTargets(galleries, selectedGallery.id)
    : [];

  function toggleMoveSelect(itemId: string) {
    setMoveSelected((prev) => {
      const next = new Set(prev);
      if (next.has(itemId)) next.delete(itemId);
      else next.add(itemId);
      return next;
    });
  }

  // Move the picked items from their current secure albums into the open album.
  async function handleMoveSelected() {
    if (!selectedGallery || moveSelected.size === 0 || moving) return;
    setMoving(true);
    let moved = 0;
    let failed = 0;
    // Resolve each selected item to its owning gallery, then reassign it. Each
    // is isolated so one failure never aborts the rest (mirrors secure-add).
    const moves = resolveSecureMoves(otherSecureItems, moveSelected);
    failed += moveSelected.size - moves.length; // selections we couldn't route
    for (const mv of moves) {
      try {
        await api.secureGalleries.moveItem(mv.sourceGalleryId, mv.itemId, selectedGallery.id);
        moved++;
      } catch (err) {
        console.error("[SecureGallery] move item failed", err); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
        failed++;
      }
    }
    if (moved > 0) setSuccess(`Moved ${moved} item${moved !== 1 ? "s" : ""} into "${selectedGallery.name}".`);
    if (failed > 0) setError(`${failed} item${failed !== 1 ? "s" : ""} couldn't be moved.`);
    setMoveSelected(new Set());
    setShowMovePicker(false);
    setMoving(false);
    // Refresh: aggregate feed re-derives, the open album re-fetches, counts update.
    await loadAllItems();
    await loadGalleries();
    await loadItems(selectedGallery.id);
  }

  // Push (#43): move the items selected in the OPEN album into `targetGalleryId`.
  async function moveSelectedTo(targetGalleryId: string) {
    if (!selectedGallery || pushSelect.selectedIds.size === 0 || movingPush) return;
    setMovingPush(true);
    setError("");
    setSuccess("");
    // A burst tile stands in for every frame, so a move must carry them all
    // (mirrors secure-add) or a burst is split across two albums. Each item
    // routes from its own gallery_id — right for a real album AND a smart view.
    const expanded = expandSecureSelection(items, pushSelect.selectedIds);
    const moves = planSecureMovesToTarget(items, expanded, targetGalleryId);
    const targetName = galleries.find((g) => g.id === targetGalleryId)?.name ?? "the album";
    if (moves.length === 0) {
      // Everything selected already lives in the target — nothing to do.
      setSuccess(`Those photos are already in "${targetName}".`);
      pushSelect.clear();
      setShowMoveTarget(false);
      setMovingPush(false);
      return;
    }
    let moved = 0;
    let failed = 0;
    // Each move is isolated so one failure never aborts the rest — a partial
    // failure must not lose items (mirrors the pull picker and secure-add).
    for (const mv of moves) {
      try {
        await api.secureGalleries.moveItem(mv.sourceGalleryId, mv.itemId, targetGalleryId);
        moved++;
      } catch (err) {
        console.error("[SecureGallery] push move failed", err); // nosemgrep: javascript.lang.security.audit.unsafe-formatstring.unsafe-formatstring
        failed++;
      }
    }
    if (moved > 0) setSuccess(`Moved ${moved} item${moved !== 1 ? "s" : ""} into "${targetName}".`);
    if (failed > 0) setError(`${failed} item${failed !== 1 ? "s" : ""} couldn't be moved.`);
    pushSelect.clear();
    setShowMoveTarget(false);
    setMovingPush(false);
    // Refresh: aggregate feed re-derives smart views; a real album re-fetches.
    await loadAllItems();
    await loadGalleries();
    if (!isSecureSmartAlbum(selectedGallery.id)) await loadItems(selectedGallery.id);
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
            {!isBackupServer && !pushSelect.selectionMode && (
              <div className="flex gap-2">
                {/* Select items to push OUT to another secure album (#43). Works
                    in real AND smart albums, whenever there's a target to move
                    into. */}
                {items.length > 0 && moveTargets.length > 0 && (
                  <button
                    onClick={() => pushSelect.enterEmpty()}
                    className="btn btn-secondary btn-md inline-flex items-center"
                    title="Select photos to move to another secure album"
                  >
                    Select
                  </button>
                )}
                {!isSecureSmartAlbum(selectedGallery.id) && (
                  <>
                    {/* Move media in from the user's OTHER secure albums (#31).
                        Only offered when there's somewhere to pull from. */}
                    {otherSecureItems.length > 0 && (
                      <button
                        onClick={() => { setShowMovePicker((v) => !v); setMoveSelected(new Set()); }}
                        className={`btn btn-md inline-flex items-center ${showMovePicker ? "btn-primary" : "btn-secondary"}`}
                        title="Move photos in from your other secure albums"
                      >
                        <span className="mr-1">🔒</span>
                        {showMovePicker ? "Done" : "From secure albums"}
                      </button>
                    )}
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
                  </>
                )}
              </div>
            )}
          </div>

          {/* Push selection bar (#43): move the selected items OUT to another
              secure album. Mirrors the main gallery's selection bar. */}
          {pushSelect.selectionMode && (
            <div className="flex items-center justify-between gap-3 mb-4 p-3 bg-accent-50 dark:bg-accent-900/30 rounded-lg">
              <div className="flex items-center gap-3">
                <button
                  onClick={() => pushSelect.clear()}
                  className="text-fg-muted hover:text-fg"
                  aria-label="Cancel selection"
                >
                  <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
                <span className="text-sm font-medium tabular-nums">
                  {pushSelect.selectedIds.size} selected
                </span>
                <button
                  onClick={() =>
                    pushSelect.selectedIds.size === displayItems.length
                      ? pushSelect.clear()
                      : pushSelect.setAll(displayItems.map((i) => i.id))
                  }
                  className="text-accent-600 dark:text-accent-400 text-sm hover:underline"
                >
                  {displayItems.length > 0 && pushSelect.selectedIds.size === displayItems.length
                    ? "Deselect All"
                    : "Select All"}
                </button>
              </div>
              <button
                onClick={() => setShowMoveTarget(true)}
                disabled={pushSelect.selectedIds.size === 0 || movingPush}
                className="btn btn-primary btn-md inline-flex items-center gap-1.5"
                title="Move selected photos to another secure album"
              >
                <span>🔒</span>
                {`Move to album (${pushSelect.selectedIds.size})`}
              </button>
            </div>
          )}

          {/* Cross-secure-album move picker (#31) */}
          {showMovePicker && !isBackupServer && !isSecureSmartAlbum(selectedGallery.id) && (
            <div className="card p-4 mb-6">
              <div className="flex items-center justify-between mb-3 gap-3">
                <h3 className="text-sm font-semibold text-fg-muted min-w-0 truncate">
                  Move from your other secure albums
                  <span className="tabular-nums"> ({moveSelected.size} selected)</span>
                </h3>
                <button
                  onClick={handleMoveSelected}
                  disabled={moveSelected.size === 0 || moving}
                  className="btn btn-primary btn-md shrink-0 whitespace-nowrap"
                >
                  {moving ? "Moving…" : `Move here (${moveSelected.size})`}
                </button>
              </div>
              <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 gap-2">
                {otherSecureItems.map((it) => {
                  const selected = moveSelected.has(it.id);
                  return (
                    <div
                      key={it.id}
                      className={`relative aspect-square rounded overflow-hidden cursor-pointer ${selected ? "ring-2 ring-accent-500" : ""}`}
                      onClick={() => toggleMoveSelect(it.id)}
                    >
                      <SecureGalleryItem item={it} onClick={() => toggleMoveSelect(it.id)} />
                      {selected && (
                        <div className="absolute top-1 right-1 w-5 h-5 rounded-full bg-green-500 flex items-center justify-center z-10 pointer-events-none">
                          <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                          </svg>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

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

          {(isSecureSmartAlbum(selectedGallery.id) ? allItemsLoading : itemsLoading) ? (
            <GallerySkeleton />
          ) : items.length === 0 ? (
            <div className="text-center py-16 border-2 border-dashed border-edge rounded-lg">
              <span className="text-4xl mb-3 block">🖼️</span>
              <p className="text-fg-muted text-sm mb-3">This album is empty.</p>
              {!isBackupServer && !isSecureSmartAlbum(selectedGallery.id) && (
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
              getAspectRatio={(item) =>
                getEffectiveAspectRatio(item.width, item.height, item.crop_metadata)
              }
              getKey={(item) => item.id}
              renderItem={(item, idx) => {
                const pushSelected = pushSelect.selectedIds.has(item.id);
                return (
                <div
                  className={`group relative w-full h-full ${
                    pushSelect.selectionMode && pushSelected ? "ring-2 ring-accent-500 rounded" : ""
                  }`}
                >
                  <SecureGalleryItem
                    item={item}
                    burstCount={item.burst_id ? secureBurstCounts.get(item.burst_id) : undefined}
                    onClick={() =>
                      pushSelect.selectionMode
                        ? pushSelect.toggle(item.id)
                        : navigate(`/photo/${item.blob_id}`, {
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
                  {pushSelect.selectionMode && pushSelected && (
                    <div className="absolute top-1 right-1 w-5 h-5 rounded-full bg-green-500 flex items-center justify-center z-10 pointer-events-none">
                      <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                      </svg>
                    </div>
                  )}
                  {!isBackupServer && !pushSelect.selectionMode && (
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
                );
              }}
            />
          )}

          {/* Target-album picker (#43): pick which secure album to move the
              selection into. */}
          {showMoveTarget && (
            <div
              className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
              onClick={() => { if (!movingPush) setShowMoveTarget(false); }}
            >
              <div className="card p-5 w-full max-w-sm" onClick={(e) => e.stopPropagation()}>
                <h3 className="text-base font-semibold text-fg mb-3 flex items-center gap-2">
                  <span>🔒</span>
                  Move {pushSelect.selectedIds.size} item{pushSelect.selectedIds.size !== 1 ? "s" : ""} to
                </h3>
                {moveTargets.length === 0 ? (
                  <p className="text-sm text-fg-muted">No other secure albums to move into.</p>
                ) : (
                  <div className="space-y-1 max-h-72 overflow-y-auto">
                    {moveTargets.map((t) => (
                      <button
                        key={t.id}
                        onClick={() => moveSelectedTo(t.id)}
                        disabled={movingPush}
                        className="w-full text-left px-3 py-2 rounded-md hover:bg-surface-raised flex items-center gap-2 disabled:opacity-50"
                      >
                        <span className="shrink-0">🔒</span>
                        <span className="truncate">{t.name}</span>
                      </button>
                    ))}
                  </div>
                )}
                <div className="flex justify-end mt-4">
                  <button
                    onClick={() => setShowMoveTarget(false)}
                    disabled={movingPush}
                    className="btn btn-secondary btn-md"
                  >
                    {movingPush ? "Moving…" : "Cancel"}
                  </button>
                </div>
              </div>
            </div>
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

        {/* Smart albums — built-in, media-type derived; only non-empty types.
            Read-only: no delete affordance, no create. */}
        {smartAlbums.length > 0 && (
          <div className="mb-8">
            <h3 className="text-sm font-semibold text-fg-muted mb-3">Smart albums</h3>
            <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 gap-3">
              {smartAlbums.map((sa) => (
                <div
                  key={sa.id}
                  className="card card-interactive p-2 cursor-pointer relative"
                  onClick={() => {
                    setSelectedGallery(smartToGallery(sa));
                    navigate(`/secure-gallery?album=${sa.id}`);
                  }}
                >
                  <div className="aspect-square bg-surface-raised rounded mb-1.5 flex items-center justify-center overflow-hidden">
                    <SecureSmartAlbumCover item={sa.coverItem} />
                  </div>
                  <p className="font-medium text-sm truncate flex items-center gap-1">
                    <span className="shrink-0">🔒</span>
                    <span className="truncate">{sa.label}</span>
                  </p>
                  <p className="text-xs text-fg-muted">
                    {sa.count} item{sa.count !== 1 ? "s" : ""}
                  </p>
                </div>
              ))}
            </div>
          </div>
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
