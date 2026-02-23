import { useState, useCallback, useEffect } from "react";
import { api } from "../api/client";
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
 * Secure Gallery page.
 *
 * Flow: password gate → gallery list → gallery detail with items.
 * Uses the user's account password (not a per-gallery password).
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

  // Create gallery state
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  // Error / success
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  // Load galleries after auth
  const loadGalleries = useCallback(async () => {
    setGalleriesLoading(true);
    try {
      const res = await api.secureGalleries.list();
      setGalleries(res.galleries);
    } catch {
      setError("Failed to load galleries.");
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
        setError("Failed to load gallery items.");
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
      setSuccess(`Gallery "${newName.trim()}" created.`);
      setNewName("");
      setShowCreate(false);
      await loadGalleries();
    } catch (err: any) {
      setError(err.message);
    } finally {
      setCreating(false);
    }
  }

  // Delete gallery
  async function handleDelete(gallery: Gallery) {
    if (!confirm(`Delete secure gallery "${gallery.name}"? All items inside will be removed.`))
      return;
    try {
      await api.secureGalleries.delete(gallery.id);
      setSuccess(`Gallery "${gallery.name}" deleted.`);
      if (selectedGallery?.id === gallery.id) {
        setSelectedGallery(null);
        setItems([]);
      }
      await loadGalleries();
    } catch (err: any) {
      setError(err.message);
    }
  }

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
                Secure Gallery
              </h2>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-2">
                Enter your account password to access your secure galleries.
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

  // ── Gallery Detail View ─────────────────────────────────────────────────────

  if (selectedGallery) {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
        <AppHeader />
        <main className="max-w-6xl mx-auto p-4">
          {/* Back + title */}
          <div className="flex items-center gap-3 mb-6">
            <button
              onClick={() => {
                setSelectedGallery(null);
                setItems([]);
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
          </div>

          {error && (
            <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">
              {error}
            </p>
          )}

          {itemsLoading ? (
            <div className="flex justify-center py-12">
              <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
            </div>
          ) : items.length === 0 ? (
            <div className="text-center py-16 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
              <span className="text-4xl mb-3 block">🖼️</span>
              <p className="text-gray-400 text-sm">This gallery is empty.</p>
              <p className="text-xs text-gray-400 mt-1">
                Items can be added to secure galleries from the Import page.
              </p>
            </div>
          ) : (
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-2">
              {items.map((item) => (
                <div
                  key={item.id}
                  className="aspect-square bg-gray-200 dark:bg-gray-700 rounded-lg flex items-center justify-center overflow-hidden"
                >
                  {/* Items are encrypted blobs — show a placeholder icon */}
                  <div className="text-center text-gray-400">
                    <span className="text-2xl block mb-1">🔐</span>
                    <span className="text-xs">Encrypted</span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </main>
      </div>
    );
  }

  // ── Gallery List View ───────────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />
      <main className="max-w-4xl mx-auto p-4">
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <div>
            <h2 className="text-xl font-semibold dark:text-white flex items-center gap-2">
              <span>🔒</span> Secure Galleries
            </h2>
            <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
              End-to-end encrypted galleries for your most private photos.
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
              + New Gallery
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

        {/* Create gallery form */}
        {showCreate && (
          <form
            onSubmit={handleCreate}
            className="bg-white dark:bg-gray-800 rounded-lg shadow p-5 mb-6 space-y-3"
          >
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300">
              Create New Gallery
            </h3>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Gallery Name
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
                {creating ? "Creating…" : "Create Gallery"}
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

        {/* Gallery list */}
        {galleriesLoading ? (
          <div className="flex justify-center py-12">
            <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
          </div>
        ) : galleries.length === 0 ? (
          <div className="text-center py-16 bg-white dark:bg-gray-800 rounded-lg shadow">
            <span className="text-4xl mb-3 block">🔒</span>
            <p className="text-gray-500 dark:text-gray-400 font-medium">
              No secure galleries yet
            </p>
            <p className="text-sm text-gray-400 mt-1">
              Create a gallery to store your most private photos securely.
            </p>
            {!showCreate && (
              <button
                onClick={() => setShowCreate(true)}
                className="mt-4 bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium"
              >
                + Create your first gallery
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
                  title="Delete gallery"
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
