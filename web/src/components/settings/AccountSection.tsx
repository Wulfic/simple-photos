/** Account settings section — password change, TOTP 2FA setup/disable. */
import { useState, useMemo } from "react";
import { QRCodeSVG } from "qrcode.react";
import { api } from "../../api/client";
import { checkPasswordStrength } from "../../utils/validation";
import { getErrorMessage } from "../../utils/formatters";
import { Checkmark } from "../PasswordFields";

interface AccountSectionProps {
  username: string;
  error: string;
  setError: (e: string) => void;
  success: string;
  setSuccess: (s: string) => void;
  loading: boolean;
  setLoading: (l: boolean) => void;
}

export default function AccountSection({
  username,
  setError,
  setSuccess,
  loading,
  setLoading,
}: AccountSectionProps) {
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
    } catch (err: unknown) {
      setError(getErrorMessage(err));
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
    } catch (err: unknown) {
      setError(getErrorMessage(err));
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
    } catch (err: unknown) {
      setError(getErrorMessage(err));
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
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }

  return (
      <section className="card p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Account</h2>
        <p className="text-fg-muted mb-4">
          Signed in as <span className="font-medium">{username}</span>
        </p>

        {/* ── Change Password ──────────────────────────────────────── */}
        <div className="border-t border-edge pt-4 mt-4">
          <h3 className="text-base font-semibold mb-3">Password</h3>

          {!showChangePassword ? (
            <button
              onClick={() => {
                setShowChangePassword(true);
                setError("");
                setSuccess("");
              }}
              className="btn btn-primary btn-md"
            >
              Change Password
            </button>
          ) : (
            <form onSubmit={handleChangePassword} className="space-y-3">
              <div>
                <label className="block text-sm font-medium text-fg-muted mb-1">
                  Current Password
                </label>
                <input
                  type="password"
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                  className="input"
                  required
                  autoComplete="current-password"
                  autoFocus
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-fg-muted mb-1">
                  New Password
                </label>
                <input
                  type="password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  className="input"
                  required
                  minLength={8}
                  maxLength={128}
                  autoComplete="new-password"
                />
                {/* Strength bar */}
                {newPassword.length > 0 && (
                  <div className="mt-2">
                    <div className="flex items-center gap-2 mb-1">
                      <div className="flex-1 h-1.5 bg-edge-strong rounded-full overflow-hidden">
                        <div
                          className={`h-full rounded-full transition-all duration-300 ${pw.color}`}
                          style={{ width: `${(pw.score / pw.max) * 100}%` }}
                        />
                      </div>
                      <span className="text-xs font-medium text-fg-muted w-12 text-right">
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
                <label className="block text-sm font-medium text-fg-muted mb-1">
                  Confirm New Password
                </label>
                <input
                  type="password"
                  value={confirmNewPassword}
                  onChange={(e) => setConfirmNewPassword(e.target.value)}
                  className="input"
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
                  className="btn btn-primary btn-md"
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
                  className="btn btn-secondary btn-md"
                >
                  Cancel
                </button>
              </div>
              <p className="text-xs text-fg-muted">
                Changing your password will sign you out of all other sessions.
              </p>
            </form>
          )}
        </div>

        {/* ── Two-Factor Authentication ─────────────────────────────── */}
        <div className="border-t border-edge pt-4 mt-4">
          <h3 className="text-base font-semibold mb-3">Two-Factor Authentication</h3>

          {!totpUri && !showDisable2fa && (
            <div className="flex gap-2">
              <button
                onClick={handleSetup2fa}
                disabled={loading}
                className="btn btn-primary btn-md"
              >
                Enable 2FA
              </button>
              <button
                onClick={() => setShowDisable2fa(true)}
                className="btn btn-secondary btn-md"
              >
                Disable 2FA
              </button>
            </div>
          )}

          {totpUri && (
            <div className="space-y-4">
              <p className="text-sm text-fg-muted">
                Scan this QR code with your authenticator app:
              </p>
              <div className="flex justify-center">
                <QRCodeSVG value={totpUri} size={200} />
              </div>

              {backupCodes.length > 0 && (
                <div>
                  <p className="text-sm font-medium text-fg-muted mb-2">
                    Backup codes (save these somewhere safe):
                  </p>
                  <div className="bg-surface-raised rounded p-3 font-mono text-sm grid grid-cols-2 gap-1">
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
                  className="input flex-1"
                  autoFocus
                />
                <button
                  type="submit"
                  disabled={loading}
                  className="btn btn-primary btn-md"
                >
                  Confirm
                </button>
              </form>
            </div>
          )}

          {showDisable2fa && (
            <form onSubmit={handleDisable2fa} className="space-y-3">
              <p className="text-sm text-fg-muted">
                Enter a TOTP code to disable two-factor authentication:
              </p>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={disableCode}
                  onChange={(e) => setDisableCode(e.target.value)}
                  placeholder="6-digit code"
                  className="input flex-1"
                  autoFocus
                />
                <button
                  type="submit"
                  disabled={loading}
                  className="btn btn-danger btn-md"
                >
                  Disable
                </button>
                <button
                  type="button"
                  onClick={() => setShowDisable2fa(false)}
                  className="btn btn-secondary btn-md"
                >
                  Cancel
                </button>
              </div>
            </form>
          )}
        </div>
      </section>
  );
}
