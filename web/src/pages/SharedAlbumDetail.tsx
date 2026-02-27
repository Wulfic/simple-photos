import { useEffect, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";

type SharedPhoto = {
  id: string;
  photo_ref: string;
  ref_type: string;
  added_at: string;
};

type AlbumMember = {
  id: string;
  user_id: string;
  username: string;
  added_at: string;
};

type ShareUser = { id: string; username: string };

export default function SharedAlbumDetail() {
  const { albumId } = useParams<{ albumId: string }>();
  const navigate = useNavigate();

  const [albumName, setAlbumName] = useState("");
  const [isOwner, setIsOwner] = useState(false);
  const [photos, setPhotos] = useState<SharedPhoto[]>([]);
  const [members, setMembers] = useState<AlbumMember[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [showMembers, setShowMembers] = useState(false);
  const [shareUsers, setShareUsers] = useState<ShareUser[]>([]);
  const [showSharePicker, setShowSharePicker] = useState(false);

  useEffect(() => {
    if (albumId) loadAlbum();
  }, [albumId]);

  async function loadAlbum() {
    setLoading(true);
    try {
      // Load album info from the list
      const albums = await api.sharing.listAlbums();
      const album = albums.find((a) => a.id === albumId);
      if (album) {
        setAlbumName(album.name);
        setIsOwner(album.is_owner);
      }
      // Load photos
      const photoList = await api.sharing.listPhotos(albumId!);
      setPhotos(photoList);
      // Load members
      const memberList = await api.sharing.listMembers(albumId!);
      setMembers(memberList);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleRemovePhoto(photoId: string) {
    try {
      await api.sharing.removePhoto(albumId!, photoId);
      setPhotos((prev) => prev.filter((p) => p.id !== photoId));
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleRemoveMember(userId: string) {
    try {
      await api.sharing.removeMember(albumId!, userId);
      setMembers((prev) => prev.filter((m) => m.user_id !== userId));
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function openSharePicker() {
    try {
      const users = await api.sharing.listUsers();
      setShareUsers(users);
      setShowSharePicker(true);
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function addMember(userId: string) {
    try {
      await api.sharing.addMember(albumId!, userId);
      setShowSharePicker(false);
      await loadAlbum();
    } catch (err: any) {
      setError(err.message);
    }
  }

  if (!albumId) {
    return <p className="p-4 text-red-600">Invalid album ID</p>;
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader>
        <div className="flex items-center gap-2">
          <button
            onClick={() => navigate("/albums")}
            className="text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100"
          >
            <AppIcon name="back-arrow" size="w-5 h-5" />
          </button>
          <span className="font-medium">{albumName || "Shared Album"}</span>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setShowMembers(!showMembers)}
            className="text-sm text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100 flex items-center gap-1"
          >
            <AppIcon name="shared" />
            {members.length}
          </button>
          {isOwner && (
            <button
              onClick={openSharePicker}
              className="inline-flex items-center gap-1 bg-green-600 text-white px-3 py-1.5 rounded-md hover:bg-green-500 text-sm font-medium"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M19 7.5v3m0 0v3m0-3h3m-3 0h-3m-2.25-4.125a3.375 3.375 0 11-6.75 0 3.375 3.375 0 016.75 0zM4 19.235v-.11a6.375 6.375 0 0112.75 0v.109A12.318 12.318 0 0110.374 21c-2.331 0-4.512-.645-6.374-1.766z" />
              </svg>
              Add Member
            </button>
          )}
        </div>
      </AppHeader>

      <main className="p-4">
        {error && (
          <p className="text-red-600 dark:text-red-400 text-sm mb-4">{error}</p>
        )}

        {/* Members panel */}
        {showMembers && (
          <div className="mb-4 bg-white dark:bg-gray-800 rounded-lg shadow p-4">
            <h3 className="text-sm font-semibold mb-2">Members</h3>
            {members.length === 0 && (
              <p className="text-sm text-gray-500 dark:text-gray-400">No members yet.</p>
            )}
            <ul className="space-y-1">
              {members.map((m) => (
                <li key={m.id} className="flex items-center justify-between text-sm py-1">
                  <span>{m.username}</span>
                  {isOwner && (
                    <button
                      onClick={() => handleRemoveMember(m.user_id)}
                      className="text-red-500 hover:text-red-700 text-xs"
                    >
                      Remove
                    </button>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}

        {/* Share picker modal */}
        {showSharePicker && (
          <div className="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4" onClick={() => setShowSharePicker(false)}>
            <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-sm w-full p-6" onClick={(e) => e.stopPropagation()}>
              <h3 className="text-lg font-semibold mb-4">Add Member</h3>
              <div className="space-y-2 max-h-64 overflow-y-auto">
                {shareUsers.map((u) => (
                  <button
                    key={u.id}
                    onClick={() => addMember(u.id)}
                    className="w-full text-left px-3 py-2 rounded-md hover:bg-gray-100 dark:hover:bg-gray-700 text-sm"
                  >
                    {u.username}
                  </button>
                ))}
              </div>
              <button onClick={() => setShowSharePicker(false)} className="mt-4 w-full py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-md">
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Photo grid */}
        <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 gap-2">
          {loading && photos.length === 0 && (
            <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-12">
              Loading...
            </p>
          )}
          {!loading && photos.length === 0 && (
            <p className="col-span-full text-gray-500 dark:text-gray-400 text-center py-12">
              No photos in this shared album yet.
            </p>
          )}
          {photos.map((photo) => (
            <div
              key={photo.id}
              className="aspect-square bg-gray-100 dark:bg-gray-700 rounded overflow-hidden relative group"
            >
              {photo.ref_type === "plain" ? (
                <img
                  src={api.photos.thumbUrl(photo.photo_ref)}
                  alt=""
                  className="w-full h-full object-cover"
                  loading="lazy"
                />
              ) : (
                <div className="w-full h-full flex items-center justify-center text-gray-400 text-xs">
                  Encrypted
                </div>
              )}
              {/* Remove button */}
              <button
                onClick={() => handleRemovePhoto(photo.id)}
                className="absolute top-1 right-1 hidden group-hover:block p-1 bg-black/60 text-white rounded"
                title="Remove from album"
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          ))}
        </div>
      </main>
    </div>
  );
}
