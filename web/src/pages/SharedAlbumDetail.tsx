/**
 * Shared album detail page — displays photos in a collaborative album
 * and manages member list (add/remove users).
 */
import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { useAppNavigate } from "../hooks/useAppNavigate";
import { api } from "../api/client";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { useThumbnailSizeStore } from "../store/thumbnailSize";
import { getErrorMessage } from "../utils/formatters";
import { toast } from "../store/toast";

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

import type { ShareUser } from "../types/sharing";
import SharePickerModal from "../components/SharePickerModal";

export default function SharedAlbumDetail() {
  const { albumId } = useParams<{ albumId: string }>();
  const navigate = useAppNavigate();
  const gridClasses = useThumbnailSizeStore((s) => s.gridClasses)();

  const [albumName, setAlbumName] = useState("");
  const [isOwner, setIsOwner] = useState(false);
  const [photos, setPhotos] = useState<SharedPhoto[]>([]);
  const [members, setMembers] = useState<AlbumMember[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  // Surface errors as a dismissible toast (#8) instead of an inline bar.
  useEffect(() => {
    if (error) {
      toast.error(error);
      setError("");
    }
  }, [error]);
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
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }

  async function handleRemovePhoto(photoId: string) {
    try {
      await api.sharing.removePhoto(albumId!, photoId);
      setPhotos((prev) => prev.filter((p) => p.id !== photoId));
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function handleRemoveMember(userId: string) {
    try {
      await api.sharing.removeMember(albumId!, userId);
      setMembers((prev) => prev.filter((m) => m.user_id !== userId));
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function openSharePicker() {
    try {
      const users = await api.sharing.listUsers();
      setShareUsers(users);
      setShowSharePicker(true);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  async function addMember(userId: string) {
    try {
      await api.sharing.addMember(albumId!, userId);
      setShowSharePicker(false);
      await loadAlbum();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    }
  }

  if (!albumId) {
    return <p className="p-4 text-red-600">Invalid album ID</p>;
  }

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      <main className="p-4">
        {/* Sub-header with album name + actions */}
        <div className="flex items-center justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <button
              onClick={() => navigate("/albums")}
              className="text-fg-muted hover:text-fg transition-colors shrink-0"
            >
              <AppIcon name="back-arrow" size="w-5 h-5" />
            </button>
            <h2 className="text-xl font-semibold truncate">{albumName || "Shared Album"}</h2>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <button
              onClick={() => setShowMembers(!showMembers)}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 text-fg-muted bg-white dark:bg-white/10 border border-edge hover:bg-surface-sunken dark:hover:bg-white/20 shadow-sm"
            >
              <AppIcon name="shared" />
              <span>{members.length}</span>
            </button>
            {isOwner && (
              <button
                onClick={openSharePicker}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm font-medium transition-all duration-200 text-fg-muted bg-white dark:bg-white/10 border border-edge hover:bg-surface-sunken dark:hover:bg-white/20 shadow-sm"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M19 7.5v3m0 0v3m0-3h3m-3 0h-3m-2.25-4.125a3.375 3.375 0 11-6.75 0 3.375 3.375 0 016.75 0zM4 19.235v-.11a6.375 6.375 0 0112.75 0v.109A12.318 12.318 0 0110.374 21c-2.331 0-4.512-.645-6.374-1.766z" />
                </svg>
                <span className="hidden sm:inline">Add Member</span>
              </button>
            )}
          </div>
        </div>

        {/* Errors surface via the global toast host (#8) */}

        {/* Members panel */}
        {showMembers && (
          <div className="card mb-4 p-4">
            <h3 className="text-sm font-semibold mb-2">Members</h3>
            {members.length === 0 && (
              <p className="text-sm text-fg-muted">No members yet.</p>
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
          <SharePickerModal
            title="Add Member"
            users={shareUsers}
            onPick={(id) => addMember(id)}
            onClose={() => setShowSharePicker(false)}
          />
        )}

        {/* Photo grid */}
        <div className={gridClasses}>
          {loading && photos.length === 0 && (
            <p className="col-span-full text-fg-muted text-center py-12">
              Loading...
            </p>
          )}
          {!loading && photos.length === 0 && (
            <p className="col-span-full text-fg-muted text-center py-12">
              No photos in this shared album yet.
            </p>
          )}
          {photos.map((photo) => (
            <div
              key={photo.id}
              className="aspect-square bg-surface-raised rounded overflow-hidden relative group"
            >
              {photo.ref_type === "photo" ? (
                <img
                  src={api.photos.thumbUrl(photo.photo_ref)}
                  alt=""
                  className="w-full h-full object-cover"
                  loading="lazy"
                />
              ) : (
                <div className="w-full h-full flex items-center justify-center text-fg-muted text-xs">
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
