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
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
        Two-Factor Authentication
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        {step === "admin-2fa"
          ? "Secure your admin account with 2FA. Highly recommended."
          : `Set up 2FA for ${pendingTotpUser?.username ?? "the new user"}. Each user can also do this later in Settings.`}
      </p>

      {!totpData && !totpConfirmed && (
        <div className="text-center space-y-4">
          <div className="bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4 text-sm text-blue-800 dark:text-blue-300">
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
                className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors"
              >
                ← Back
              </button>
            )}
            <button
              onClick={skipTotpStep}
              className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
            >
              Skip for now →
            </button>
            {step === "admin-2fa" && (
              <button
                onClick={startTotpSetup}
                disabled={loading}
                className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
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
            <div className="bg-white dark:bg-gray-800 p-4 rounded-lg border-2 border-gray-200 dark:border-gray-700">
              <QRCodeSVG
                value={totpData.otpauth_uri}
                size={200}
              />
            </div>
          </div>
          <details className="text-xs text-gray-500 dark:text-gray-400">
            <summary className="cursor-pointer hover:text-gray-700 dark:hover:text-gray-300 dark:text-gray-300">
              Can't scan? Enter manually
            </summary>
            <code className="block mt-2 bg-gray-50 dark:bg-gray-900 p-2 rounded break-all font-mono">
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
          <p className="text-center text-sm text-gray-600 dark:text-gray-400">
            Scan this QR code with your authenticator app, then enter the
            6-digit code below.
          </p>
          <form onSubmit={confirmTotp} className="space-y-3">
            <input
              type="text"
              value={totpCode}
              onChange={(e) => setTotpCode(e.target.value)}
              className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 text-center text-lg tracking-widest focus:outline-none focus:ring-2 focus:ring-blue-500"
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
              className="w-full bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium"
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
              <p className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                Save these backup codes somewhere safe. Each can be used
                once if you lose your authenticator:
              </p>
              <div className="bg-gray-50 dark:bg-gray-900 rounded-lg p-4 font-mono text-sm grid grid-cols-2 gap-1">
                {backupCodes.map((code, i) => (
                  <div key={i} className="text-gray-700 dark:text-gray-300">
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
                className="mt-2 text-blue-600 text-sm hover:underline"
              >
                Copy all codes
              </button>
            </div>
          )}

          <button
            onClick={finishTotpStep}
            className="w-full bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 text-sm font-medium transition-colors"
          >
            Continue →
          </button>
        </div>
      )}
    </div>
  );
}
