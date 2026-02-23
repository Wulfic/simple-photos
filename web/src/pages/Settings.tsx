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

// Minimal role check: decode JWT payload to see if user is admin
function useIsAdmin(): boolean {
  const { accessToken } = useAuthStore();
  if (!accessToken) return false;
  try {
    const payload = JSON.parse(atob(accessToken.split(".")[1]));
    return payload.role === "admin";
  } catch {
    return false;
  }
}

export default function Settings() {
  const { username } = useAuthStore();
  const isAdmin = useIsAdmin();
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

  // ── SSL / TLS state (admin only) ──────────────────────────────────────────
  const [sslEnabled, setSslEnabled] = useState(false);
  const [sslCertPath, setSslCertPath] = useState("");
  const [sslKeyPath, setSslKeyPath] = useState("");
  const [sslLoaded, setSslLoaded] = useState(false);
  const [sslSaving, setSslSaving] = useState(false);
  const [sslSaved, setSslSaved] = useState(false);
  const [sslMode, setSslMode] = useState<"manual" | "letsencrypt">("manual");
  const [leDomain, setLeDomain] = useState("");
  const [leEmail, setLeEmail] = useState("");
  const [leStaging, setLeStaging] = useState(false);
  const [leGenerating, setLeGenerating] = useState(false);
  const [leGenerated, setLeGenerated] = useState(false);

  // ── User Management state (admin only) ─────────────────────────────────
  type ManagedUser = { id: string; username: string; role: string; totp_enabled: boolean; created_at: string };
  const [managedUsers, setManagedUsers] = useState<ManagedUser[]>([]);
  const [usersLoaded, setUsersLoaded] = useState(false);
  const [showAddUser, setShowAddUser] = useState(false);
  const [newUsername, setNewUsername] = useState("");
  const [newUserPassword, setNewUserPassword] = useState("");
  const [newUserRole, setNewUserRole] = useState<"user" | "admin">("user");
  const [resetPwUserId, setResetPwUserId] = useState<string | null>(null);
  const [resetPwValue, setResetPwValue] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

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
    loadSslSettings();
    loadManagedUsers();
  }, [loadEncryptionSettings, loadBackupServers]);

  async function loadSslSettings() {
    try {
      const res = await api.admin.getSsl();
      setSslEnabled(res.enabled);
      setSslCertPath(res.cert_path ?? "");
      setSslKeyPath(res.key_path ?? "");
      setSslLoaded(true);
    } catch {
      // Not admin or SSL endpoints not available — silently skip
    }
  }

  // ── User Management handlers (admin only) ────────────────────────────────

  async function loadManagedUsers() {
    try {
      const users = await api.admin.listUsers();
      setManagedUsers(users);
      setUsersLoaded(true);
    } catch {
      // Not admin — silently skip
    }
  }

  async function handleAddUser(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.admin.createUser(newUsername, newUserPassword, newUserRole);
      setSuccess(`User "${newUsername}" created.`);
      setNewUsername("");
      setNewUserPassword("");
      setNewUserRole("user");
      setShowAddUser(false);
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleDeleteUser(userId: string) {
    setError("");
    try {
      await api.admin.deleteUser(userId);
      setSuccess("User deleted.");
      setConfirmDeleteId(null);
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleChangeRole(userId: string, role: "admin" | "user") {
    setError("");
    try {
      await api.admin.updateUserRole(userId, role);
      setSuccess("Role updated.");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleResetUserPassword(userId: string) {
    setError("");
    if (!resetPwValue || resetPwValue.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    try {
      await api.admin.resetUserPassword(userId, resetPwValue);
      setSuccess("Password reset.");
      setResetPwUserId(null);
      setResetPwValue("");
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleResetUser2fa(userId: string) {
    setError("");
    try {
      await api.admin.resetUser2fa(userId);
      setSuccess("2FA disabled for user.");
      await loadManagedUsers();
    } catch (err: any) {
      setError(err.message);
    }
  }

  async function handleSaveSsl() {
    setSslSaving(true);
    setError("");
    try {
      await api.admin.updateSsl({
        enabled: sslEnabled,
        cert_path: sslCertPath || undefined,
        key_path: sslKeyPath || undefined,
      });
      setSslSaved(true);
      setSuccess("TLS configuration saved. Restart the server to apply changes.");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setSslSaving(false);
    }
  }

  async function handleGenerateLeCert() {
    if (!leDomain.trim() || !leEmail.trim()) {
      setError("Domain and e-mail are both required.");
      return;
    }
    setLeGenerating(true);
    setError("");
    try {
      const res = await api.admin.generateLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        staging: leStaging,
      });
      setLeGenerated(true);
      setSslEnabled(true);
      setSslCertPath(res.cert_path);
      setSslKeyPath(res.key_path);
      setSuccess(res.message);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLeGenerating(false);
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

      {/* ── SSL / TLS (admin only) ─────────────────────────────────────────── */}
      {sslLoaded && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <h2 className="text-lg font-semibold mb-3">SSL / TLS</h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
            Serve your photos over HTTPS with a TLS certificate.
            Changes require a server restart.
          </p>

          {/* Enable toggle */}
          <div className="flex items-center justify-between mb-4">
            <div>
              <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">Enable TLS</h3>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                {sslEnabled ? "HTTPS is enabled." : "Running on plain HTTP."}
              </p>
            </div>
            <button
              onClick={() => {
                setSslEnabled(!sslEnabled);
                setSslSaved(false);
              }}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                sslEnabled ? "bg-blue-600" : "bg-gray-300 dark:bg-gray-600"
              }`}
              role="switch"
              aria-checked={sslEnabled}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  sslEnabled ? "translate-x-6" : "translate-x-1"
                }`}
              />
            </button>
          </div>

          {/* Mode tabs */}
          {sslEnabled && (
            <div className="space-y-4">
              <div className="flex gap-2 mb-3">
                <button
                  onClick={() => setSslMode("manual")}
                  className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                    sslMode === "manual"
                      ? "bg-blue-600 text-white"
                      : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600"
                  }`}
                >
                  Manual Certificate
                </button>
                <button
                  onClick={() => setSslMode("letsencrypt")}
                  className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                    sslMode === "letsencrypt"
                      ? "bg-green-600 text-white"
                      : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600"
                  }`}
                >
                  Let's Encrypt
                </button>
              </div>

              {/* Manual cert fields */}
              {sslMode === "manual" && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Certificate Path
                    </label>
                    <input
                      type="text"
                      value={sslCertPath}
                      onChange={(e) => { setSslCertPath(e.target.value); setSslSaved(false); }}
                      placeholder="/etc/ssl/certs/my-cert.pem"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Private Key Path
                    </label>
                    <input
                      type="text"
                      value={sslKeyPath}
                      onChange={(e) => { setSslKeyPath(e.target.value); setSslSaved(false); }}
                      placeholder="/etc/ssl/private/my-key.pem"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <button
                    onClick={handleSaveSsl}
                    disabled={sslSaving}
                    className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
                  >
                    {sslSaving ? "Saving…" : sslSaved ? "✓ Saved" : "Save TLS Configuration"}
                  </button>
                </div>
              )}

              {/* Let's Encrypt */}
              {sslMode === "letsencrypt" && !leGenerated && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Domain Name
                    </label>
                    <input
                      type="text"
                      value={leDomain}
                      onChange={(e) => setLeDomain(e.target.value)}
                      placeholder="photos.example.com"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Contact E-mail
                    </label>
                    <input
                      type="email"
                      value={leEmail}
                      onChange={(e) => setLeEmail(e.target.value)}
                      placeholder="you@example.com"
                      className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
                    <input
                      type="checkbox"
                      checked={leStaging}
                      onChange={(e) => setLeStaging(e.target.checked)}
                      className="accent-blue-600"
                    />
                    Use staging environment (testing only)
                  </label>
                  <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-700 dark:text-amber-400">
                    Port 80 must be available and the domain must resolve to this server.
                  </div>
                  <button
                    onClick={handleGenerateLeCert}
                    disabled={leGenerating}
                    className="bg-green-600 text-white px-4 py-2 rounded-md hover:bg-green-700 disabled:opacity-50 text-sm"
                  >
                    {leGenerating ? (
                      <span className="flex items-center gap-2">
                        <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                        Generating…
                      </span>
                    ) : (
                      "Generate Let's Encrypt Certificate"
                    )}
                  </button>
                </div>
              )}

              {/* LE success */}
              {sslMode === "letsencrypt" && leGenerated && (
                <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-4 flex items-start gap-2">
                  <svg className="w-5 h-5 text-green-600 mt-0.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                  <div>
                    <p className="text-sm font-medium text-green-700 dark:text-green-300">Certificate generated!</p>
                    <p className="text-xs text-green-600 dark:text-green-400 mt-1">
                      Restart the server to start serving HTTPS on {leDomain}.
                    </p>
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Disable save btn */}
          {!sslEnabled && (
            <button
              onClick={handleSaveSsl}
              disabled={sslSaving}
              className="mt-2 bg-gray-600 text-white px-4 py-2 rounded-md hover:bg-gray-700 disabled:opacity-50 text-sm"
            >
              {sslSaving ? "Saving…" : "Disable TLS & Save"}
            </button>
          )}
        </section>
      )}

      {/* ── Manage Users (admin only) ────────────────────────────────────── */}
      {usersLoaded && isAdmin && (
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold">Manage Users</h2>
            <button
              onClick={() => setShowAddUser(!showAddUser)}
              className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
              </svg>
              Add User
            </button>
          </div>

          {/* Add user form */}
          {showAddUser && (
            <form onSubmit={handleAddUser} className="mb-4 p-4 bg-gray-50 dark:bg-gray-700/50 rounded-lg space-y-3">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Username</label>
                <input
                  type="text"
                  value={newUsername}
                  onChange={(e) => setNewUsername(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                  required
                  minLength={3}
                  autoFocus
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Password</label>
                <input
                  type="password"
                  value={newUserPassword}
                  onChange={(e) => setNewUserPassword(e.target.value)}
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                  required
                  minLength={8}
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Role</label>
                <div className="flex gap-4">
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      checked={newUserRole === "user"}
                      onChange={() => setNewUserRole("user")}
                      className="accent-blue-600"
                    />
                    User
                  </label>
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      checked={newUserRole === "admin"}
                      onChange={() => setNewUserRole("admin")}
                      className="accent-blue-600"
                    />
                    Admin
                  </label>
                </div>
              </div>
              <div className="flex gap-2">
                <button type="submit" className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm">
                  Create User
                </button>
                <button type="button" onClick={() => setShowAddUser(false)} className="px-4 py-2 rounded-md text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700">
                  Cancel
                </button>
              </div>
            </form>
          )}

          {/* User table */}
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-gray-200 dark:border-gray-700 text-left">
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Username</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Role</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">2FA</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400">Created</th>
                  <th className="pb-2 font-medium text-gray-500 dark:text-gray-400 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {managedUsers.map((u) => (
                  <tr key={u.id} className="border-b border-gray-100 dark:border-gray-700/50">
                    <td className="py-2.5 font-medium">{u.username}</td>
                    <td className="py-2.5">
                      <select
                        value={u.role}
                        onChange={(e) => handleChangeRole(u.id, e.target.value as "admin" | "user")}
                        className="text-xs border rounded px-2 py-1 bg-transparent focus:outline-none focus:ring-1 focus:ring-blue-500"
                      >
                        <option value="user">User</option>
                        <option value="admin">Admin</option>
                      </select>
                    </td>
                    <td className="py-2.5">
                      {u.totp_enabled ? (
                        <span className="inline-flex items-center gap-1 text-green-600 dark:text-green-400 text-xs">
                          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                          </svg>
                          Enabled
                        </span>
                      ) : (
                        <span className="text-xs text-gray-400">Off</span>
                      )}
                    </td>
                    <td className="py-2.5 text-xs text-gray-500 dark:text-gray-400">
                      {new Date(u.created_at).toLocaleDateString()}
                    </td>
                    <td className="py-2.5 text-right">
                      <div className="flex items-center justify-end gap-1">
                        {/* Reset Password */}
                        <button
                          onClick={() => { setResetPwUserId(resetPwUserId === u.id ? null : u.id); setResetPwValue(""); }}
                          className="p-1.5 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400"
                          title="Reset password"
                        >
                          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25a3 3 0 013 3m3 0a6 6 0 01-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 1121.75 8.25z" />
                          </svg>
                        </button>
                        {/* Reset 2FA (only if enabled) */}
                        {u.totp_enabled && (
                          <button
                            onClick={() => handleResetUser2fa(u.id)}
                            className="p-1.5 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400"
                            title="Reset 2FA"
                          >
                            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
                            </svg>
                          </button>
                        )}
                        {/* Delete */}
                        <button
                          onClick={() => setConfirmDeleteId(confirmDeleteId === u.id ? null : u.id)}
                          className="p-1.5 rounded hover:bg-red-50 dark:hover:bg-red-900/20 text-red-500"
                          title="Delete user"
                        >
                          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
                          </svg>
                        </button>
                      </div>
                      {/* Reset Password inline form */}
                      {resetPwUserId === u.id && (
                        <div className="flex gap-1 mt-2 justify-end">
                          <input
                            type="password"
                            value={resetPwValue}
                            onChange={(e) => setResetPwValue(e.target.value)}
                            placeholder="New password"
                            className="border rounded px-2 py-1 text-xs w-36 focus:outline-none focus:ring-1 focus:ring-blue-500"
                            autoFocus
                          />
                          <button
                            onClick={() => handleResetUserPassword(u.id)}
                            className="bg-blue-600 text-white px-2 py-1 rounded text-xs hover:bg-blue-700"
                          >
                            Set
                          </button>
                        </div>
                      )}
                      {/* Delete confirmation */}
                      {confirmDeleteId === u.id && (
                        <div className="flex items-center gap-1 mt-2 justify-end">
                          <span className="text-xs text-red-600 dark:text-red-400">Delete?</span>
                          <button
                            onClick={() => handleDeleteUser(u.id)}
                            className="bg-red-600 text-white px-2 py-1 rounded text-xs hover:bg-red-700"
                          >
                            Yes
                          </button>
                          <button
                            onClick={() => setConfirmDeleteId(null)}
                            className="px-2 py-1 rounded text-xs text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700"
                          >
                            No
                          </button>
                        </div>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
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
      </main>
    </div>
  );
}
