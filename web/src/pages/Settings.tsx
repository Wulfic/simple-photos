import { useState, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { QRCodeSVG } from "qrcode.react";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
import { clearKey } from "../crypto/crypto";
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
  const { username, refreshToken, logout: storeLogout } = useAuthStore();
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

  const pw = useMemo(() => checkPasswordStrength(newPassword), [newPassword]);

  async function handleLogout() {
    try {
      if (refreshToken) {
        await api.auth.logout(refreshToken).catch(() => {});
      }
    } finally {
      clearKey();
      storeLogout();
      navigate("/login");
    }
  }

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
        <div className="flex gap-2 mt-4">
          <button
            onClick={handleLogout}
            className="bg-red-600 text-white px-4 py-2 rounded-md hover:bg-red-700 text-sm"
          >
            Sign Out
          </button>
        </div>
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
