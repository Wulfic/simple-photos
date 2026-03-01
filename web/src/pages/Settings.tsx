import { useState, useEffect, useCallback } from "react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import { useProcessingStore } from "../store/processing";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { useIsAdmin } from "../hooks/useIsAdmin";
import StorageStatsSection from "../components/StorageStatsSection";
import UserManagement from "../components/settings/UserManagement";
import SslSettings from "../components/settings/SslSettings";
import AccountSection from "../components/settings/AccountSection";
import { useMigrationWorker } from "../hooks/useMigrationWorker";

export default function Settings() {
  const { username } = useAuthStore();
  const isAdmin = useIsAdmin();
  const { startTask, endTask } = useProcessingStore();

  // ── General state ────────────────────────────────────────────────────────
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");
  const [loading, setLoading] = useState(false);

  // ── Encryption state ─────────────────────────────────────────────────────
  const [encryptionMode, setEncryptionMode] = useState<"plain" | "encrypted">("plain");
  const [migrationStatus, setMigrationStatus] = useState("idle");
  const [migrationTotal, setMigrationTotal] = useState(0);
  const [migrationCompleted, setMigrationCompleted] = useState(0);
  const [migrationError, setMigrationError] = useState<string | null>(null);
  const [encryptionLoading, setEncryptionLoading] = useState(true);
  const [togglingEncryption, setTogglingEncryption] = useState(false);
  const [showEncryptionWarning, setShowEncryptionWarning] = useState(false);

  // ── Backup recovery state ────────────────────────────────────────────────
  const [showRecoverWarning, setShowRecoverWarning] = useState(false);
  const { backupServers, loaded: backupLoaded, recovering, setRecovering, setBackupServers, setLoaded: setBackupLoaded, viewMode, setViewMode, activeBackupServerId, setActiveBackupServerId } = useBackupStore();

  // ── Manual backup server state ────────────────────────────────────────────
  const [showAddBackupServer, setShowAddBackupServer] = useState(false);
  const [backupServerName, setBackupServerName] = useState("");
  const [backupServerAddress, setBackupServerAddress] = useState("");
  const [backupServerApiKey, setBackupServerApiKey] = useState("");
  const [backupServerFrequency, setBackupServerFrequency] = useState("24");
  const [addingBackupServer, setAddingBackupServer] = useState(false);

  // ── Scan state (admin, plain mode) ──────────────────────────────────────
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<string | null>(null);

  // ── Storage stats state ─────────────────────────────────────────────────
  type StorageStats = {
    photo_bytes: number; photo_count: number;
    video_bytes: number; video_count: number;
    other_blob_bytes: number; other_blob_count: number;
    plain_bytes: number; plain_count: number;
    user_total_bytes: number;
    fs_total_bytes: number; fs_free_bytes: number;
  };
  const [storageStats, setStorageStats] = useState<StorageStats | null>(null);
  const [storageLoading, setStorageLoading] = useState(true);

  // ── Encryption handlers ──────────────────────────────────────────────────

  const loadEncryptionSettings = useCallback(async () => {
    try {
      const res = await api.encryption.getSettings();
      setEncryptionMode(res.encryption_mode as "plain" | "encrypted");
      setMigrationStatus(res.migration_status);
      setMigrationTotal(res.migration_total);
      setMigrationCompleted(res.migration_completed);
      setMigrationError(res.migration_error);
    } catch {
      // Settings might not exist yet (pre-migration)
    } finally {
      setEncryptionLoading(false);
    }
  }, []);

  // Load backup servers on mount
  const loadBackupServers = useCallback(async () => {
    try {
      const res = await api.backup.listServers();
      setBackupServers(res.servers);
    } catch {
      // Ignore if backup isn't configured
    } finally {
      setBackupLoaded(true);
    }
  }, [setBackupServers, setBackupLoaded]);

  // Fetch encryption settings and backup servers on mount
  useEffect(() => {
    loadEncryptionSettings();
    loadBackupServers();
    loadStorageStats();
  }, [loadEncryptionSettings, loadBackupServers]);

  async function loadStorageStats() {
    setStorageLoading(true);
    try {
      const stats = await api.storageStats.get();
      setStorageStats(stats);
    } catch {
      // Endpoint may not be available — silently skip
    } finally {
      setStorageLoading(false);
    }
  }

  // Poll migration progress when a migration is active
  useEffect(() => {
    if (migrationStatus !== "encrypting" && migrationStatus !== "decrypting") return;
    startTask("encryption");
    const interval = setInterval(loadEncryptionSettings, 3000);
    return () => {
      clearInterval(interval);
      endTask("encryption");
    };
  }, [migrationStatus, loadEncryptionSettings, startTask, endTask]);

  useMigrationWorker(migrationStatus, loadEncryptionSettings);

  async function handleToggleEncryption() {
    setShowEncryptionWarning(false);
    setTogglingEncryption(true);
    setError("");
    try {
      const newMode = encryptionMode === "plain" ? "encrypted" : "plain";
      const res = await api.encryption.setMode(newMode);
      setEncryptionMode(newMode);
      setSuccess(res.message);
      // Reload to get migration status
      await loadEncryptionSettings();
    } catch (err: any) {
      setError(err.message);
    } finally {
      setTogglingEncryption(false);
    }
  }

  async function handleRecover() {
    if (backupServers.length === 0) return;
    setShowRecoverWarning(false);
    setRecovering(true);
    startTask("recovery");
    setError("");
    try {
      const target = backupServers.find((s) => s.enabled) ?? backupServers[0];
      const res = await api.backup.recover(target.id);
      setSuccess(res.message);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setRecovering(false);
      endTask("recovery");
    }
  }

  async function handleAddBackupServer(e: React.FormEvent) {
    e.preventDefault();
    if (!backupServerName.trim() || !backupServerAddress.trim() || !backupServerApiKey.trim()) {
      setError("All backup server fields are required.");
      return;
    }
    const freq = parseInt(backupServerFrequency, 10);
    if (isNaN(freq) || freq < 1) {
      setError("Frequency must be a positive number of hours.");
      return;
    }
    setAddingBackupServer(true);
    setError("");
    try {
      await api.backup.addServer({
        name: backupServerName.trim(),
        address: backupServerAddress.trim(),
        api_key: backupServerApiKey.trim(),
        sync_frequency_hours: freq,
      });
      setSuccess("Backup server added successfully.");
      setShowAddBackupServer(false);
      setBackupServerName("");
      setBackupServerAddress("");
      setBackupServerApiKey("");
      setBackupServerFrequency("24");
      await loadBackupServers();
    } catch (err: any) {
      setError(err.message || "Failed to add backup server.");
    } finally {
      setAddingBackupServer(false);
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="max-w-2xl mx-auto p-4">

      {error && (
        <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">{error}</p>
      )}
      {success && (
        <p className="text-green-600 dark:text-green-400 text-sm mb-4 p-3 bg-green-50 dark:bg-green-900/30 rounded">
          {success}
        </p>
      )}

      <AccountSection username={username ?? ""} error={error} setError={setError} success={success} setSuccess={setSuccess} loading={loading} setLoading={setLoading} />

      {/* ── Storage Usage ──────────────────────────────────────────────────── */}
      <StorageStatsSection stats={storageStats} loading={storageLoading} />

      {/* ── Server Selection ───────────────────────────────────────────────── */}
      {backupLoaded && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Active Server</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
            Choose which server to view photos from.
          </p>
          <select
            value={viewMode === "main" ? "__main__" : (activeBackupServerId ?? "__main__")}
            onChange={(e) => {
              const val = e.target.value;
              if (val === "__main__") {
                setViewMode("main");
              } else {
                setActiveBackupServerId(val);
                setViewMode("backup");
              }
            }}
            className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
          >
            <option value="__main__">Main Server (local)</option>
            {backupServers.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name} — {s.address}
              </option>
            ))}
          </select>
          {backupServers.length === 0 && (
            <p className="text-xs text-gray-400 mt-2">
              No backup servers configured. Add one in the Backup Recovery section below.
            </p>
          )}
        </section>
      )}

      {/* ── Scan for New Files (admin, plain mode) ───────────────────────── */}
      {isAdmin && encryptionMode === "plain" && !encryptionLoading && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-2">Scan for New Files</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
            Scan the storage directory for new photos and videos that haven't been registered yet.
          </p>
          <div className="flex items-center gap-3">
            <button
              onClick={async () => {
                setScanning(true);
                setScanResult(null);
                setError("");
                try {
                  const res = await api.admin.scanAndRegister();
                  setScanResult(
                    res.registered > 0
                      ? `Found and registered ${res.registered} new file${res.registered > 1 ? "s" : ""}.`
                      : "No new files found."
                  );
                } catch (err: any) {
                  setError(err.message || "Scan failed");
                } finally {
                  setScanning(false);
                }
              }}
              disabled={scanning}
              className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors disabled:opacity-50"
            >
              <AppIcon name="reload" size="w-4 h-4" className={scanning ? "animate-spin" : ""} />
              {scanning ? "Scanning…" : "Scan Now"}
            </button>
            {scanResult && (
              <span className="text-sm text-gray-600 dark:text-gray-400">{scanResult}</span>
            )}
          </div>
        </section>
      )}

      {/* ── Privacy & Encryption ─────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Privacy & Encryption</h2>

        {encryptionLoading ? (
          <div className="text-gray-400 text-sm">Loading encryption settings…</div>
        ) : (
          <div className="space-y-4">
            {/* Toggle switch */}
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
                  End-to-End Encryption
                </h3>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  {encryptionMode === "encrypted"
                    ? "Photos are encrypted — only you can view them."
                    : "Photos are stored as regular files on disk."}
                </p>
              </div>
              <button
                onClick={() => {
                  if (migrationStatus !== "idle") return;
                  setShowEncryptionWarning(true);
                }}
                disabled={togglingEncryption || migrationStatus !== "idle"}
                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:opacity-50 ${
                  encryptionMode === "encrypted" ? "bg-blue-600" : "bg-gray-300 dark:bg-gray-600"
                }`}
                role="switch"
                aria-checked={encryptionMode === "encrypted"}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                    encryptionMode === "encrypted" ? "translate-x-6" : "translate-x-1"
                  }`}
                />
              </button>
            </div>

            {/* Migration progress */}
            {(migrationStatus === "encrypting" || migrationStatus === "decrypting") && (() => {
              const pct = migrationTotal > 0 ? Math.min(Math.round((migrationCompleted / migrationTotal) * 100), 100) : 0;
              const action = migrationStatus === "encrypting" ? "Encrypting" : "Decrypting";
              return (
              <div className="bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  <div
                    className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin cursor-help"
                    title={`${action} photos — ${pct}% complete (${migrationCompleted}/${migrationTotal})`}
                  />
                  <span
                    className="text-sm font-medium text-blue-700 dark:text-blue-300 cursor-help"
                    title={`${action} photos — ${pct}% complete (${migrationCompleted}/${migrationTotal})`}
                  >
                    {action} photos… {pct}%
                  </span>
                </div>
                <div className="w-full h-2 bg-blue-200 dark:bg-blue-800 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-600 rounded-full transition-all duration-500"
                    style={{ width: `${pct}%` }}
                  />
                </div>
                <p className="text-xs text-blue-600 dark:text-blue-400 mt-1">
                  {migrationCompleted} / {migrationTotal} items processed ({pct}%)
                </p>
              </div>
              );
            })()}

            {/* Migration error */}
            {migrationError && (
              <div className="bg-red-50 dark:bg-red-900/30 rounded-lg p-3">
                <p className="text-sm text-red-600 dark:text-red-400">
                  Migration error: {migrationError}
                </p>
              </div>
            )}

            {/* Toggle confirmation warning */}
            {showEncryptionWarning && (
              <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-4">
                <h4 className="text-sm font-semibold text-amber-800 dark:text-amber-300 mb-2">
                  ⚠️ {encryptionMode === "plain" ? "Enable Encryption?" : "Disable Encryption?"}
                </h4>
                <p className="text-sm text-amber-700 dark:text-amber-400 mb-3">
                  This process can take a significant amount of time depending on your library size.
                  It will run in the background — you can continue using the app while it processes.
                </p>
                <div className="flex gap-2">
                  <button
                    onClick={handleToggleEncryption}
                    disabled={togglingEncryption}
                    className={`px-4 py-2 rounded-md text-sm text-white disabled:opacity-50 ${
                      encryptionMode === "plain"
                        ? "bg-amber-600 hover:bg-amber-700"
                        : "bg-blue-600 hover:bg-blue-700"
                    }`}
                  >
                    {togglingEncryption ? "Switching…" : "Confirm"}
                  </button>
                  <button
                    onClick={() => setShowEncryptionWarning(false)}
                    className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </section>

      {/* ── Backup Recovery ─────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Backup Recovery</h2>
        <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
          Recover photos from a configured backup server. Any photos on the backup
          that don't already exist on this server (by filename) will be downloaded and imported.
        </p>

        {!backupLoaded ? (
          <div className="text-gray-400 text-sm">Loading backup servers…</div>
        ) : backupServers.length === 0 ? (
          <div className="text-center py-4 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
            <p className="text-gray-400 text-sm">No backup servers configured.</p>
            <p className="text-xs text-gray-400 mt-1 mb-3">
              Auto-detection didn't find any servers. You can add one manually below.
            </p>
          </div>
        ) : !showRecoverWarning ? (
          <button
            onClick={() => {
              setShowRecoverWarning(true);
              setError("");
              setSuccess("");
            }}
            disabled={recovering}
            className="bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 disabled:opacity-50 text-sm"
          >
            {recovering ? (
              <span className="flex items-center gap-2">
                <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                Recovering…
              </span>
            ) : (
              "Recover from Backup Server"
            )}
          </button>
        ) : (
          <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-4">
            <h4 className="text-sm font-semibold text-amber-800 dark:text-amber-300 mb-2">
              ⚠️ Confirm Recovery
            </h4>
            <p className="text-sm text-amber-700 dark:text-amber-400 mb-1">
              This will download <strong>all photos</strong> from the backup server
              {" "}<strong>"{backupServers.find((s) => s.enabled)?.name ?? backupServers[0]?.name}"</strong> to
              this server.
            </p>
            <ul className="text-sm text-amber-700 dark:text-amber-400 list-disc list-inside mb-3 space-y-0.5">
              <li>Photos with the same filename will be <strong>skipped</strong> (not overwritten).</li>
              <li>This process runs in the background and may take a while for large libraries.</li>
              <li>The backup server must be reachable and have its API key configured.</li>
            </ul>
            <div className="flex gap-2">
              <button
                onClick={handleRecover}
                disabled={recovering}
                className="bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 disabled:opacity-50 text-sm"
              >
                {recovering ? "Starting…" : "Confirm Recovery"}
              </button>
              <button
                onClick={() => setShowRecoverWarning(false)}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* ── Manually add a backup server ───────────────────────── */}
        <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
          {!showAddBackupServer ? (
            <button
              onClick={() => setShowAddBackupServer(true)}
              className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
            >
              + Add backup server manually
            </button>
          ) : (
            <form onSubmit={handleAddBackupServer} className="space-y-3">
              <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-300">Add Backup Server</h4>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Name</label>
                <input
                  type="text"
                  value={backupServerName}
                  onChange={(e) => setBackupServerName(e.target.value)}
                  placeholder="My Backup Server"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Server Address</label>
                <input
                  type="text"
                  value={backupServerAddress}
                  onChange={(e) => setBackupServerAddress(e.target.value)}
                  placeholder="https://backup.example.com:8443"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">API Key</label>
                <input
                  type="password"
                  value={backupServerApiKey}
                  onChange={(e) => setBackupServerApiKey(e.target.value)}
                  placeholder="Backup server API key"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Backup Frequency (hours)</label>
                <input
                  type="number"
                  min={1}
                  value={backupServerFrequency}
                  onChange={(e) => setBackupServerFrequency(e.target.value)}
                  className="w-28 border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
                />
              </div>
              <div className="flex gap-2">
                <button
                  type="submit"
                  disabled={addingBackupServer}
                  className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
                >
                  {addingBackupServer ? (
                    <span className="flex items-center gap-2">
                      <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                      Adding…
                    </span>
                  ) : (
                    "Add Server"
                  )}
                </button>
                <button
                  type="button"
                  onClick={() => setShowAddBackupServer(false)}
                  className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
                >
                  Cancel
                </button>
              </div>
            </form>
          )}
        </div>
      </section>

      {/* ── Apps ───────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Apps</h2>
        <div className="space-y-4">
          <div>
            <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Android App</h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-2">
              Download the Simple Photos Android app to automatically back up photos from your phone.
            </p>
            <button
              onClick={async () => {
                try {
                  const res = await fetch("/api/downloads/android", { method: "HEAD" });
                  if (res.ok) {
                    window.location.href = "/api/downloads/android";
                  } else {
                    setError("Android APK is not available yet. Build it with: cd android && ./gradlew assembleRelease — or place a pre-built APK at downloads/simple-photos.apk");
                  }
                } catch {
                  setError("Could not check APK availability.");
                }
              }}
              className="inline-flex items-center gap-1.5 bg-green-600 text-white px-4 py-2 rounded-md hover:bg-green-700 text-sm"
            >
              📱 Download Android App (.apk)
            </button>
          </div>
        </div>
      </section>



      <SslSettings error={error} setError={setError} success={success} setSuccess={setSuccess} />

      {/* ── Manage Users (admin only) ────────────────────────────────────── */}
      <UserManagement error={error} setError={setError} success={success} setSuccess={setSuccess} />



      {/* ── About ───────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">About</h2>
        <div className="flex flex-col items-center text-center">
          <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mb-3" />
          <h3 className="text-xl font-bold text-gray-900 dark:text-gray-100">Simple Photos</h3>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
            v0.6.9 — Self-hosted, end-to-end encrypted photo & video library
          </p>
          <hr className="w-full border-gray-100 dark:border-gray-700 mb-4" />
          <p className="text-xs text-gray-400 mb-2">Developed by</p>
          <img
            src="/wulfnet.jpg"
            alt="WulfNet Designs"
            className="h-16 mb-1"
          />
          <p className="text-sm font-semibold text-gray-700 dark:text-gray-300">WulfNet Designs</p>
          <p className="text-xs text-gray-400 mt-3">
            &copy; {new Date().getFullYear()} WulfNet Designs. All rights
            reserved.
          </p>
        </div>
      </section>

      {/* ── Credits & Links ─────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">Credits &amp; Links</h2>
        <div className="space-y-3 text-sm">
          <div className="flex items-center gap-3">
            <AppIcon name="star" size="w-5 h-5" />
            <div>
              <p className="text-gray-900 dark:text-gray-100 font-medium">Icons</p>
              <p className="text-gray-500 dark:text-gray-400">
                Custom icons by{" "}
                <a
                  href="https://www.flaticon.com/authors/angus-87"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  Angus_87
                </a>{" "}
                on{" "}
                <a
                  href="https://www.flaticon.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  Flaticon
                </a>
              </p>
            </div>
          </div>
          <hr className="border-gray-100 dark:border-gray-700" />
          <div className="flex items-center gap-3">
            <AppIcon name="shared" size="w-5 h-5" />
            <div>
              <p className="text-gray-900 dark:text-gray-100 font-medium">Source Code</p>
              <p className="text-gray-500 dark:text-gray-400">
                <a
                  href="https://github.com/wulfic/simple-photos"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline"
                >
                  github.com/wulfic/simple-photos
                </a>
              </p>
            </div>
          </div>
        </div>
      </section>
      </main>
    </div>
  );
}
