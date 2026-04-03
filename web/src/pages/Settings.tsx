/**
 * Settings page — admin and user configuration panel.
 *
 * Sections: encryption key management, backup server management,
 * auto-scan, SSL settings, account (password/2FA),
 * user management (admin), and thumbnail size preference.
 */
import { useState, useEffect, useCallback } from "react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { useIsAdmin } from "../hooks/useIsAdmin";
import StorageStatsSection from "../components/StorageStatsSection";
import UserManagement from "../components/settings/UserManagement";
import SslSettings from "../components/settings/SslSettings";
import AccountSection from "../components/settings/AccountSection";
import BackupRecoverySection from "../components/settings/BackupRecoverySection";
// Migration is now fully server-side — no browser-based worker needed
import { getErrorMessage } from "../utils/formatters";
import { useThumbnailSizeStore } from "../store/thumbnailSize";

export default function Settings() {
  const { username } = useAuthStore();
  const isAdmin = useIsAdmin();
  const { thumbnailSize, toggle: toggleThumbnailSize } = useThumbnailSizeStore();

  // ── General state ────────────────────────────────────────────────────────
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");
  const [loading, setLoading] = useState(false);

  // ── Backup mode state (is this a backup server?) ────────────────────────
  const [isBackupMode, setIsBackupMode] = useState(false);
  const [primaryServerUrl, setPrimaryServerUrl] = useState<string | null>(null);

  // ── Backup store (for Active Server dropdown) ────────────────────────────
  const { backupServers, loaded: backupLoaded, setBackupServers, setLoaded: setBackupLoaded, viewMode, setViewMode, activeBackupServerId, setActiveBackupServerId } = useBackupStore();

  // ── Audio backup setting ────────────────────────────────────────────────
  const [audioBackupEnabled, setAudioBackupEnabled] = useState(false);
  const [audioBackupLoading, setAudioBackupLoading] = useState(true);
  const [togglingAudioBackup, setTogglingAudioBackup] = useState(false);

  // ── Storage stats state ─────────────────────────────────────────────────
  type StorageStats = {
    photo_bytes: number; photo_count: number;
    video_bytes: number; video_count: number;
    other_blob_bytes: number; other_blob_count: number;
    user_total_bytes: number;
    fs_total_bytes: number; fs_free_bytes: number;
  };
  const [storageStats, setStorageStats] = useState<StorageStats | null>(null);
  const [storageLoading, setStorageLoading] = useState(true);

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

  // Fetch backup servers and mode on mount
  useEffect(() => {
    loadBackupServers();
    loadStorageStats();
    loadAudioBackupSetting();
    loadBackupMode();
  }, [loadBackupServers]);

  async function loadBackupMode() {
    try {
      const mode = await api.backup.getMode();
      setIsBackupMode(mode.mode === "backup");
      setPrimaryServerUrl(mode.primary_server_url ?? null);
    } catch {
      // Not an admin or endpoint unavailable — default to primary behaviour
    }
  }

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

  async function loadAudioBackupSetting() {
    setAudioBackupLoading(true);
    try {
      const res = await api.backup.getAudioBackupSetting();
      setAudioBackupEnabled(res.audio_backup_enabled);
    } catch {
      // Setting may not exist yet — default to false
    } finally {
      setAudioBackupLoading(false);
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

      {!isBackupMode ? (
        <AccountSection username={username ?? ""} error={error} setError={setError} success={success} setSuccess={setSuccess} loading={loading} setLoading={setLoading} />
      ) : (
        /* Backup servers mirror accounts from the primary — no local changes */
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-2">Account</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400">
            Logged in as <strong>{username}</strong>. Account changes (password, 2FA) are managed on the primary server.
          </p>
        </section>
      )}

      {/* ── Storage Usage ──────────────────────────────────────────────────── */}
      <StorageStatsSection stats={storageStats} loading={storageLoading} />

      {/* ── Server Selection (hidden on backup servers) ─────────────────── */}
      {backupLoaded && !isBackupMode && (
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

      {/* ── Display ──────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Display</h2>
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
              Thumbnail Size
            </h3>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              {thumbnailSize === "large"
                ? "Large — 2 photos per row for bigger previews."
                : "Normal — 3 photos per row (default)."}
            </p>
          </div>
          <div className="flex items-center gap-2 flex-shrink-0">
            <span className={`text-xs font-medium ${
              thumbnailSize === "normal"
                ? "text-blue-600 dark:text-blue-400"
                : "text-gray-400 dark:text-gray-500"
            }`}>Normal</span>
            <button
              onClick={toggleThumbnailSize}
              className={`relative inline-flex h-6 w-11 flex-shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                thumbnailSize === "large"
                  ? "bg-blue-600"
                  : "bg-gray-300 dark:bg-gray-600"
              }`}
              role="switch"
              aria-checked={thumbnailSize === "large"}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  thumbnailSize === "large"
                    ? "translate-x-6"
                    : "translate-x-1"
                }`}
              />
            </button>
            <span className={`text-xs font-medium ${
              thumbnailSize === "large"
                ? "text-blue-600 dark:text-blue-400"
                : "text-gray-400 dark:text-gray-500"
            }`}>Large</span>
          </div>
        </div>
      </section>

      {/* ── Backup Recovery / Primary Server Connection ─────────────────── */}
      {isAdmin && (
        <BackupRecoverySection
          isBackupMode={isBackupMode}
          primaryServerUrl={primaryServerUrl}
          setError={setError}
          setSuccess={setSuccess}
          loadBackupServers={loadBackupServers}
        />
      )}

      {/* ── Apps (hidden on backup servers — Android clients connect to primary) ── */}
      {!isBackupMode && (
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
                    // Programmatic download via temporary anchor — avoids SPA
                    // navigation issues that window.location.href can cause.
                    const a = document.createElement("a");
                    a.href = "/api/downloads/android";
                    a.download = "simple-photos.apk";
                    document.body.appendChild(a);
                    a.click();
                    a.remove();
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
      )}



      {/* ── Audio Backup (hidden on backup servers) ─────────────────────── */}
      {isAdmin && !audioBackupLoading && !isBackupMode && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Audio Backup</h2>
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0">
              <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
                Include Audio in Backups
              </h3>
              <p className="text-sm text-gray-500 dark:text-gray-400">
                {audioBackupEnabled
                  ? "Audio files (MP3, FLAC, WAV, etc.) are included when syncing to backup servers."
                  : "Audio files are excluded from backup sync. Only photos and videos will be backed up."}
              </p>
            </div>
            <button
              onClick={async () => {
                setTogglingAudioBackup(true);
                setError("");
                try {
                  const newVal = !audioBackupEnabled;
                  const res = await api.backup.setAudioBackupSetting(newVal);
                  setAudioBackupEnabled(res.audio_backup_enabled);
                  setSuccess(res.message);
                } catch (err: unknown) {
                  setError(getErrorMessage(err, "Failed to update audio backup setting."));
                } finally {
                  setTogglingAudioBackup(false);
                }
              }}
              disabled={togglingAudioBackup}
              className={`relative inline-flex h-6 w-11 flex-shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:opacity-50 ${
                audioBackupEnabled ? "bg-blue-600" : "bg-gray-300 dark:bg-gray-600"
              }`}
              role="switch"
              aria-checked={audioBackupEnabled}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  audioBackupEnabled ? "translate-x-6" : "translate-x-1"
                }`}
              />
            </button>
          </div>
        </section>
      )}

      <SslSettings error={error} setError={setError} success={success} setSuccess={setSuccess} />

      {/* ── Manage Users (admin only, hidden on backup servers) ────────── */}
      {!isBackupMode && (
        <UserManagement error={error} setError={setError} success={success} setSuccess={setSuccess} />
      )}



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
