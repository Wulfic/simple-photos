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
  handleSelectStoragePath: (path?: string) => void;

  // Shared
  loading: boolean;
  error: string;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  /** Server role — determines next step */
  serverRole?: ServerRole;
  /** Install type — determines back navigation */
  installType?: "fresh" | "restore" | null;
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
  serverRole,
  installType,
}: ServerConfigStepProps) {
  const [pathInput, setPathInput] = useState(storagePath || "");
  const [saving, setSaving] = useState(false);

  // ── SMB / network-share state ───────────────────────────────────────────
  // When the user types an SMB-style address into the storage path field
  // (smb://host/share, \\host\share, or //host/share/sub) we don't treat it
  // as a local path — instead we pop a small credentials modal so the wizard
  // can collect the username / password / domain and then call
  // `configureSmbStorage`, which mounts the share on the server side.
  const [smbModalOpen, setSmbModalOpen] = useState(false);
  const [smbUser, setSmbUser] = useState("");
  const [smbPass, setSmbPass] = useState("");
  const [smbDomain, setSmbDomain] = useState("");
  const [smbAnonymous, setSmbAnonymous] = useState(false);
  const [smbBusy, setSmbBusy] = useState(false);
  const [smbStatus, setSmbStatus] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);

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

  async function handleNativePick() {
    setError("");

    // 1. Preferred: a real native OS folder dialog spawned by the server on
    //    the machine it runs on. For the common desktop/localhost install the
    //    server and browser are the same computer, so this pops the normal
    //    Windows/macOS/Linux folder chooser and returns an absolute path
    //    directly — no in-browser file browser, no sentinel round-trip.
    try {
      const res = await api.admin.pickDirectory();
      if (res?.path) {
        setPathInput(res.path);
        return;
      }
    } catch (err: unknown) {
      const msg = (err as { message?: string })?.message ?? "";
      // User closed the dialog → do nothing, leave the field as-is.
      if (/cancel/i.test(msg)) return;
      // "native_picker_unavailable" (headless server, service session, or a
      // remote browser) → fall through to the browser-based pickers below.
    }

    // 2. Browser File System Access API (Chrome/Edge/Opera). Useful when the
    //    UI is opened from a *different* machine than the server.
    if (
      typeof window !== "undefined" &&
      "showDirectoryPicker" in window
    ) {
      setError("");
      let dirHandle: FileSystemDirectoryHandle | null = null;
      const sentinelName = `sp-picker-${crypto.randomUUID()}.tmp`;
      try {
        // This opens the exact same native OS dialog as <input type="file">
        dirHandle = await (
          window as Window &
            typeof globalThis & {
              showDirectoryPicker: (
                o?: object
              ) => Promise<FileSystemDirectoryHandle>;
            }
        ).showDirectoryPicker({ mode: "readwrite" });

        // Write a tiny sentinel file so the server can locate the directory.
        const fileHandle = await dirHandle.getFileHandle(sentinelName, {
          create: true,
        });
        const writable = await fileHandle.createWritable();
        await writable.write("x");
        await writable.close();

        // Ask the server to resolve the absolute path.
        const res = await api.admin.resolveStorageSentinel(sentinelName);
        setPathInput(res.path);
      } catch (err: unknown) {
        const e = err as { name?: string };
        if (e?.name === "AbortError") return; // user pressed Cancel
        // Any other failure (write permissions, server unreachable, etc.)
        // → fall back to the in-browser directory browser.
        handleOpenBrowser();
      } finally {
        // Best-effort cleanup — ignore errors (e.g. handle already released).
        if (dirHandle) {
          dirHandle.removeEntry(sentinelName).catch(() => {});
        }
      }
      return;
    }

    // Browser doesn't support showDirectoryPicker — fall back to the
    // in-browser FolderBrowserModal (Firefox, Safari, older browsers).
    handleOpenBrowser();
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
    // SMB-style address? Pop the credentials modal instead of trying to
    // treat the address as a local directory.
    if (isSmbAddress(trimmed)) {
      setSmbStatus(null);
      setSmbModalOpen(true);
      return;
    }
    setSaving(true);
    setError("");
    try {
      // Pass the path directly — avoids the React setState closure timing issue
      await handleSelectStoragePath(trimmed);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to set storage path."));
    } finally {
      setSaving(false);
    }
  }

  // ── SMB handlers ────────────────────────────────────────────────────────

  function smbPayload() {
    const trimmed = pathInput.trim();
    if (!trimmed) {
      throw new Error("Enter the SMB share address.");
    }
    return {
      address: trimmed,
      username: smbAnonymous ? undefined : smbUser.trim() || undefined,
      password: smbAnonymous ? undefined : smbPass || undefined,
      domain: smbAnonymous ? undefined : smbDomain.trim() || undefined,
    };
  }

  async function handleTestSmb() {
    setSmbBusy(true);
    setSmbStatus(null);
    setError("");
    try {
      const payload = smbPayload();
      const res = await api.admin.testSmbConnection(payload);
      setSmbStatus({ kind: "ok", msg: res.message });
    } catch (err: unknown) {
      setSmbStatus({ kind: "err", msg: getErrorMessage(err, "SMB test failed.") });
    } finally {
      setSmbBusy(false);
    }
  }

  async function handleConnectSmb() {
    setSmbBusy(true);
    setSmbStatus(null);
    setError("");
    try {
      const payload = smbPayload();
      const res = await api.admin.configureSmbStorage(payload);
      // The server has already swapped its storage root. Reflect that in the
      // wizard state so the green "Storage path saved" panel renders.
      setPathInput(res.storage_path);
      await handleSelectStoragePath(res.storage_path);
      setSmbStatus({ kind: "ok", msg: res.message });
      setSmbModalOpen(false);
      // Wipe credentials from React state once the share is mounted — they
      // live encrypted in config.toml on the server now.
      setSmbPass("");
    } catch (err: unknown) {
      const msg = getErrorMessage(err, "Failed to mount SMB share.");
      setSmbStatus({ kind: "err", msg });
    } finally {
      setSmbBusy(false);
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
        This can be a local folder, an external drive, or a network share —
        for SMB, just type{" "}
        <code className="font-mono text-[11px]">smb://host/share</code> or{" "}
        <code className="font-mono text-[11px]">{"\\\\host\\share"}</code> and
        we&apos;ll prompt for credentials.
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
          onClick={handleNativePick}
          className="px-3 py-2 bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors flex items-center gap-1.5"
          title="Open system folder picker"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
          </svg>
          Browse…
        </button>
        <button
          type="button"
          onClick={handleSavePath}
          disabled={saving || !pathInput.trim()}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
        >
          {saving
            ? "Saving…"
            : isSmbAddress(pathInput)
              ? "Connect…"
              : "Set Path"}
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

      {/* Navigation */}
      <div className="flex gap-3">
        {serverRole === "primary" && installType === "fresh" && (
          <button
            onClick={() => {
              setError("");
              setStep("admin-2fa");
            }}
            className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors"
          >
            ← Back
          </button>
        )}
        <button
          onClick={() => {
            setError("");
            // Both primary and backup flows now run through the SSL step
            // so every server can opt into Let's Encrypt or a manual cert.
            setStep("ssl");
          }}
          className={`${serverRole === "primary" && installType === "fresh" ? "flex-[2]" : "w-full"} bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors`}
        >
          Continue →
        </button>
      </div>

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

      {/* SMB credentials prompt — opens automatically when the user types
          an SMB-style address into the storage path field and hits Set Path. */}
      {smbModalOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          onClick={(e) => {
            if (e.target === e.currentTarget && !smbBusy) setSmbModalOpen(false);
          }}
        >
          <div className="bg-white dark:bg-gray-800 rounded-xl shadow-2xl w-full max-w-md p-6">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-1">
              Connect to network share
            </h3>
            <p className="font-mono text-xs text-gray-500 dark:text-gray-400 mb-4 break-all">
              {pathInput.trim()}
            </p>

            <label className="flex items-center gap-2 mb-3 text-sm text-gray-700 dark:text-gray-300">
              <input
                type="checkbox"
                checked={smbAnonymous}
                onChange={(e) => setSmbAnonymous(e.target.checked)}
                className="rounded"
              />
              Connect as guest (no credentials)
            </label>

            <div className={`space-y-3 ${smbAnonymous ? "opacity-50 pointer-events-none" : ""}`}>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
                  Username
                </label>
                <input
                  type="text"
                  value={smbUser}
                  onChange={(e) => setSmbUser(e.target.value)}
                  autoComplete="off"
                  autoFocus
                  className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:text-gray-200"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
                  Password
                </label>
                <input
                  type="password"
                  value={smbPass}
                  onChange={(e) => setSmbPass(e.target.value)}
                  autoComplete="new-password"
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !smbBusy) handleConnectSmb();
                  }}
                  className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:text-gray-200"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
                  Domain <span className="text-gray-400 font-normal">(optional, AD only)</span>
                </label>
                <input
                  type="text"
                  value={smbDomain}
                  onChange={(e) => setSmbDomain(e.target.value)}
                  className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:text-gray-200"
                />
              </div>
            </div>

            {smbStatus && (
              <div
                className={`mt-3 text-xs p-2 rounded-lg ${
                  smbStatus.kind === "ok"
                    ? "bg-green-50 dark:bg-green-900/30 text-green-700 dark:text-green-300"
                    : "bg-red-50 dark:bg-red-900/30 text-red-700 dark:text-red-300"
                }`}
              >
                {smbStatus.msg}
              </div>
            )}

            <p className="text-[11px] text-gray-400 dark:text-gray-500 mt-4 leading-snug">
              The server stores the password encrypted at rest (AES-GCM keyed off
              the JWT secret) and remounts the share on every restart. Requires{" "}
              <code className="font-mono">cifs-utils</code> on the host.
            </p>

            <div className="flex gap-2 mt-5">
              <button
                type="button"
                onClick={() => setSmbModalOpen(false)}
                disabled={smbBusy}
                className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 text-sm font-medium transition-colors"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={handleTestSmb}
                disabled={smbBusy}
                className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 text-sm font-medium transition-colors"
              >
                {smbBusy ? "…" : "Test"}
              </button>
              <button
                type="button"
                onClick={handleConnectSmb}
                disabled={smbBusy}
                className="flex-1 bg-blue-600 text-white py-2 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
              >
                {smbBusy ? "Mounting…" : "Connect"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Detect whether a path string looks like an SMB / network-share address.
 * Matches the same three forms the server's parser accepts: `smb://...`,
 * Windows UNC `\\host\share`, and `//host/share/...`.
 */
function isSmbAddress(raw: string): boolean {
  const s = raw.trim();
  if (!s) return false;
  if (/^smb:\/\//i.test(s)) return true;
  if (s.startsWith("\\\\")) return true;
  // POSIX-style only counts when there's a host/share component (avoids
  // false-positives on `//` typed by mistake).
  if (s.startsWith("//") && s.length > 2 && !s.startsWith("///")) {
    const rest = s.slice(2);
    return rest.includes("/");
  }
  return false;
}
