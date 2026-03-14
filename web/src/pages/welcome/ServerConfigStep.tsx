/** Wizard step — configure storage path, photo directory, and server URL. */
import { useState, useCallback } from "react";
import { api } from "../../api/client";
import FolderBrowserModal from "../../components/FolderBrowserModal";
import type { WizardStep, ServerRole } from "./types";
import { getErrorMessage } from "../../utils/formatters";

export interface ServerConfigStepProps {
  // Port state
  portInput: string;
  setPortInput: (v: string) => void;
  portSaved: boolean;
  setPortSaved: (v: boolean) => void;
  serverPort: number;
  originalPort: number;
  handleSavePort: () => void;

  // Storage state
  storagePath: string;
  storageConfirmed: boolean;
  handleSelectStoragePath: () => void;

  // Shared
  loading: boolean;
  error: string;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  /** Directly set the storage path to save */
  setStoragePathDirect: (path: string) => void;
  /** Server role — determines next step */
  serverRole?: ServerRole;
}

export default function ServerConfigStep({
  portInput,
  setPortInput,
  portSaved,
  setPortSaved,
  serverPort,
  originalPort,
  handleSavePort,
  storagePath,
  storageConfirmed,
  handleSelectStoragePath,
  loading,
  error,
  setStep,
  setError,
  setStoragePathDirect,
  serverRole,
}: ServerConfigStepProps) {
  const [pathInput, setPathInput] = useState(storagePath || "");
  const [saving, setSaving] = useState(false);

  // ── Folder browser state ────────────────────────────────────────────────
  const [browserOpen, setBrowserOpen] = useState(false);
  const [browsePath, setBrowsePath] = useState("/");
  const [browseParent, setBrowseParent] = useState<string | null>(null);
  const [browseDirs, setBrowseDirs] = useState<Array<{ name: string; path: string }>>([]);
  const [browseWritable, setBrowseWritable] = useState(false);
  const [browseLoading, setBrowseLoading] = useState(false);

  const browseDirectory = useCallback(async (path?: string) => {
    setBrowseLoading(true);
    try {
      const res = await api.admin.browseDirectory(path);
      setBrowsePath(res.current_path);
      setBrowseParent(res.parent_path);
      setBrowseDirs(res.directories);
      setBrowseWritable(res.writable);
    } catch {
      // If browsing fails, just stay on current path
    } finally {
      setBrowseLoading(false);
    }
  }, []);

  function handleOpenBrowser() {
    // Start browsing from the current path input, or root
    const startPath = pathInput.trim() || storagePath || "/";
    browseDirectory(startPath);
    setBrowserOpen(true);
  }

  function handleBrowserSelect(selectedPath: string) {
    setPathInput(selectedPath);
    setBrowserOpen(false);
  }

  async function handleSavePath() {
    const trimmed = pathInput.trim();
    if (!trimmed) {
      setError("Please enter a storage path.");
      return;
    }
    setSaving(true);
    setError("");
    try {
      setStoragePathDirect(trimmed);
      await new Promise((r) => setTimeout(r, 50));
      handleSelectStoragePath();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to set storage path."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
        Server Configuration
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-5">
        Configure the server port and choose where your photos
        will be stored.
      </p>

      {/* ── Server Port ─────────────────────────────────────────── */}
      <div className="mb-6">
        <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2 flex items-center gap-2">
          <svg className="w-4 h-4 text-gray-500 dark:text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z" />
          </svg>
          Server Port
        </h3>
        <div className="flex items-center gap-3">
          <input
            type="number"
            min={1024}
            max={65535}
            value={portInput}
            onChange={(e) => {
              setPortInput(e.target.value);
              setPortSaved(false);
            }}
            className="w-28 border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 font-mono text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent text-center"
            placeholder="8080"
          />
          <button
            type="button"
            onClick={handleSavePort}
            disabled={loading || portSaved || portInput === String(serverPort)}
            className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
          >
            {portSaved ? "Saved ✓" : "Save"}
          </button>
          {serverPort !== originalPort && (
            <span className="text-xs text-amber-600 dark:text-amber-400">
              Restart required after setup
            </span>
          )}
        </div>
        <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
          Range: 1024–65535. Currently running on port {originalPort}.
        </p>
      </div>

      {/* ── Storage Location ────────────────────────────────────── */}
      <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2 flex items-center gap-2">
        <svg className="w-4 h-4 text-gray-500 dark:text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
        </svg>
        Photo Storage Location
      </h3>
      <p className="text-gray-500 dark:text-gray-400 text-xs mb-3">
        Enter the full path to where your photos and videos will be stored.
        This can be a local folder, mounted network share, or external drive.
      </p>

      {/* Current / selected path display */}
      {storageConfirmed && (
        <div className="bg-green-50 dark:bg-green-900/20 rounded-lg p-4 mb-4 flex items-center gap-2">
          <svg className="w-4 h-4 text-green-600 dark:text-green-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
          </svg>
          <div>
            <span className="text-xs font-medium text-green-700 dark:text-green-300 block">Storage path saved</span>
            <span className="font-mono text-sm text-green-800 dark:text-green-200 break-all">{storagePath}</span>
          </div>
        </div>
      )}

      {/* Path input */}
      <div className="flex gap-2 mb-3">
        <input
          type="text"
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") handleSavePath(); }}
          placeholder="/path/to/photo/storage"
          className="flex-1 border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 font-mono text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent dark:bg-gray-700 dark:text-gray-200"
        />
        <button
          type="button"
          onClick={handleOpenBrowser}
          className="px-3 py-2 bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors flex items-center gap-1.5"
          title="Browse server directories"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
          </svg>
          Browse
        </button>
        <button
          type="button"
          onClick={handleSavePath}
          disabled={saving || !pathInput.trim()}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
        >
          {saving ? "Saving…" : "Set Path"}
        </button>
      </div>

      <p className="text-xs text-gray-400 dark:text-gray-500 mb-4">
        The directory will be created if it doesn't exist. Must be writable by the server process.
      </p>

      {error && (
        <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg mb-4">
          {error}
        </div>
      )}

      {/* Continue button */}
      <button
        onClick={() => {
          setError("");
          setStep(serverRole === "backup" ? "complete" : "ssl");
        }}
        className="w-full bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors"
      >
        {storageConfirmed ? "Continue →" : "Keep Default & Continue →"}
      </button>

      <FolderBrowserModal
        open={browserOpen}
        onClose={() => setBrowserOpen(false)}
        onSelect={handleBrowserSelect}
        browsePath={browsePath}
        browseParent={browseParent}
        browseDirs={browseDirs}
        browseWritable={browseWritable}
        browseLoading={browseLoading}
        browseDirectory={browseDirectory}
      />
    </div>
  );
}
