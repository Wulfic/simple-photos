/**
 * Full-screen modal for browsing and selecting server-side directories.
 *
 * Used by admin storage configuration and import features to let the user
 * navigate the server's filesystem and pick a target folder.
 */
import { useState } from "react";
import { Modal } from "./ui";

interface FolderBrowserModalProps {
  open: boolean;
  onClose: () => void;
  onSelect: (path: string) => void;
  browsePath: string;
  browseParent: string | null;
  browseDirs: Array<{ name: string; path: string }>;
  browseWritable: boolean;
  browseLoading: boolean;
  browseDirectory: (path?: string) => void;
}

/**
 * Full-screen modal folder browser for selecting a storage directory.
 * Replaces the inline folder browser in the setup wizard with a cleaner UX.
 */
export default function FolderBrowserModal({
  open,
  onClose,
  onSelect,
  browsePath,
  browseParent,
  browseDirs,
  browseWritable,
  browseLoading,
  browseDirectory,
}: FolderBrowserModalProps) {
  const [manualPath, setManualPath] = useState("");
  const [showManual, setShowManual] = useState(false);

  if (!open) return null;

  function handleManualGo() {
    if (!manualPath.trim()) return;
    browseDirectory(manualPath.trim());
  }

  return (
    <Modal onClose={onClose} size="lg" panelClassName="max-h-[80vh] flex flex-col overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-edge">
          <div>
            <h3 className="text-lg font-semibold text-fg">
              Choose Storage Folder
            </h3>
            <p className="text-xs text-fg-muted mt-0.5">
              Navigate to the folder where photos will be stored
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-sunken dark:hover:bg-white/10 transition-colors"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Current path breadcrumb */}
        <div className="bg-canvas px-5 py-2.5 flex items-center gap-2 border-b border-edge">
          <svg className="w-4 h-4 text-fg-muted shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
          </svg>
          <span className="font-mono text-xs text-fg-muted truncate flex-1">
            {browsePath}
          </span>
          {browseLoading && (
            <div className="w-4 h-4 border-2 border-accent-500 border-t-transparent rounded-full animate-spin shrink-0" />
          )}
          {/* Writable indicator */}
          <span className={`flex items-center gap-1 text-xs shrink-0 ${browseWritable ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}`}>
            <span className={`w-2 h-2 rounded-full ${browseWritable ? "bg-green-500" : "bg-red-500"}`} />
            {browseWritable ? "Writable" : "Read-only"}
          </span>
        </div>

        {/* Directory listing */}
        <div className="flex-1 overflow-y-auto min-h-0">
          {/* Parent directory */}
          {browseParent && (
            <button
              type="button"
              onClick={() => browseDirectory(browseParent)}
              disabled={browseLoading}
              className="w-full text-left px-5 py-2.5 hover:bg-accent-50 dark:hover:bg-accent-900/20 flex items-center gap-3 text-sm border-b border-edge/50 transition-colors disabled:opacity-50"
            >
              <svg className="w-4 h-4 text-accent-500 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3" />
              </svg>
              <span className="text-accent-600 dark:text-accent-400 font-medium">..</span>
              <span className="text-fg-muted text-xs ml-auto">Parent folder</span>
            </button>
          )}

          {/* Empty state */}
          {browseDirs.length === 0 && !browseLoading && (
            <div className="px-5 py-10 text-center text-fg-muted text-sm">
              No subdirectories
            </div>
          )}

          {/* Subdirectories */}
          {browseDirs.map((dir) => (
            <button
              key={dir.path}
              type="button"
              onClick={() => browseDirectory(dir.path)}
              disabled={browseLoading}
              className="w-full text-left px-5 py-2.5 hover:bg-accent-50 dark:hover:bg-accent-900/20 flex items-center gap-3 text-sm border-b border-edge/50 last:border-b-0 transition-colors disabled:opacity-50"
            >
              <svg className="w-4 h-4 text-yellow-500 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
              </svg>
              <span className="text-fg truncate">{dir.name}</span>
            </button>
          ))}
        </div>

        {/* Manual path entry */}
        <div className="border-t border-edge px-5 py-3">
          <button
            type="button"
            onClick={() => setShowManual((v) => !v)}
            className="text-xs text-accent-600 hover:text-accent-800 dark:text-accent-400 dark:hover:text-accent-300 flex items-center gap-1 mb-2"
          >
            <svg className={`w-3 h-3 transition-transform ${showManual ? "rotate-90" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
            </svg>
            Enter path manually
          </button>
          {showManual && (
            <div className="flex gap-2">
              <input
                type="text"
                value={manualPath}
                onChange={(e) => setManualPath(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleManualGo(); }}
                className="input flex-1 font-mono"
                placeholder="/path/to/storage"
              />
              <button
                type="button"
                onClick={handleManualGo}
                disabled={browseLoading}
                className="btn btn-secondary btn-md"
              >
                Go
              </button>
            </div>
          )}
        </div>

        {/* Footer actions */}
        <div className="flex items-center justify-between gap-3 px-5 py-4 border-t border-edge bg-canvas">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-sm text-fg-muted hover:text-fg transition-colors"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onSelect(browsePath)}
            disabled={!browseWritable}
            className="btn btn-primary btn-md"
          >
            Select This Folder
          </button>
        </div>
    </Modal>
  );
}
