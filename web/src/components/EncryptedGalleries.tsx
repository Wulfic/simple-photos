import { useState, useCallback, useEffect } from "react";
import { api } from "../api/client";

interface Gallery {
  id: string;
  name: string;
  created_at: string;
  item_count: number;
}

interface EncryptedGalleriesProps {
  onError: (msg: string) => void;
  onSuccess: (msg: string) => void;
}

export default function EncryptedGalleries({ onError, onSuccess }: EncryptedGalleriesProps) {
  const [galleries, setGalleries] = useState<Gallery[]>([]);
  const [galleriesLoading, setGalleriesLoading] = useState(true);
  const [showCreateGallery, setShowCreateGallery] = useState(false);
  const [newGalleryName, setNewGalleryName] = useState("");
  const [newGalleryPassword, setNewGalleryPassword] = useState("");
  const [newGalleryConfirm, setNewGalleryConfirm] = useState("");
  const [loading, setLoading] = useState(false);

  const loadGalleries = useCallback(async () => {
    try {
      const res = await api.encryptedGalleries.list();
      setGalleries(res.galleries);
    } catch {
      // Ignore if galleries table doesn't exist yet
    } finally {
      setGalleriesLoading(false);
    }
  }, []);

  useEffect(() => {
    loadGalleries();
  }, [loadGalleries]);

  async function handleCreateGallery(e: React.FormEvent) {
    e.preventDefault();
    if (newGalleryPassword !== newGalleryConfirm) {
      onError("Gallery passwords do not match.");
      return;
    }
    setLoading(true);
    try {
      await api.encryptedGalleries.create(newGalleryName, newGalleryPassword);
      onSuccess(`Encrypted gallery "${newGalleryName}" created.`);
      setShowCreateGallery(false);
      setNewGalleryName("");
      setNewGalleryPassword("");
      setNewGalleryConfirm("");
      await loadGalleries();
    } catch (err: any) {
      onError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleDeleteGallery(id: string, name: string) {
    if (!confirm(`Delete encrypted gallery "${name}"? All items inside will be removed.`)) return;
    try {
      await api.encryptedGalleries.delete(id);
      onSuccess(`Encrypted gallery "${name}" deleted.`);
      await loadGalleries();
    } catch (err: any) {
      onError(err.message);
    }
  }

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <div className="flex items-center justify-between mb-3">
        <h2 className="text-lg font-semibold">Encrypted Galleries</h2>
        {!showCreateGallery && (
          <button
            onClick={() => {
              setShowCreateGallery(true);
              onError("");
            }}
            className="bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-700 text-sm"
          >
            + New Gallery
          </button>
        )}
      </div>

      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Encrypted galleries are always end-to-end encrypted and password-protected, independent of<br className="hidden sm:inline" /> your global storage mode. Use them to keep sensitive photos separate and secure.
      </p>

      {/* Create gallery form */}
      {showCreateGallery && (
        <form onSubmit={handleCreateGallery} className="bg-gray-50 dark:bg-gray-700/50 rounded-lg p-4 mb-4 space-y-3">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Gallery Name
            </label>
            <input
              type="text"
              value={newGalleryName}
              onChange={(e) => setNewGalleryName(e.target.value)}
              placeholder="e.g. Private Photos"
              className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-800 dark:border-gray-600"
              required
              maxLength={100}
              autoFocus
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Gallery Password
            </label>
            <input
              type="password"
              value={newGalleryPassword}
              onChange={(e) => setNewGalleryPassword(e.target.value)}
              placeholder="At least 4 characters"
              className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-800 dark:border-gray-600"
              required
              minLength={4}
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Confirm Password
            </label>
            <input
              type="password"
              value={newGalleryConfirm}
              onChange={(e) => setNewGalleryConfirm(e.target.value)}
              className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-800 dark:border-gray-600"
              required
              minLength={4}
            />
            {newGalleryConfirm.length > 0 && newGalleryPassword !== newGalleryConfirm && (
              <p className="text-xs text-red-500 dark:text-red-400 mt-1">Passwords do not match</p>
            )}
          </div>
          <div className="flex gap-2">
            <button
              type="submit"
              disabled={loading || !newGalleryName || newGalleryPassword.length < 4 || newGalleryPassword !== newGalleryConfirm}
              className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
            >
              {loading ? "Creating…" : "Create Gallery"}
            </button>
            <button
              type="button"
              onClick={() => {
                setShowCreateGallery(false);
                setNewGalleryName("");
                setNewGalleryPassword("");
                setNewGalleryConfirm("");
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
        <div className="text-gray-400 text-sm">Loading galleries…</div>
      ) : galleries.length === 0 ? (
        <div className="text-center py-6 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
          <p className="text-gray-400 text-sm">No encrypted galleries yet.</p>
        </div>
      ) : (
        <div className="space-y-2">
          {galleries.map((g) => (
            <div
              key={g.id}
              className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg"
            >
              <div>
                <div className="flex items-center gap-2">
                  <span className="text-sm">🔒</span>
                  <span className="font-medium text-gray-900 dark:text-gray-100">{g.name}</span>
                </div>
                <p className="text-xs text-gray-400 mt-0.5">
                  {g.item_count} item{g.item_count !== 1 ? "s" : ""} · Created {new Date(g.created_at).toLocaleDateString()}
                </p>
              </div>
              <button
                onClick={() => handleDeleteGallery(g.id, g.name)}
                className="text-red-500 hover:text-red-600 text-sm px-2 py-1"
                title="Delete gallery"
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
