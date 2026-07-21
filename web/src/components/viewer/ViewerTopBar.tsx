/**
 * Viewer top toolbar — back, favorite, info, edit up front; tags, slideshow,
 * download and delete/remove tucked into a 3-dot overflow menu (todo1 #3).
 */
import { useState, useRef, useEffect } from "react";
import type { MediaType } from "../../db";
import AppIcon from "../AppIcon";

export interface ViewerTopBarProps {
  editMode: boolean;
  showOverlay: boolean;
  showInfoPanel: boolean;
  setShowInfoPanel: (v: boolean) => void;
  showTagPanel: boolean;
  setShowTagPanel: (v: boolean) => void;
  mediaType: MediaType;
  mediaUrl: string | null;
  isFavorite: boolean;
  isBackupServer: boolean;
  /**
   * Allow the Edit button even when `isBackupServer` is true. Used by the secure
   * viewer (#31): secure mode passes `isBackupServer` to keep favorite / tags /
   * delete hidden, but edit is safe (a crop stays inside the secure album and is
   * persisted via the secure crop endpoint), so it is opted back in here.
   */
  allowEdit?: boolean;
  isRenderingVideo: boolean;
  /** True only for real user-created albums — smart albums (Photos, Videos,
   *  GIFs, Audio, People, …) can't have items "removed" so they show Delete. */
  canRemoveFromAlbum?: boolean;
  onBack: () => void;
  onToggleEdit: () => void;
  onToggleFavorite: () => void;
  onDownload: () => void;
  onDelete: () => void;
  onRemoveFromAlbum: () => void;
  onStartSlideshow?: () => void;
  hasSlideshow?: boolean;
}

export default function ViewerTopBar({
  editMode,
  showOverlay,
  showInfoPanel,
  setShowInfoPanel,
  showTagPanel,
  setShowTagPanel,
  mediaType,
  mediaUrl,
  isFavorite,
  isBackupServer,
  allowEdit,
  isRenderingVideo,
  canRemoveFromAlbum,
  onBack,
  onToggleEdit,
  onToggleFavorite,
  onDownload,
  onDelete,
  onRemoveFromAlbum,
  onStartSlideshow,
  hasSlideshow,
}: ViewerTopBarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // Close the overflow menu on outside click or Escape (same pattern as the
  // AppHeader user dropdown).
  useEffect(() => {
    if (!menuOpen) return;
    function onClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuOpen(false);
    }
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  const canEdit =
    (mediaType === "photo" || mediaType === "gif" || mediaType === "video" || mediaType === "audio") &&
    (!isBackupServer || !!allowEdit);

  const menuItemClass =
    "w-full text-left px-4 py-2 text-sm text-fg-muted hover:bg-surface-sunken dark:hover:bg-white/10 flex items-center gap-2 transition-colors";

  return (
    <div className={`absolute top-0 left-0 right-0 z-30 transition-opacity duration-300 ${
      showOverlay || editMode ? "opacity-100" : "opacity-0 pointer-events-none"
    }`}>
    <div className="flex items-center justify-between px-4 py-3 bg-black/80">
      <button
        onClick={onBack}
        className="text-white hover:text-gray-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
        title="Back"
      >
        <AppIcon name="back-arrow" size="w-5 h-5" themed={false} className="invert" />
      </button>
      <div className="flex gap-3 items-center">
        {!isBackupServer && (
        <button
          onClick={onToggleFavorite}
          className={`hover:scale-110 transition-transform ${isFavorite ? "text-yellow-400" : "text-white hover:text-yellow-300"}`}
          title={isFavorite ? "Unfavorite" : "Favorite"}
        >
          {isFavorite ? (
            <svg className="w-5 h-5" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" /></svg>
          ) : (
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z" /></svg>
          )}
        </button>
        )}
        {canEdit && (
          <button
            onClick={onToggleEdit}
            className={`flex items-center gap-1 px-2 py-1 rounded text-sm font-medium transition-colors ${
              editMode ? "bg-accent-600 text-white" : "text-white hover:bg-white/20"
            }`}
            title="Edit"
          >Edit</button>
        )}

        {/* ── Overflow menu: tags · slideshow · download · delete/remove ── */}
        <div className="relative" ref={menuRef}>
          <button
            onClick={() => setMenuOpen((v) => !v)}
            className={`flex items-center justify-center w-8 h-8 rounded-full transition-colors ${
              menuOpen ? "bg-white/20 text-white" : "text-white hover:bg-white/20"
            }`}
            title="More options"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
          >
            <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
              <path d="M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z" />
            </svg>
          </button>

          {menuOpen && (
            <div
              className="absolute right-0 top-full mt-2 w-48 bg-surface rounded-lg shadow-2xl border border-edge py-1"
              style={{ zIndex: 9999 }}
              role="menu"
            >
              {/* Info lives ONLY here (#44). #30 added this entry and left the
                  standalone top-bar button in place; the button is gone now, so
                  this is the one way in (the swipe-up gesture aside). */}
              <button
                onClick={() => { setShowInfoPanel(!showInfoPanel); setMenuOpen(false); }}
                className={menuItemClass}
                role="menuitem"
              >
                <svg className="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                </svg>
                Info
              </button>
              {!isBackupServer && (
                <button
                  onClick={() => { setShowTagPanel(!showTagPanel); setMenuOpen(false); }}
                  className={menuItemClass}
                  role="menuitem"
                >
                  <AppIcon name="tag" />
                  Tags
                </button>
              )}
              {hasSlideshow && onStartSlideshow && (
                <button
                  onClick={() => { onStartSlideshow(); setMenuOpen(false); }}
                  className={menuItemClass}
                  role="menuitem"
                >
                  <svg className="w-4 h-4 shrink-0" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
                  Start Slideshow
                </button>
              )}
              <button
                onClick={() => { onDownload(); setMenuOpen(false); }}
                disabled={!mediaUrl || isRenderingVideo}
                className={`${menuItemClass} disabled:opacity-50 disabled:cursor-wait`}
                role="menuitem"
              >
                {isRenderingVideo
                  ? <div className="w-4 h-4 shrink-0 border-2 border-fg-muted/40 border-t-fg-muted rounded-full animate-spin" />
                  : <AppIcon name="download" />}
                {isRenderingVideo ? "Converting…" : "Download"}
              </button>
              {!isBackupServer && (
                <>
                  <div className="border-t border-edge my-1" />
                  {canRemoveFromAlbum ? (
                    <button
                      onClick={() => { onRemoveFromAlbum(); setMenuOpen(false); }}
                      className="w-full text-left px-4 py-2 text-sm text-orange-500 dark:text-orange-400 hover:bg-orange-50 dark:hover:bg-orange-900/30 flex items-center gap-2 transition-colors"
                      role="menuitem"
                    >
                      <svg className="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M15 12H9m12 0a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                      Remove from album
                    </button>
                  ) : (
                    <button
                      onClick={() => { onDelete(); setMenuOpen(false); }}
                      className="w-full text-left px-4 py-2 text-sm text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30 flex items-center gap-2 transition-colors"
                      role="menuitem"
                    >
                      <AppIcon name="trashcan" />
                      Delete
                    </button>
                  )}
                </>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
    </div>
  );
}
