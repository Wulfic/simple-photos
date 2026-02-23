import { useState, useCallback, useEffect } from "react";
import { api } from "../api/client";
import { db, type CachedPhoto } from "../db";
import { useLiveQuery } from "dexie-react-hooks";
import AppHeader from "../components/AppHeader";

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
}

/**
 * Secure Albums page.
 *
 * Flow: password gate → album list → album detail with items.
 * Uses the user's account password (not a per-album password).
 */
export default function SecureGallery() {
  // Auth gate state
  const [authenticated, setAuthenticated] = useState(false);
  const [galleryToken, setGalleryToken] = useState("");
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

  // Create album state
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  // Add photos state
  const [showAddPhotos, setShowAddPhotos] = useState(false);
  const [selectedPhotos, setSelectedPhotos] = useState<Set<string>>(new Set());
  const [addingPhotos, setAddingPhotos] = useState(false);

  // Error / success
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  // Cached photos from IndexedDB for photo picker
  const cachedPhotos = useLiveQuery(() =>
    db.photos.orderBy("takenAt").reverse().toArray()
  );

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
      } catch {
        setError("Failed to load album items.");
      } finally {
        setItemsLoading(false);
      }
    },
    [galleryToken]
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
      setAuthenticated(true);
      setPassword("");
    } catch (err: any) {
      setAuthError(err.message || "Invalid password");
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
    } catch (err: any) {
      setError(err.message);
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
    } catch (err: any) {
      setError(err.message);
    }
  }

  // Toggle photo selection for adding to album
  function togglePhotoSelection(blobId: string) {
    setSelectedPhotos((prev) => {
      const next = new Set(prev);
      if (next.has(blobId)) next.delete(blobId);
      else next.add(blobId);
      return next;
    });
  }

  // Add selected photos to the current album
  async function handleAddSelectedPhotos() {
    if (!selectedGallery || selectedPhotos.size === 0) return;
    setAddingPhotos(true);
    setError("");
    try {
      for (const blobId of selectedPhotos) {
        await api.secureGalleries.addItem(selectedGallery.id, blobId);
      }
      setSuccess(`${selectedPhotos.size} photo${selectedPhotos.size !== 1 ? "s" : ""} added to album.`);
      setSelectedPhotos(new Set());
      setShowAddPhotos(false);
      await loadItems(selectedGallery.id);
      await loadGalleries();
    } catch (err: any) {
      setError(err.message);
    } finally {
      setAddingPhotos(false);
    }
  }

  // Get blob IDs already in this album (to filter from picker)
  const albumBlobIds = new Set(items.map((i) => i.blob_id));
  const availablePhotos = (cachedPhotos || []).filter(
    (p) => !albumBlobIds.has(p.blobId)
  );

  // ── Password Gate ───────────────────────────────────────────────────────────

  if (!authenticated) {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
        <AppHeader />
        <main className="max-w-md mx-auto p-4 mt-16">
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow-lg p-8">
            <div className="text-center mb-6">
              <div className="w-16 h-16 mx-auto mb-4 bg-blue-100 dark:bg-blue-900/30 rounded-full flex items-center justify-center">
                <span className="text-3xl">🔒</span>
              </div>
              <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100">
                Secure Albums
              </h2>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-2">
                Enter your account password to access your secure albums.
              </p>
            </div>

            <form onSubmit={handleUnlock} className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Password
                </label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600 dark:text-white"
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
                className="w-full bg-blue-600 text-white py-2.5 rounded-md hover:bg-blue-700 disabled:opacity-50 font-medium text-sm"
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
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
        <AppHeader>
          {showAddPhotos ? (
            <>
              <button
                onClick={handleAddSelectedPhotos}
                disabled={selectedPhotos.size === 0 || addingPhotos}
                className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3.5 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors shadow-sm shadow-blue-900/20 disabled:opacity-50"
              >
                {addingPhotos ? "Adding…" : `Add (${selectedPhotos.size})`}
              </button>
              <button
                onClick={() => {
                  setShowAddPhotos(false);
                  setSelectedPhotos(new Set());
                }}
                className="inline-flex items-center gap-1.5 bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-3.5 py-1.5 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm font-medium transition-colors"
              >
                Cancel
              </button>
            </>
          ) : null}
        </AppHeader>
        <main className="max-w-6xl mx-auto p-4">
          {/* Back + title + actions */}
          <div className="flex items-center justify-between mb-6">
            <div className="flex items-center gap-3">
              <button
                onClick={() => {
                  setSelectedGallery(null);
                  setItems([]);
                  setShowAddPhotos(false);
                  setSelectedPhotos(new Set());
                }}
                className="text-blue-600 hover:text-blue-700 text-sm font-medium flex items-center gap-1"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                </svg>
                Back
              </button>
              <h2 className="text-xl font-semibold dark:text-white flex items-center gap-2">
                <span>🔒</span> {selectedGallery.name}
              </h2>
              <span className="text-gray-400 text-sm">{items.length} items</span>
            </div>
            {!showAddPhotos && (
              <div className="flex gap-2">
                <button
                  onClick={() => {
                    setShowAddPhotos(true);
                    setSelectedPhotos(new Set());
                  }}
                  className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors"
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

          {/* Add photos picker */}
          {showAddPhotos && (
            <div className="mb-6 p-4 bg-blue-50 dark:bg-blue-900/30 rounded-lg">
              <p className="text-sm font-medium text-blue-800 dark:text-blue-300 mb-3">
                Select photos from your gallery to add ({selectedPhotos.size} selected)
              </p>
              {availablePhotos.length === 0 ? (
                <p className="text-gray-500 dark:text-gray-400 text-sm">
                  {(cachedPhotos?.length ?? 0) === 0
                    ? "No photos in your gallery yet. Upload some photos first."
                    : "All photos are already in this album."}
                </p>
              ) : (
                <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 gap-1.5 max-h-80 overflow-y-auto">
                  {availablePhotos.map((photo) => {
                    const isSelected = selectedPhotos.has(photo.blobId);
                    return (
                      <div
                        key={photo.blobId}
                        className={`relative aspect-square rounded-lg overflow-hidden cursor-pointer border-2 transition-all ${
                          isSelected
                            ? "border-blue-600 ring-2 ring-blue-400"
                            : "border-transparent hover:border-gray-300 dark:hover:border-gray-500"
                        }`}
                        onClick={() => togglePhotoSelection(photo.blobId)}
                      >
                        <PhotoThumbnail photo={photo} />
                        {/* Selection circle in top-right */}
                        <div
                          className={`absolute top-1.5 right-1.5 w-5 h-5 rounded-full border-2 flex items-center justify-center transition-all ${
                            isSelected
                              ? "bg-blue-600 border-blue-600"
                              : "bg-white/70 border-gray-400 dark:bg-gray-800/70 dark:border-gray-500"
                          }`}
                        >
                          {isSelected && (
                            <svg className="w-3 h-3 text-white" fill="currentColor" viewBox="0 0 20 20">
                              <path
                                fillRule="evenodd"
                                d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z"
                                clipRule="evenodd"
                              />
                            </svg>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {itemsLoading ? (
            <div className="flex justify-center py-12">
              <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
            </div>
          ) : items.length === 0 && !showAddPhotos ? (
            <div className="text-center py-16 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
              <span className="text-4xl mb-3 block">🖼️</span>
              <p className="text-gray-400 text-sm mb-3">This album is empty.</p>
              <button
                onClick={() => {
                  setShowAddPhotos(true);
                  setSelectedPhotos(new Set());
                }}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium"
              >
                Add Photos from Gallery
              </button>
            </div>
          ) : !showAddPhotos ? (
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-2">
              {items.map((item) => (
                <ItemTile key={item.id} item={item} />
              ))}
            </div>
          ) : null}
        </main>
      </div>
    );
  }

  // ── Album List View ─────────────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />
      <main className="max-w-4xl mx-auto p-4">
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <div>
            <h2 className="text-xl font-semibold dark:text-white flex items-center gap-2">
              <span>🔒</span> Secure Albums
            </h2>
            <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
              End-to-end encrypted albums for your most private photos.
            </p>
          </div>
          {!showCreate && (
            <button
              onClick={() => {
                setShowCreate(true);
                setError("");
                setSuccess("");
              }}
              className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium"
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
            className="bg-white dark:bg-gray-800 rounded-lg shadow p-5 mb-6 space-y-3"
          >
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300">
              Create New Album
            </h3>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Album Name
              </label>
              <input
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="e.g. Private Photos"
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600 dark:text-white"
                required
                maxLength={100}
                autoFocus
              />
            </div>
            <div className="flex gap-2">
              <button
                type="submit"
                disabled={creating || !newName.trim()}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                {creating ? "Creating…" : "Create Album"}
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowCreate(false);
                  setNewName("");
                }}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </form>
        )}

        {/* Album list */}
        {galleriesLoading ? (
          <div className="flex justify-center py-12">
            <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
          </div>
        ) : galleries.length === 0 ? (
          <div className="text-center py-16 bg-white dark:bg-gray-800 rounded-lg shadow">
            <span className="text-4xl mb-3 block">🔒</span>
            <p className="text-gray-500 dark:text-gray-400 font-medium">
              No secure albums yet
            </p>
            <p className="text-sm text-gray-400 mt-1">
              Create an album to store your most private photos securely.
            </p>
            {!showCreate && (
              <button
                onClick={() => setShowCreate(true)}
                className="mt-4 bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium"
              >
                + Create your first album
              </button>
            )}
          </div>
        ) : (
          <div className="space-y-3">
            {galleries.map((g) => (
              <div
                key={g.id}
                className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 flex items-center justify-between hover:ring-2 hover:ring-blue-200 dark:hover:ring-blue-800 transition-all cursor-pointer"
                onClick={() => setSelectedGallery(g)}
              >
                <div className="flex items-center gap-3">
                  <div className="w-12 h-12 bg-blue-100 dark:bg-blue-900/30 rounded-lg flex items-center justify-center">
                    <span className="text-xl">🔒</span>
                  </div>
                  <div>
                    <h3 className="font-medium text-gray-900 dark:text-gray-100">
                      {g.name}
                    </h3>
                    <p className="text-xs text-gray-400 mt-0.5">
                      {g.item_count} item{g.item_count !== 1 ? "s" : ""} · Created{" "}
                      {new Date(g.created_at).toLocaleDateString()}
                    </p>
                  </div>
                </div>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    handleDelete(g);
                  }}
                  className="text-red-500 hover:text-red-600 text-sm px-3 py-1.5 rounded-md hover:bg-red-50 dark:hover:bg-red-900/20"
                  title="Delete album"
                >
                  Delete
                </button>
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

// ── Item Tile (shows decrypted thumbnail if available) ────────────────────────

function ItemTile({ item }: { item: GalleryItem }) {
  const cachedPhoto = useLiveQuery(
    () => db.photos.get(item.blob_id),
    [item.blob_id]
  );

  if (cachedPhoto?.thumbnailData) {
    return (
      <div className="aspect-square bg-gray-200 dark:bg-gray-700 rounded-lg overflow-hidden">
        <PhotoThumbnail photo={cachedPhoto} />
      </div>
    );
  }

  return (
    <div className="aspect-square bg-gray-200 dark:bg-gray-700 rounded-lg flex items-center justify-center overflow-hidden">
      <div className="text-center text-gray-400">
        <span className="text-2xl block mb-1">🔐</span>
        <span className="text-xs">Encrypted</span>
      </div>
    </div>
  );
}

// ── Photo Thumbnail helper ────────────────────────────────────────────────────

function PhotoThumbnail({ photo }: { photo: CachedPhoto }) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    if (photo.thumbnailData) {
      const url = URL.createObjectURL(
        new Blob([photo.thumbnailData], { type: "image/jpeg" })
      );
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    }
  }, [photo.thumbnailData]);

  if (src) {
    return (
      <img
        src={src}
        alt={photo.filename}
        className="w-full h-full object-cover"
        loading="lazy"
      />
    );
  }

  return (
    <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs px-1 text-center bg-gray-100 dark:bg-gray-700">
      {photo.filename}
    </div>
  );
}
