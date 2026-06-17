/** Wizard step — set up TOTP two-factor authentication for the admin account. */
import { QRCodeSVG } from "qrcode.react";
import type { TotpSetupResponse } from "../../api/client";
import type { WizardStep, CreatedUser } from "./types";

export interface TwoFactorStepProps {
  totpData: TotpSetupResponse | null;
  totpCode: string;
  setTotpCode: (v: string) => void;
  backupCodes: string[];
  totpConfirmed: boolean;
  loading: boolean;
  error: string;
  startTotpSetup: () => void;
  confirmTotp: (e: React.FormEvent) => void;
  finishTotpStep: () => void;
  skipTotpStep: () => void;
  step: WizardStep;
  pendingTotpUser: CreatedUser | null;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
}

export default function TwoFactorStep({
  totpData,
  totpCode,
  setTotpCode,
  backupCodes,
  totpConfirmed,
  loading,
  error,
  startTotpSetup,
  confirmTotp,
  finishTotpStep,
  skipTotpStep,
  step,
  pendingTotpUser,
  setStep,
  setError,
}: TwoFactorStepProps) {
  return (
    <div>
      <h2 className="text-2xl font-bold text-fg mb-1">
        Two-Factor Authentication
      </h2>
      <p className="text-fg-muted text-sm mb-6">
        {step === "admin-2fa"
          ? "Secure your admin account with 2FA. Highly recommended."
          : `Set up 2FA for ${pendingTotpUser?.username ?? "the new user"}. Each user can also do this later in Settings.`}
      </p>

      {!totpData && !totpConfirmed && (
        <div className="text-center space-y-4">
          <div className="bg-accent-50 dark:bg-accent-900/30 rounded-lg p-4 text-sm text-accent-800 dark:text-accent-300">
            <p>
              Two-factor authentication adds an extra layer of security.
              You'll need an authenticator app like Google Authenticator,
              Authy, or 1Password.
            </p>
          </div>
          {step === "user-2fa" && (
            <div className="bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-800 dark:text-amber-300">
              <strong>Note:</strong> 2FA can only be set up while logged
              in as that user. The user can enable it themselves in Settings
              after their first login.
            </div>
          )}
          <div className="flex gap-3">
            {step === "admin-2fa" && (
              <button
                onClick={() => {
                  setStep("account");
                  setError("");
                }}
                className="btn btn-secondary btn-md flex-1"
              >
                ← Back
              </button>
            )}
            <button
              onClick={skipTotpStep}
              className="btn btn-secondary btn-md flex-1"
            >
              Skip for now →
            </button>
            {step === "admin-2fa" && (
              <button
                onClick={startTotpSetup}
                disabled={loading}
                className="btn btn-primary btn-md flex-[2]"
              >
                {loading ? "Setting up\u2026" : "Enable 2FA →"}
              </button>
            )}
          </div>
        </div>
      )}

      {totpData && !totpConfirmed && (
        <div className="space-y-4">
          <div className="flex justify-center">
            <div className="bg-surface p-4 rounded-lg border-2 border-edge">
              <QRCodeSVG
                value={totpData.otpauth_uri}
                size={200}
              />
            </div>
          </div>
          <details className="text-xs text-fg-muted">
            <summary className="cursor-pointer hover:text-fg dark:text-gray-300">
              Can't scan? Enter manually
            </summary>
            <code className="block mt-2 bg-canvas p-2 rounded break-all font-mono">
              {(() => {
                try {
                  const url = new URL(totpData.otpauth_uri);
                  return url.searchParams.get("secret") ?? totpData.otpauth_uri;
                } catch {
                  return totpData.otpauth_uri;
                }
              })()}
            </code>
          </details>
          <p className="text-center text-sm text-fg-muted">
            Scan this QR code with your authenticator app, then enter the
            6-digit code below.
          </p>
          <form onSubmit={confirmTotp} className="space-y-3">
            <input
              type="text"
              value={totpCode}
              onChange={(e) => setTotpCode(e.target.value)}
              className="input text-center text-lg tracking-widest"
              placeholder="000000"
              maxLength={6}
              pattern="\d{6}"
              autoFocus
              required
            />
            {error && (
              <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
                {error}
              </div>
            )}
            <button
              type="submit"
              disabled={loading || totpCode.length !== 6}
              className="btn btn-primary btn-md w-full"
            >
              {loading ? "Verifying\u2026" : "Verify Code"}
            </button>
          </form>
        </div>
      )}

      {totpConfirmed && (
        <div className="space-y-4">
          <div className="bg-green-50 dark:bg-green-900/30 rounded-lg p-4 text-center">
            <span className="text-green-600 dark:text-green-400 text-2xl">{"\u2713"}</span>
            <p className="text-green-800 dark:text-green-300 font-medium mt-1">
              2FA Enabled!
            </p>
          </div>

          {backupCodes.length > 0 && (
            <div>
              <p className="text-sm font-medium text-fg-muted mb-2">
                Save these backup codes somewhere safe. Each can be used
                once if you lose your authenticator:
              </p>
              <div className="bg-canvas rounded-lg p-4 font-mono text-sm grid grid-cols-2 gap-1">
                {backupCodes.map((code, i) => (
                  <div key={i} className="text-fg-muted">
                    {code}
                  </div>
                ))}
              </div>
              <button
                onClick={() => {
                  navigator.clipboard.writeText(
                    backupCodes.join("\n")
                  );
                }}
                className="mt-2 text-accent-600 text-sm hover:underline"
              >
                Copy all codes
              </button>
            </div>
          )}

          <button
            onClick={finishTotpStep}
            className="btn btn-primary btn-md w-full"
          >
            Continue →
          </button>
        </div>
      )}
    </div>
  );
}
