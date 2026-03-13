/** Wizard step — create the first admin account (username + password). */
import type { WizardStep } from "./types";
import type { PasswordStrength } from "../../utils/validation";
import { Checkmark, PasswordField, ConfirmPasswordField } from "../../components/PasswordFields";

export interface AccountStepProps {
  username: string;
  setUsername: (v: string) => void;
  password: string;
  setPassword: (v: string) => void;
  confirmPassword: string;
  setConfirmPassword: (v: string) => void;
  pw: PasswordStrength;
  un: { length: boolean; chars: boolean };
  loading: boolean;
  error: string;
  handleCreateAccount: (e: React.FormEvent) => void;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
}

export default function AccountStep({
  username,
  setUsername,
  password,
  setPassword,
  confirmPassword,
  setConfirmPassword,
  pw,
  un,
  loading,
  error,
  handleCreateAccount,
  setStep,
  setError,
}: AccountStepProps) {
  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
        Create Admin Account
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        This will be the first account with full admin privileges.
      </p>

      <form onSubmit={handleCreateAccount} className="space-y-4">
        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
            Username
          </label>
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            required
            minLength={3}
            maxLength={50}
            autoComplete="username"
            autoFocus
            placeholder="e.g. admin"
          />
          {username.length > 0 && (
            <ul className="text-xs mt-1.5 space-y-0.5">
              <li>
                <Checkmark ok={un.length} /> 3–50 characters
              </li>
              <li>
                <Checkmark ok={un.chars} /> Letters, numbers, underscores
                only
              </li>
            </ul>
          )}
        </div>

        <PasswordField
          value={password}
          onChange={setPassword}
          pwData={pw}
        />

        <ConfirmPasswordField
          value={confirmPassword}
          onChange={setConfirmPassword}
          password={password}
        />

        {error && (
          <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
            {error}
          </div>
        )}

        <div className="flex gap-3 pt-2">
          <button
            type="button"
            onClick={() => {
              setStep("server-role");
              setError("");
            }}
            className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 dark:bg-gray-600 text-sm font-medium transition-colors"
          >
            ← Back
          </button>
          <button
            type="submit"
            disabled={loading || !pw.core || !un.length || !un.chars}
            className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
          >
            {loading ? "Creating account\u2026" : "Create Account →"}
          </button>
        </div>
      </form>
    </div>
  );
}
