import type { WizardStep } from "./types";

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
  browsePath: string;
  browseParent: string | null;
  browseDirs: Array<{ name: string; path: string }>;
  browseWritable: boolean;
  browseLoading: boolean;
  manualPathInput: string;
  setManualPathInput: (v: string) => void;
  showManualInput: boolean;
  setShowManualInput: (v: boolean | ((prev: boolean) => boolean)) => void;
  browseDirectory: (path?: string) => void;
  handleSelectStoragePath: () => void;
  handleManualPathGo: () => void;

  // Shared
  loading: boolean;
  error: string;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
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
  browsePath,
  browseParent,
  browseDirs,
  browseWritable,
  browseLoading,
  manualPathInput,
  setManualPathInput,
  showManualInput,
  setShowManualInput,
  browseDirectory,
  handleSelectStoragePath,
  handleManualPathGo,
  loading,
  error,
  setStep,
  setError,
}: ServerConfigStepProps) {
  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
        Server Configuration
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-5">
        Configure the server port and choose where your encrypted photos
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
        Choose where your encrypted photos and videos will be stored.
        Local folder, mounted network share, or external drive.
      </p>

      {/* Current / selected path indicator */}
      <div className="bg-gray-50 dark:bg-gray-900 rounded-lg p-3 mb-4">
        <div className="flex items-center justify-between">
          <div>
            <span className="text-xs font-medium text-gray-500 dark:text-gray-400 block mb-0.5">
              {storageConfirmed ? "Selected path" : "Current path"}
            </span>
            <span className="font-mono text-sm text-gray-800 dark:text-gray-200 break-all">
              {storagePath || browsePath || "Loading\u2026"}
            </span>
          </div>
          {storageConfirmed && (
            <span className="text-green-600 dark:text-green-400 text-sm font-medium flex items-center gap-1">
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              Saved
            </span>
          )}
        </div>
      </div>

      {/* Directory browser */}
      <div className="border border-gray-200 dark:border-gray-700 rounded-lg mb-4 overflow-hidden">
        {/* Breadcrumb / current browse path */}
        <div className="bg-gray-100 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-700 px-3 py-2 flex items-center gap-2">
          <svg className="w-4 h-4 text-gray-500 dark:text-gray-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
          </svg>
          <span className="font-mono text-xs text-gray-700 dark:text-gray-300 truncate flex-1">
            {browsePath}
          </span>
          {browseLoading && (
            <div className="w-4 h-4 border-2 border-blue-500 dark:border-blue-400 border-t-transparent rounded-full animate-spin" />
          )}
        </div>

        {/* Directory list */}
        <div className="max-h-60 overflow-y-auto">
          {/* Up / parent directory */}
          {browseParent && (
            <button
              type="button"
              onClick={() => browseDirectory(browseParent)}
              disabled={browseLoading}
              className="w-full text-left px-3 py-2 hover:bg-blue-50 dark:hover:bg-blue-900/30 dark:bg-blue-900/30 flex items-center gap-2 text-sm border-b border-gray-100 dark:border-gray-700 transition-colors disabled:opacity-50"
            >
              <svg className="w-4 h-4 text-blue-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3" />
              </svg>
              <span className="text-blue-600 font-medium">..</span>
              <span className="text-gray-400 text-xs ml-auto">
                Parent folder
              </span>
            </button>
          )}

          {/* Subdirectories */}
          {browseDirs.length === 0 && !browseLoading && (
            <div className="px-3 py-6 text-center text-gray-400 text-sm">
              No subdirectories
            </div>
          )}
          {browseDirs.map((dir) => (
            <button
              key={dir.path}
              type="button"
              onClick={() => browseDirectory(dir.path)}
              disabled={browseLoading}
              className="w-full text-left px-3 py-2 hover:bg-blue-50 dark:hover:bg-blue-900/30 dark:bg-blue-900/30 flex items-center gap-2 text-sm border-b border-gray-100 dark:border-gray-700 last:border-b-0 transition-colors disabled:opacity-50"
            >
              <svg className="w-4 h-4 text-yellow-500 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
              </svg>
              <span className="text-gray-800 dark:text-gray-200 truncate">{dir.name}</span>
            </button>
          ))}
        </div>
      </div>

      {/* Writable indicator */}
      <div className="flex items-center gap-2 mb-3 text-xs">
        <span className={`w-2 h-2 rounded-full ${browseWritable ? "bg-green-500" : "bg-red-500"}`} />
        <span className={browseWritable ? "text-green-700 dark:text-green-400" : "text-red-700 dark:text-red-400"}>
          {browseWritable
            ? "This directory is writable"
            : "This directory is not writable — choose a different location"}
        </span>
      </div>

      {/* Manual path entry toggle */}
      <div className="mb-4">
        <button
          type="button"
          onClick={() => setShowManualInput((v: boolean) => !v)}
          className="text-xs text-blue-600 hover:text-blue-800 dark:hover:text-blue-300 dark:text-blue-300 flex items-center gap-1"
        >
          <svg className={`w-3 h-3 transition-transform ${showManualInput ? "rotate-90" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
          </svg>
          Enter path manually
        </button>
        {showManualInput && (
          <div className="flex gap-2 mt-2">
            <input
              type="text"
              value={manualPathInput}
              onChange={(e) => setManualPathInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleManualPathGo();
              }}
              className="flex-1 border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 font-mono text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
              placeholder="/path/to/storage"
            />
            <button
              type="button"
              onClick={handleManualPathGo}
              disabled={browseLoading}
              className="px-4 py-2 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors disabled:opacity-50"
            >
              Go
            </button>
          </div>
        )}
      </div>

      {error && (
        <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg mb-4">
          {error}
        </div>
      )}

      {/* Action buttons */}
      <div className="flex gap-3">
        <button
          type="button"
          onClick={handleSelectStoragePath}
          disabled={loading || !browseWritable || browsePath === storagePath}
          className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
        >
          {loading
            ? "Saving\u2026"
            : browsePath === storagePath
              ? "Current location selected"
              : "Use This Location"}
        </button>
      </div>

      {/* Continue button — always visible after confirming or if using default */}
      <button
        onClick={() => {
          setError("");
          setStep("backup");
        }}
        className="w-full mt-3 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
      >
        {storageConfirmed ? "Continue →" : "Keep Default & Continue →"}
      </button>
    </div>
  );
}
