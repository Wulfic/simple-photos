/**
 * Settings page — admin and user configuration panel.
 *
 * Sections: encryption key management, backup server management,
 * auto-scan, SSL settings, account (password/2FA),
 * user management (admin), and thumbnail size preference.
 */
import { useState, useEffect, useCallback } from "react";
import { useAppNavigate } from "../hooks/useAppNavigate";
import { api } from "../api/client";
import type { ExportJob, ExportFile } from "../api/export";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { Toggle, Select } from "../components/ui";
import { useIsAdmin } from "../hooks/useIsAdmin";
import StorageStatsSection from "../components/StorageStatsSection";
import UserManagement from "../components/settings/UserManagement";
import SslSettings from "../components/settings/SslSettings";
import AccountSection from "../components/settings/AccountSection";
import BackupRecoverySection from "../components/settings/BackupRecoverySection";
import AiRecognitionSection from "../components/settings/AiRecognitionSection";
import GeolocationSection from "../components/settings/GeolocationSection";
// Migration is now fully server-side — no browser-based worker needed
import CastDialog, { CastIcon } from "../components/CastDialog";
import { getErrorMessage } from "../utils/formatters";
import { useThumbnailSizeStore } from "../store/thumbnailSize";
import { usePwaInstall } from "../hooks/usePwaInstall";
import PwaInstallInstructionsDialog from "../components/PwaInstallInstructionsDialog";

export default function Settings() {
  const { username } = useAuthStore();
  const isAdmin = useIsAdmin();
  const { canInstall, isInstalled, promptInstall } = usePwaInstall();
  const { thumbnailSize, toggle: toggleThumbnailSize } = useThumbnailSizeStore();
  const navigate = useAppNavigate();

  // ── General state ────────────────────────────────────────────────────────
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");
  const [loading, setLoading] = useState(false);

  // ── Export state ─────────────────────────────────────────────────────────
  const [exportSizeLimit, setExportSizeLimit] = useState<number>(10_737_418_240); // 10 GB
  const [exportJob, setExportJob] = useState<ExportJob | null>(null);
  const [exportFiles, setExportFiles] = useState<ExportFile[]>([]);
  const [exportLoading, setExportLoading] = useState(false);
  const [exportStatusLoading, setExportStatusLoading] = useState(true);

  // ── Backup mode state (is this a backup server?) ────────────────────────
  const [isBackupMode, setIsBackupMode] = useState(false);
  const [primaryServerUrl, setPrimaryServerUrl] = useState<string | null>(null);

  // ── Backup store (for Active Server dropdown) ────────────────────────────
  const { backupServers, loaded: backupLoaded, setBackupServers, setLoaded: setBackupLoaded, viewMode, setViewMode, activeBackupServerId, setActiveBackupServerId } = useBackupStore();

  // ── Audio backup setting ────────────────────────────────────────────────
  const [audioBackupEnabled, setAudioBackupEnabled] = useState(false);
  const [audioBackupLoading, setAudioBackupLoading] = useState(true);
  const [togglingAudioBackup, setTogglingAudioBackup] = useState(false);

  // ── Restart server state ──────────────────────────────────────────────
  const [restartLoading, setRestartLoading] = useState(false);
  const [restartConfirm, setRestartConfirm] = useState(false);

  // ── Cast dialog state ─────────────────────────────────────────────────
  const [castDialogOpen, setCastDialogOpen] = useState(false);

  // ── PWA install-instructions dialog (shown when the browser hasn't
  //    fired beforeinstallprompt — e.g. Brave, Firefox, Safari). ────────
  const [installHelpOpen, setInstallHelpOpen] = useState(false);

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
  const [showInstallInstructions, setShowInstallInstructions] = useState(false);

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
    loadExportStatus();
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

  async function handleRestartServer() {
    if (!restartConfirm) {
      setRestartConfirm(true);
      return;
    }
    setRestartLoading(true);
    setError("");
    setRestartConfirm(false);
    try {
      await api.admin.restart();
      setSuccess("Server restarting… page will reload when it comes back up.");
      // Poll /health until the server responds again, then reload.
      const poll = setInterval(async () => {
        try {
          const r = await fetch("/health", { cache: "no-store" });
          if (r.ok) {
            clearInterval(poll);
            window.location.reload();
          }
        } catch {
          // Server still down — keep polling
        }
      }, 2000);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to restart server."));
    } finally {
      setRestartLoading(false);
    }
  }

  async function loadExportStatus() {
    setExportStatusLoading(true);
    try {
      const res = await api.export.status();
      setExportJob(res.job);
      setExportFiles(res.files);
    } catch {
      // No export job yet — that's fine
      setExportJob(null);
      setExportFiles([]);
    } finally {
      setExportStatusLoading(false);
    }
  }

  async function startExport() {
    setExportLoading(true);
    setError("");
    try {
      const job = await api.export.start(exportSizeLimit);
      setExportJob(job);
      setSuccess("Export started! This may take a while for large libraries.");
      // Poll for status updates
      pollExportStatus();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to start export."));
    } finally {
      setExportLoading(false);
    }
  }

  function pollExportStatus() {
    const interval = setInterval(async () => {
      try {
        const res = await api.export.status();
        setExportJob(res.job);
        setExportFiles(res.files);
        if (res.job.status === "completed" || res.job.status === "failed") {
          clearInterval(interval);
          if (res.job.status === "completed") {
            setSuccess("Export completed! Your download files are ready.");
          } else if (res.job.error) {
            setError(`Export failed: ${res.job.error}`);
          }
        }
      } catch {
        clearInterval(interval);
      }
    }, 3000);
  }

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />

      <main className="max-w-5xl 2xl:max-w-[1600px] mx-auto p-4">

      {error && (
        <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">{error}</p>
      )}
      {success && (
        <p className="text-green-600 dark:text-green-400 text-sm mb-4 p-3 bg-green-50 dark:bg-green-900/30 rounded">
          {success}
        </p>
      )}

      {/* Masonry card layout: single column on narrow screens (the original
          look), two columns when there's room, and three on very wide screens
          to make use of the space. break-inside-avoid keeps each card whole;
          the cards' own mb-4 provides vertical rhythm. */}
      <div className="lg:columns-2 2xl:columns-3 gap-4 [&>*]:break-inside-avoid">

      {!isBackupMode ? (
        <AccountSection username={username ?? ""} error={error} setError={setError} success={success} setSuccess={setSuccess} loading={loading} setLoading={setLoading} />
      ) : (
        /* Backup servers mirror accounts from the primary — no local changes */
        <section className="card p-6 mb-4">
          <h2 className="text-lg font-semibold mb-2">Account</h2>
          <p className="text-sm text-fg-muted">
            Logged in as <strong>{username}</strong>. Account changes (password, 2FA) are managed on the primary server.
          </p>
        </section>
      )}

      {/* ── Storage Usage ──────────────────────────────────────────────────── */}
      <StorageStatsSection stats={storageStats} loading={storageLoading} />

      {/* ── Library Export ─────────────────────────────────────────────────── */}
      {!isBackupMode && (
        <section className="card p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Library Export</h2>
          <p className="text-sm text-fg-muted mb-4">
            Package your entire media library with metadata into downloadable zip files.
            Files are available for 24 hours after export.
          </p>

          {/* Export controls */}
          <div className="flex flex-wrap items-center gap-3 mb-4">
            <Select
              value={exportSizeLimit}
              onChange={(e) => setExportSizeLimit(Number(e.target.value))}
              disabled={exportLoading || (exportJob?.status === "pending" || exportJob?.status === "running")}
            >
              <option value={10_737_418_240}>10 GB per file</option>
              <option value={21_474_836_480}>20 GB per file</option>
              <option value={53_687_091_200}>50 GB per file</option>
            </Select>

            <button
              onClick={() => navigate("/export-downloads")}
              disabled={exportFiles.length === 0}
              className="btn btn-success btn-md gap-1.5"
            >
              Downloads{exportFiles.length > 0 ? ` (${exportFiles.length})` : ""}
            </button>

            <button
              onClick={startExport}
              disabled={exportLoading || exportJob?.status === "pending" || exportJob?.status === "running"}
              className="btn btn-primary btn-md inline-flex items-center"
            >
              {exportJob?.status === "pending" || exportJob?.status === "running"
                ? "Exporting…"
                : "Export Library"}
            </button>
          </div>

          {/* Status indicator */}
          {!exportStatusLoading && exportJob && (
            <div className="text-sm">
              {exportJob.status === "pending" && (
                <p className="text-yellow-600 dark:text-yellow-400 flex items-center gap-2">
                  <span className="w-3 h-3 border-2 border-yellow-500 border-t-transparent rounded-full animate-spin inline-block" />
                  Export queued…
                </p>
              )}
              {exportJob.status === "running" && (
                <p className="text-accent-600 dark:text-accent-400 flex items-center gap-2">
                  <span className="w-3 h-3 border-2 border-accent-500 border-t-transparent rounded-full animate-spin inline-block" />
                  Packaging your library…
                </p>
              )}
              {exportJob.status === "completed" && (
                <p className="text-green-600 dark:text-green-400">
                  Export completed — {exportFiles.length} file{exportFiles.length !== 1 ? "s" : ""} ready for download.
                </p>
              )}
              {exportJob.status === "failed" && (
                <p className="text-red-600 dark:text-red-400">
                  Export failed{exportJob.error ? `: ${exportJob.error}` : "."}
                </p>
              )}
            </div>
          )}
        </section>
      )}

      {/* ── Server Selection (hidden on backup servers) ─────────────────── */}
      {backupLoaded && !isBackupMode && (
        <section className="card p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Active Server</h2>
          <p className="text-sm text-fg-muted mb-3">
            Choose which server to view photos from.
          </p>
          <Select
            fullWidth
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
          >
            <option value="__main__">Main Server (local)</option>
            {backupServers.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name} — {s.address}
              </option>
            ))}
          </Select>
          {backupServers.length === 0 && (
            <p className="text-xs text-fg-muted mt-2">
              No backup servers configured. Add one in the Backup Recovery section below.
            </p>
          )}
        </section>
      )}

      {/* ── Display ──────────────────────────────────────────────────── */}
      <section className="card p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Display</h2>
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <h3 className="text-sm font-medium text-fg-muted">
              Thumbnail Size
            </h3>
            <p className="text-sm text-fg-muted">
              {thumbnailSize === "large"
                ? "Large — taller rows for bigger photo previews."
                : "Normal — compact rows showing more photos (default)."}
            </p>
          </div>
          <div className="flex items-center gap-2 flex-shrink-0">
            <span className={`text-xs font-medium ${
              thumbnailSize === "normal"
                ? "text-accent-600 dark:text-accent-400"
                : "text-fg-muted"
            }`}>Normal</span>
            <Toggle
              label="Use large thumbnails"
              checked={thumbnailSize === "large"}
              onClick={toggleThumbnailSize}
            />
            <span className={`text-xs font-medium ${
              thumbnailSize === "large"
                ? "text-accent-600 dark:text-accent-400"
                : "text-fg-muted"
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

      {/* ── AI Recognition (primary only — backups receive AI data via sync) ── */}
      {!isBackupMode && (
        <AiRecognitionSection
          error={error}
          setError={setError}
          success={success}
          setSuccess={setSuccess}
        />
      )}

      {/* ── Geolocation (primary only — backups receive geo data via sync) ── */}
      {!isBackupMode && (
        <GeolocationSection
          error={error}
          setError={setError}
          success={success}
          setSuccess={setSuccess}
        />
      )}

      {/* ── Apps (hidden on backup servers — Android clients connect to primary) ── */}
      {!isBackupMode && (
      <section className="card p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Apps</h2>
        <div className="space-y-4">
          <div>
            <h3 className="text-sm font-medium text-fg-muted mb-1">Install Simple Photos</h3>
            <p className="text-sm text-fg-muted mb-2">
              Use this site like a native app. Choose <strong>Install App</strong> for a quick web-app install on any device, or <strong>Android App</strong> for automatic phone-photo backup.
            </p>
            <div className="flex flex-wrap gap-2">
              <button
                onClick={async () => {
                  if (isInstalled) {
                    setSuccess("Simple Photos is already installed on this device.");
                    return;
                  }
                  // When the browser hasn't fired `beforeinstallprompt`
                  // (Brave with shields, Firefox, Safari, or Chromium before
                  // it has decided the site is "installable"), we cannot
                  // trigger a native install programmatically. Instead, open
                  // a dialog with browser-specific manual install steps so
                  // the button is always actionable.
                  if (!canInstall) {
                    setInstallHelpOpen(true);
                    return;
                  }
                  const outcome = await promptInstall();
                  if (outcome === "accepted") setSuccess("App installed.");
                  else if (outcome === "dismissed") setError("Install dismissed.");
                  else if (outcome === "unavailable") setInstallHelpOpen(true);
                }}
                className="btn btn-primary btn-md inline-flex items-center"
                disabled={isInstalled}
                title={
                  isInstalled
                    ? "Already installed on this device"
                    : canInstall
                      ? "Install Simple Photos as a web app on this device"
                      : "Show install instructions for this browser"
                }
              >
                ⬇️ {isInstalled ? "App Installed" : "Install App (Web)"}
              </button>
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
                className="btn btn-success btn-md gap-1.5"
                title="Download the native Android APK for automatic phone-photo backup"
              >
                📱 Android App (.apk)
              </button>
            </div>

            {/* Collapsible install instructions */}
            <button
              onClick={() => setShowInstallInstructions((v) => !v)}
              className="mt-3 flex items-center gap-1.5 text-sm text-accent-600 dark:text-accent-400 hover:text-accent-800 dark:hover:text-accent-300 transition-colors"
            >
              <span className={`inline-block transition-transform ${showInstallInstructions ? "rotate-90" : ""}`}>▶</span>
              Android Install Instructions
            </button>
            {showInstallInstructions && (
              <div className="mt-3 bg-canvas rounded-lg p-4">
                <h3 className="font-medium text-fg text-sm mb-3">
                  How to install (sideload):
                </h3>
                <ol className="text-sm text-fg-muted space-y-3">
                  <li className="flex gap-3">
                    <span className="flex-shrink-0 w-6 h-6 bg-accent-100 dark:bg-accent-900/40 text-accent-700 dark:text-accent-300 rounded-full flex items-center justify-center text-xs font-bold">1</span>
                    <div>
                      <p className="font-medium text-fg-muted">Download the APK</p>
                      <p className="text-xs text-fg-muted">Click the button above or transfer the APK to your phone via USB/email.</p>
                    </div>
                  </li>
                  <li className="flex gap-3">
                    <span className="flex-shrink-0 w-6 h-6 bg-accent-100 dark:bg-accent-900/40 text-accent-700 dark:text-accent-300 rounded-full flex items-center justify-center text-xs font-bold">2</span>
                    <div>
                      <p className="font-medium text-fg-muted">Enable "Install unknown apps"</p>
                      <p className="text-xs text-fg-muted">Go to <strong>Settings → Apps → Special access → Install unknown apps</strong> and enable it for your file manager or browser.</p>
                    </div>
                  </li>
                  <li className="flex gap-3">
                    <span className="flex-shrink-0 w-6 h-6 bg-accent-100 dark:bg-accent-900/40 text-accent-700 dark:text-accent-300 rounded-full flex items-center justify-center text-xs font-bold">3</span>
                    <div>
                      <p className="font-medium text-fg-muted">Open the APK</p>
                      <p className="text-xs text-fg-muted">Tap the downloaded APK file and confirm the installation prompt.</p>
                    </div>
                  </li>
                  <li className="flex gap-3">
                    <span className="flex-shrink-0 w-6 h-6 bg-accent-100 dark:bg-accent-900/40 text-accent-700 dark:text-accent-300 rounded-full flex items-center justify-center text-xs font-bold">4</span>
                    <div>
                      <p className="font-medium text-fg-muted">Connect to your server</p>
                      <p className="text-xs text-fg-muted">Open the app, enter your server URL:</p>
                      <code className="block mt-1 bg-edge-strong px-2 py-1 rounded text-xs text-fg break-all">{window.location.origin}</code>
                    </div>
                  </li>
                  <li className="flex gap-3">
                    <span className="flex-shrink-0 w-6 h-6 bg-accent-100 dark:bg-accent-900/40 text-accent-700 dark:text-accent-300 rounded-full flex items-center justify-center text-xs font-bold">5</span>
                    <div>
                      <p className="font-medium text-fg-muted">Sign in & grant permissions</p>
                      <p className="text-xs text-fg-muted">Log in with your account and allow the app to access your photos and videos for automatic encrypted backup.</p>
                    </div>
                  </li>
                </ol>
                <div className="mt-3 bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-800 dark:text-amber-300">
                  <strong>Note:</strong> Keep "Install unknown apps" disabled after installation for security. You can always re-enable it when updating the app.
                </div>
              </div>
            )}
          </div>
        </div>
      </section>
      )}



      {/* ── Audio Backup (hidden on backup servers) ─────────────────────── */}
      {isAdmin && !audioBackupLoading && !isBackupMode && (
        <section className="card p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Audio Backup</h2>
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0">
              <h3 className="text-sm font-medium text-fg-muted">
                Include Audio in Backups
              </h3>
              <p className="text-sm text-fg-muted">
                {audioBackupEnabled
                  ? "Audio files (MP3, FLAC, WAV, etc.) are included when syncing to backup servers."
                  : "Audio files are excluded from backup sync. Only photos and videos will be backed up."}
              </p>
            </div>
            <Toggle
              label="Include Audio in Backups"
              checked={audioBackupEnabled}
              disabled={togglingAudioBackup}
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
            />
          </div>
        </section>
      )}

      <SslSettings error={error} setError={setError} success={success} setSuccess={setSuccess} />

      {/* ── Manage Users (admin only, hidden on backup servers) ────────── */}
      {!isBackupMode && (
        <UserManagement error={error} setError={setError} success={success} setSuccess={setSuccess} />
      )}



      {/* ── Cast (HTTPS only) ────────────────────────────────────────────── */}
      {window.location.protocol === "https:" && (
        <section className="card p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Cast</h2>
          <p className="text-sm text-fg-muted mb-4">
            Stream your gallery slideshow to a Chromecast or compatible receiver on your local network.
          </p>
          <button
            onClick={() => setCastDialogOpen(true)}
            className="btn btn-primary btn-md inline-flex items-center"
          >
            <CastIcon className="w-4 h-4" />
            Cast to device…
          </button>
          <CastDialog open={castDialogOpen} onClose={() => setCastDialogOpen(false)} />
        </section>
      )}

      {/* PWA install-instructions fallback dialog — rendered globally so it
          works even if the Apps section is collapsed/hidden on small screens. */}
      <PwaInstallInstructionsDialog
        open={installHelpOpen}
        onClose={() => setInstallHelpOpen(false)}
      />

      {/* ── Restart Server (admin only) ─────────────────────────────────── */}
      {isAdmin && (
        <section className="card p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">Server</h2>
          <p className="text-sm text-fg-muted mb-4">
            Restart the server process. The page will automatically reload once the server comes back up.
          </p>
          {restartConfirm ? (
            <div className="flex items-center gap-3">
              <span className="text-sm text-amber-700 dark:text-amber-400 font-medium">
                Are you sure? The server will briefly be unavailable.
              </span>
              <button
                onClick={handleRestartServer}
                disabled={restartLoading}
                className="btn btn-danger btn-md inline-flex items-center"
              >
                {restartLoading ? "Restarting…" : "Yes, restart"}
              </button>
              <button
                onClick={() => setRestartConfirm(false)}
                disabled={restartLoading}
                className="btn btn-secondary btn-md inline-flex items-center"
              >
                Cancel
              </button>
            </div>
          ) : (
            <button
              onClick={handleRestartServer}
              className="inline-flex items-center gap-1.5 bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 text-sm"
            >
              Restart Server
            </button>
          )}
        </section>
      )}

      </div>{/* end masonry config cards */}

      {/* ── About & Credits — pulled out of the masonry so they always sit at
          the bottom of the page. Side-by-side on wide screens, stacked when
          narrow; items-start keeps each card at its natural height. ───────── */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">

      {/* ── About ───────────────────────────────────────────────────────────── */}
      <section className="card p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">About</h2>
        <div className="flex flex-col items-center text-center">
          <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mb-3" />
          <h3 className="text-xl font-bold text-fg">Simple Photos</h3>
          <p className="text-sm text-fg-muted mb-4">
            v1.0.0 — Self-hosted, end-to-end encrypted photo & video library
          </p>
          <hr className="w-full border-edge mb-4" />
          <p className="text-xs text-fg-muted mb-2">Developed by</p>
          <img
            src="/wulfnet.jpg"
            alt="WulfNet Designs"
            className="h-16 mb-1"
          />
          <p className="text-sm font-semibold text-fg-muted">WulfNet Designs</p>
          <p className="text-xs text-fg-muted mt-3">
            &copy; {new Date().getFullYear()} WulfNet Designs. All rights
            reserved.
          </p>
        </div>
      </section>

      {/* ── Credits & Links ─────────────────────────────────────────────────── */}
      <section className="card p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">Credits &amp; Links</h2>
        <div className="space-y-3 text-sm">
          <div className="flex items-center gap-3">
            <AppIcon name="star" size="w-5 h-5" />
            <div>
              <p className="text-fg font-medium">Icons</p>
              <p className="text-fg-muted">
                Custom icons by{" "}
                <a
                  href="https://www.flaticon.com/authors/angus-87"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent-600 dark:text-accent-400 hover:underline"
                >
                  Angus_87
                </a>{" "}
                on{" "}
                <a
                  href="https://www.flaticon.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent-600 dark:text-accent-400 hover:underline"
                >
                  Flaticon
                </a>
              </p>
            </div>
          </div>
          <hr className="border-edge" />
          <div className="flex items-center gap-3">
            <AppIcon name="shared" size="w-5 h-5" />
            <div>
              <p className="text-fg font-medium">Source Code</p>
              <p className="text-fg-muted">
                <a
                  href="https://github.com/wulfic/simple-photos"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent-600 dark:text-accent-400 hover:underline"
                >
                  github.com/wulfic/simple-photos
                </a>
              </p>
            </div>
          </div>
        </div>
      </section>
      </div>{/* end About & Credits footer row */}
      </main>
    </div>
  );
}
