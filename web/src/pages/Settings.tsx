import { useState, useMemo, useEffect, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { QRCodeSVG } from "qrcode.react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import AppHeader from "../components/AppHeader";

/** Password strength check — same logic as Register page. */
function checkPasswordStrength(pw: string) {
  const checks = {
    length: pw.length >= 8,
    uppercase: /[A-Z]/.test(pw),
    lowercase: /[a-z]/.test(pw),
    digit: /\d/.test(pw),
    long: pw.length >= 12,
    special: /[^A-Za-z0-9]/.test(pw),
  };
  const core = checks.length && checks.uppercase && checks.lowercase && checks.digit;
  const score = Object.values(checks).filter(Boolean).length;
  const label =
    score <= 2 ? "Weak" : score <= 3 ? "Fair" : score <= 4 ? "Good" : "Strong";
  const color =
    score <= 2
      ? "bg-red-500"
      : score <= 3
        ? "bg-yellow-500"
        : score <= 4
          ? "bg-blue-500"
          : "bg-green-500";
  return { checks, core, score, label, color, max: 6 };
}

export default function Settings() {
  const { username } = useAuthStore();
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

  // ── Encrypted galleries state ────────────────────────────────────────────
  const [galleries, setGalleries] = useState<Array<{ id: string; name: string; created_at: string; item_count: number }>>([]);
  const [galleriesLoading, setGalleriesLoading] = useState(true);
  const [showCreateGallery, setShowCreateGallery] = useState(false);
  const [newGalleryName, setNewGalleryName] = useState("");
  const [newGalleryPassword, setNewGalleryPassword] = useState("");
  const [newGalleryConfirm, setNewGalleryConfirm] = useState("");

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

  const loadGalleries = useCallback(async () => {
    try {
      const res = await api.encryptedGalleries.list();
      setGalleries(res.galleries);
    } catch {
      // Ignore if galleries table doesn't exist yet
    } finally {
      setGalleriesLoading(false);
    }
  }, []);

  // Fetch encryption settings and galleries on mount
  useEffect(() => {
    loadEncryptionSettings();
    loadGalleries();
  }, [loadEncryptionSettings, loadGalleries]);

  // Poll migration progress when a migration is active
  useEffect(() => {
    if (migrationStatus !== "encrypting" && migrationStatus !== "decrypting") return;
    const interval = setInterval(loadEncryptionSettings, 3000);
    return () => clearInterval(interval);
  }, [migrationStatus, loadEncryptionSettings]);

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

  async function handleCreateGallery(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    if (newGalleryPassword !== newGalleryConfirm) {
      setError("Gallery passwords do not match.");
      return;
    }
    setLoading(true);
    try {
      await api.encryptedGalleries.create(newGalleryName, newGalleryPassword);
      setSuccess(`Encrypted gallery "${newGalleryName}" created.`);
      setShowCreateGallery(false);
      setNewGalleryName("");
      setNewGalleryPassword("");
      setNewGalleryConfirm("");
      await loadGalleries();
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleDeleteGallery(id: string, name: string) {
    if (!confirm(`Delete encrypted gallery "${name}"? All items inside will be removed.`)) return;
    setError("");
    try {
      await api.encryptedGalleries.delete(id);
      setSuccess(`Encrypted gallery "${name}" deleted.`);
      await loadGalleries();
    } catch (err: any) {
      setError(err.message);
    }
  }

  const Checkmark = ({ ok }: { ok: boolean }) => (
    <span className={ok ? "text-green-600 dark:text-green-400" : "text-gray-400"}>
      {ok ? "✓" : "○"}
    </span>
  );

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
            {/* Current mode display */}
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
                  Storage Mode
                </h3>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  {encryptionMode === "encrypted" ? (
                    <>
                      <span className="inline-flex items-center gap-1 text-amber-600 dark:text-amber-400 font-medium">
                        🔒 Encrypted
                      </span>
                      {" — "}All photos are end-to-end encrypted in blob storage.
                    </>
                  ) : (
                    <>
                      <span className="inline-flex items-center gap-1 text-green-600 dark:text-green-400 font-medium">
                        📁 Standard
                      </span>
                      {" — "}Photos are stored as regular files on disk.
                    </>
                  )}
                </p>
              </div>
              {migrationStatus === "idle" && (
                <button
                  onClick={() => setShowEncryptionWarning(true)}
                  disabled={togglingEncryption}
                  className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm whitespace-nowrap disabled:opacity-50"
                >
                  {encryptionMode === "plain" ? "Enable Encryption" : "Disable Encryption"}
                </button>
              )}
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
                  {encryptionMode === "plain"
                    ? "All existing photos will be encrypted and stored as opaque blobs. This process runs in the background and may take a while for large libraries. Files will no longer be directly accessible on disk."
                    : "All encrypted photos will be decrypted and stored as regular files on disk. Anyone with server access will be able to view the files directly. Encrypted galleries are not affected by this change."}
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

            {/* Info about encrypted galleries */}
            <p className="text-xs text-gray-400 dark:text-gray-500">
              Encrypted galleries are always encrypted regardless of the global setting above.
            </p>
          </div>
        )}
      </section>

      {/* ── Encrypted Galleries ──────────────────────────────────────────── */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-lg font-semibold">Encrypted Galleries</h2>
          {!showCreateGallery && (
            <button
              onClick={() => {
                setShowCreateGallery(true);
                setError("");
              }}
              className="bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-700 text-sm"
            >
              + New Gallery
            </button>
          )}
        </div>

        <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
          Encrypted galleries are always end-to-end encrypted and password-protected, independent of<br className="hidden sm:inline" /> your global storage mode. Use them to keep sensitive photos separate and secure.
        </p>

        {/* Create gallery form */}
        {showCreateGallery && (
          <form onSubmit={handleCreateGallery} className="bg-gray-50 dark:bg-gray-700/50 rounded-lg p-4 mb-4 space-y-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Gallery Name
              </label>
              <input
                type="text"
                value={newGalleryName}
                onChange={(e) => setNewGalleryName(e.target.value)}
                placeholder="e.g. Private Photos"
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-800 dark:border-gray-600"
                required
                maxLength={100}
                autoFocus
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Gallery Password
              </label>
              <input
                type="password"
                value={newGalleryPassword}
                onChange={(e) => setNewGalleryPassword(e.target.value)}
                placeholder="At least 4 characters"
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-800 dark:border-gray-600"
                required
                minLength={4}
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Confirm Password
              </label>
              <input
                type="password"
                value={newGalleryConfirm}
                onChange={(e) => setNewGalleryConfirm(e.target.value)}
                className="w-full border rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-800 dark:border-gray-600"
                required
                minLength={4}
              />
              {newGalleryConfirm.length > 0 && newGalleryPassword !== newGalleryConfirm && (
                <p className="text-xs text-red-500 dark:text-red-400 mt-1">Passwords do not match</p>
              )}
            </div>
            <div className="flex gap-2">
              <button
                type="submit"
                disabled={loading || !newGalleryName || newGalleryPassword.length < 4 || newGalleryPassword !== newGalleryConfirm}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                {loading ? "Creating…" : "Create Gallery"}
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowCreateGallery(false);
                  setNewGalleryName("");
                  setNewGalleryPassword("");
                  setNewGalleryConfirm("");
                }}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </form>
        )}

        {/* Gallery list */}
        {galleriesLoading ? (
          <div className="text-gray-400 text-sm">Loading galleries…</div>
        ) : galleries.length === 0 ? (
          <div className="text-center py-6 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
            <p className="text-gray-400 text-sm">No encrypted galleries yet.</p>
          </div>
        ) : (
          <div className="space-y-2">
            {galleries.map((g) => (
              <div
                key={g.id}
                className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg"
              >
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm">🔒</span>
                    <span className="font-medium text-gray-900 dark:text-gray-100">{g.name}</span>
                  </div>
                  <p className="text-xs text-gray-400 mt-0.5">
                    {g.item_count} item{g.item_count !== 1 ? "s" : ""} · Created {new Date(g.created_at).toLocaleDateString()}
                  </p>
                </div>
                <button
                  onClick={() => handleDeleteGallery(g.id, g.name)}
                  className="text-red-500 hover:text-red-600 text-sm px-2 py-1"
                  title="Delete gallery"
                >
                  Delete
                </button>
              </div>
            ))}
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
