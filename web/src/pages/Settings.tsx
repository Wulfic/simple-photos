import { useState, useMemo, useEffect, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { QRCodeSVG } from "qrcode.react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { useBackupStore } from "../store/backup";
import { useProcessingStore } from "../store/processing";
import AppHeader from "../components/AppHeader";
import { checkPasswordStrength } from "../utils/validation";
import { Checkmark } from "../components/PasswordFields";

export default function Settings() {
  const { username } = useAuthStore();
  const { startTask, endTask } = useProcessingStore();
  const navigate = useNavigate();

  // ── 2FA state ────────────────────────────────────────────────────────────
  const [totpUri, setTotpUri] = useState<string | null>(null);
  const [backupCodes, setBackupCodes] = useState<string[]>([]);
  const [totpCode, setTotpCode] = useState("");
  const [disableCode, setDisableCode] = useState("");
  const [showDisable2fa, setShowDisable2fa] = useState(false);

  // ── Password change state ────────────────────────────────────────────────
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmNewPassword, setConfirmNewPassword] = useState("");

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
  const { backupServers, loaded: backupLoaded, recovering, setRecovering, setBackupServers, setLoaded: setBackupLoaded } = useBackupStore();

  const pw = useMemo(() => checkPasswordStrength(newPassword), [newPassword]);

  // ── 2FA handlers ─────────────────────────────────────────────────────────

  async function handleSetup2fa() {
    setError("");
    setSuccess("");
    setLoading(true);
    try {
      const res = await api.auth.setup2fa();
      setTotpUri(res.otpauth_uri);
      setBackupCodes(res.backup_codes);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleConfirm2fa(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await api.auth.confirm2fa(totpCode);
      setSuccess("Two-factor authentication enabled successfully!");
      setTotpUri(null);
      setTotpCode("");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleDisable2fa(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await api.auth.disable2fa(disableCode);
      setSuccess("Two-factor authentication disabled.");
      setShowDisable2fa(false);
      setDisableCode("");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  // ── Password change handler ──────────────────────────────────────────────

  async function handleChangePassword(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setSuccess("");

    if (!pw.core) {
      setError(
        "New password must be at least 8 characters with uppercase, lowercase, and a digit."
      );
      return;
    }
    if (newPassword !== confirmNewPassword) {
      setError("New passwords do not match.");
      return;
    }
    if (currentPassword === newPassword) {
      setError("New password must be different from current password.");
      return;
    }

    setLoading(true);
    try {
      await api.auth.changePassword(currentPassword, newPassword);
      setSuccess(
        "Password changed successfully. All other sessions have been revoked."
      );
      setShowChangePassword(false);
      setCurrentPassword("");
      setNewPassword("");
      setConfirmNewPassword("");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

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
  }, [loadEncryptionSettings, loadBackupServers]);

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

      {/* ── Account ─────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Account</h2>
        <p className="text-gray-600 dark:text-gray-400">
          Signed in as <span className="font-medium">{username}</span>
        </p>
      </section>

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
            {(migrationStatus === "encrypting" || migrationStatus === "decrypting") && (
              <div className="bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  <div className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
                  <span className="text-sm font-medium text-blue-700 dark:text-blue-300">
                    {migrationStatus === "encrypting" ? "Encrypting" : "Decrypting"} photos…
                  </span>
                </div>
                <div className="w-full h-2 bg-blue-200 dark:bg-blue-800 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-600 rounded-full transition-all duration-500"
                    style={{ width: migrationTotal > 0 ? `${(migrationCompleted / migrationTotal) * 100}%` : "0%" }}
                  />
                </div>
                <p className="text-xs text-blue-600 dark:text-blue-400 mt-1">
                  {migrationCompleted} / {migrationTotal} items processed
                </p>
              </div>
            )}

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
            <p className="text-xs text-gray-400 mt-1">
              Add a backup server in the Setup wizard to enable recovery.
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
      </section>

      {/* ── Import & Apps ───────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Import & Apps</h2>
        <div className="space-y-4">
          <div>
            <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Google Photos Import</h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-2">
              Import photos and videos from a Google Takeout export with full metadata support.
            </p>
            <button
              onClick={() => navigate("/import")}
              className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm"
            >
              Import from Google Photos
            </button>
          </div>
          <hr className="border-gray-100 dark:border-gray-700" />
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

      {/* ── Change Password ─────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Password</h2>

        {!showChangePassword ? (
          <button
            onClick={() => {
              setShowChangePassword(true);
              setError("");
              setSuccess("");
            }}
            className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm"
          >
            Change Password
          </button>
        ) : (
          <form onSubmit={handleChangePassword} className="space-y-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Current Password
              </label>
              <input
                type="password"
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                autoComplete="current-password"
                autoFocus
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                New Password
              </label>
              <input
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                minLength={8}
                maxLength={128}
                autoComplete="new-password"
              />
              {/* Strength bar */}
              {newPassword.length > 0 && (
                <div className="mt-2">
                  <div className="flex items-center gap-2 mb-1">
                    <div className="flex-1 h-1.5 bg-gray-200 dark:bg-gray-600 rounded-full overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-all duration-300 ${pw.color}`}
                        style={{ width: `${(pw.score / pw.max) * 100}%` }}
                      />
                    </div>
                    <span className="text-xs font-medium text-gray-600 dark:text-gray-400 w-12 text-right">
                      {pw.label}
                    </span>
                  </div>
                  <ul className="text-xs space-y-0.5">
                    <li><Checkmark ok={pw.checks.length} /> At least 8 characters</li>
                    <li><Checkmark ok={pw.checks.uppercase} /> Uppercase letter</li>
                    <li><Checkmark ok={pw.checks.lowercase} /> Lowercase letter</li>
                    <li><Checkmark ok={pw.checks.digit} /> Number</li>
                  </ul>
                </div>
              )}
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Confirm New Password
              </label>
              <input
                type="password"
                value={confirmNewPassword}
                onChange={(e) => setConfirmNewPassword(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                autoComplete="new-password"
              />
              {confirmNewPassword.length > 0 &&
                newPassword !== confirmNewPassword && (
                  <p className="text-xs text-red-500 dark:text-red-400 mt-1">
                    Passwords do not match
                  </p>
                )}
            </div>

            <div className="flex gap-2 pt-1">
              <button
                type="submit"
                disabled={loading || !pw.core}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                {loading ? "Saving..." : "Update Password"}
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowChangePassword(false);
                  setCurrentPassword("");
                  setNewPassword("");
                  setConfirmNewPassword("");
                }}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
            <p className="text-xs text-gray-500 dark:text-gray-400">
              Changing your password will sign you out of all other sessions.
            </p>
          </form>
        )}
      </section>

      {/* ── Two-Factor Authentication ───────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Two-Factor Authentication</h2>

        {!totpUri && !showDisable2fa && (
          <div className="flex gap-2">
            <button
              onClick={handleSetup2fa}
              disabled={loading}
              className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm disabled:opacity-50"
            >
              Enable 2FA
            </button>
            <button
              onClick={() => setShowDisable2fa(true)}
              className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
            >
              Disable 2FA
            </button>
          </div>
        )}

        {totpUri && (
          <div className="space-y-4">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Scan this QR code with your authenticator app:
            </p>
            <div className="flex justify-center">
              <QRCodeSVG value={totpUri} size={200} />
            </div>

            {backupCodes.length > 0 && (
              <div>
                <p className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Backup codes (save these somewhere safe):
                </p>
                <div className="bg-gray-100 dark:bg-gray-700 rounded p-3 font-mono text-sm grid grid-cols-2 gap-1">
                  {backupCodes.map((code, i) => (
                    <span key={i}>{code}</span>
                  ))}
                </div>
              </div>
            )}

            <form onSubmit={handleConfirm2fa} className="flex gap-2">
              <input
                type="text"
                value={totpCode}
                onChange={(e) => setTotpCode(e.target.value)}
                placeholder="Enter 6-digit code"
                className="flex-1 border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
              />
              <button
                type="submit"
                disabled={loading}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                Confirm
              </button>
            </form>
          </div>
        )}

        {showDisable2fa && (
          <form onSubmit={handleDisable2fa} className="space-y-3">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Enter a TOTP code to disable two-factor authentication:
            </p>
            <div className="flex gap-2">
              <input
                type="text"
                value={disableCode}
                onChange={(e) => setDisableCode(e.target.value)}
                placeholder="6-digit code"
                className="flex-1 border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
              />
              <button
                type="submit"
                disabled={loading}
                className="bg-red-600 text-white px-4 py-2 rounded-md hover:bg-red-700 disabled:opacity-50 text-sm"
              >
                Disable
              </button>
              <button
                type="button"
                onClick={() => setShowDisable2fa(false)}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </form>
        )}
      </section>

      {/* ── About ───────────────────────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-4">About</h2>
        <div className="flex flex-col items-center text-center">
          <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mb-3" />
          <h3 className="text-xl font-bold text-gray-900 dark:text-gray-100">Simple Photos</h3>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
            v0.1.0 — Self-hosted, end-to-end encrypted photo & video library
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
      </main>
    </div>
  );
}
