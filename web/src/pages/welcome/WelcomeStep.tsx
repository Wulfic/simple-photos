/** Wizard step — initial landing screen with existing-install detection. */
import type { WizardStep, SetupStatus } from "./types";

export interface WelcomeStepProps {
  setStep: (step: WizardStep) => void;
  status: SetupStatus | null;
  error: string;
}

export default function WelcomeStep({ setStep, status, error }: WelcomeStepProps) {
  return (
    <div className="text-center">
      <img src="/logo.png" alt="Simple Photos" className="w-24 h-24 mx-auto mb-4" />
      <h1 className="text-3xl font-bold text-gray-900 dark:text-gray-100 mb-2">
        Welcome to Simple Photos
      </h1>
      <p className="text-gray-600 dark:text-gray-400 mb-2">
        Your self-hosted, end-to-end encrypted photo & video library.
      </p>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-8">
        Let's get you set up. This will only take a minute.
      </p>

      {status && (
        <div className="text-left bg-blue-50 dark:bg-blue-900/30 rounded-lg p-4 mb-6 text-sm">
          <div className="flex items-center gap-2 mb-2">
            <span className="w-2 h-2 rounded-full bg-green-500" />
            <span className="text-gray-700 dark:text-gray-300">
              Server connected — v{status.version}
            </span>
          </div>
          <p className="text-gray-600 dark:text-gray-400">
            No users exist yet. You'll create the admin account next.
          </p>
        </div>
      )}

      {error && (
        <div className="bg-red-50 dark:bg-red-900/30 rounded-lg p-4 mb-6 text-sm text-red-700 dark:text-red-400">
          {error}
        </div>
      )}

      <button
        onClick={() => setStep("server-role")}
        disabled={!!error && !status}
        className="w-full bg-blue-600 text-white py-3 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-lg font-medium transition-colors"
      >
        Get Started →
      </button>
    </div>
  );
}
