/**
 * Viewer top toolbar — back, info, edit, favorite, download, delete/remove buttons.
 */
import type { MediaType } from "../../db";
import AppIcon from "../AppIcon";

export interface ViewerTopBarProps {
  editMode: boolean;
  showOverlay: boolean;
  showInfoPanel: boolean;
  setShowInfoPanel: (v: boolean) => void;
  mediaType: MediaType;
  mediaUrl: string | null;
  isFavorite: boolean;
  isBackupServer: boolean;
  isRenderingVideo: boolean;
  albumId?: string;
  onBack: () => void;
  onToggleEdit: () => void;
  onToggleFavorite: () => void;
  onDownload: () => void;
  onDelete: () => void;
  onRemoveFromAlbum: () => void;
}

export default function ViewerTopBar({
  editMode,
  showOverlay,
  showInfoPanel,
  setShowInfoPanel,
  mediaType,
  mediaUrl,
  isFavorite,
  isBackupServer,
  isRenderingVideo,
  albumId,
  onBack,
  onToggleEdit,
  onToggleFavorite,
  onDownload,
  onDelete,
  onRemoveFromAlbum,
}: ViewerTopBarProps) {
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
        <button
          onClick={() => setShowInfoPanel(!showInfoPanel)}
          className={`flex items-center justify-center w-8 h-8 rounded-full transition-colors ${
            showInfoPanel ? "bg-blue-600 text-white" : "text-white hover:bg-white/20"
          }`}
          title="Info"
        >
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
        </button>
        {(mediaType === "photo" || mediaType === "video" || mediaType === "audio") && !isBackupServer && (
          <button
            onClick={onToggleEdit}
            className={`flex items-center gap-1 px-2 py-1 rounded text-sm font-medium transition-colors ${
              editMode ? "bg-blue-600 text-white" : "text-white hover:bg-white/20"
            }`}
            title="Edit"
          >Edit</button>
        )}
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
        <button
          onClick={onDownload}
          className="text-white hover:text-gray-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors disabled:opacity-50 disabled:cursor-wait"
          disabled={!mediaUrl || isRenderingVideo}
          title={isRenderingVideo ? "Converting…" : "Download"}
        >
          {isRenderingVideo
            ? <div className="w-4 h-4 border-2 border-white/40 border-t-white rounded-full animate-spin" />
            : <AppIcon name="download" size="w-5 h-5" themed={false} className="invert" />}
        </button>
        {!isBackupServer && (
        <>
        {albumId ? (
          <button
            onClick={onRemoveFromAlbum}
            className="text-orange-400 hover:text-orange-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
            title="Remove from album"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M15 12H9m12 0a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
          </button>
        ) : (
          <button
            onClick={onDelete}
            className="text-red-400 hover:text-red-300 flex items-center justify-center w-8 h-8 rounded-full hover:bg-white/20 transition-colors"
            title="Delete"
          >
            <AppIcon name="trashcan" size="w-5 h-5" themed={false} className="invert" />
          </button>
        )}
        </>
        )}
      </div>
    </div>
    </div>
  );
}
