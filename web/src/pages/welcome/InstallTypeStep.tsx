/** Wizard step — choose between fresh install and restore-from-backup. */
import type { WizardStep, InstallType } from "./types";

export interface InstallTypeStepProps {
  setStep: (step: WizardStep) => void;
  setInstallType: (type_: InstallType) => void;
  setError: (msg: string) => void;
}

export default function InstallTypeStep({
  setStep,
  setInstallType,
  setError,
}: InstallTypeStepProps) {
  function choose(type_: "fresh" | "restore") {
    setError("");
    setInstallType(type_);
    if (type_ === "fresh") {
      setStep("account");
    } else {
      setStep("restore");
    }
  }

  return (
    <div>
      <h2 className="text-2xl font-bold text-fg mb-2">
        Installation Type
      </h2>
      <p className="text-fg-muted text-sm mb-6">
        Is this a brand new installation, or are you restoring from an existing
        backup server?
      </p>

      <div className="space-y-3">
        {/* Fresh Install */}
        <button
          onClick={() => choose("fresh")}
          className="w-full p-5 text-left rounded-xl border-2 border-edge hover:border-accent-400 dark:hover:border-accent-500 transition-colors group"
        >
          <div className="flex items-center gap-4">
            <div className="w-12 h-12 rounded-full bg-accent-100 dark:bg-accent-900/30 flex items-center justify-center shrink-0">
              <svg
                className="w-6 h-6 text-accent-600 dark:text-accent-400"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={1.5}
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M12 4.5v15m7.5-7.5h-15"
                />
              </svg>
            </div>
            <div>
              <p className="font-semibold text-fg text-base group-hover:text-accent-600 dark:group-hover:text-accent-400 transition-colors">
                Fresh Install
              </p>
              <p className="text-sm text-fg-muted mt-0.5">
                Start from scratch with a new, empty photo library.
              </p>
            </div>
          </div>
        </button>

        {/* Restore from Backup */}
        <button
          onClick={() => choose("restore")}
          className="w-full p-5 text-left rounded-xl border-2 border-edge hover:border-amber-400 dark:hover:border-amber-500 transition-colors group"
        >
          <div className="flex items-center gap-4">
            <div className="w-12 h-12 rounded-full bg-amber-100 dark:bg-amber-900/30 flex items-center justify-center shrink-0">
              <svg
                className="w-6 h-6 text-amber-600 dark:text-amber-400"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={1.5}
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182"
                />
              </svg>
            </div>
            <div>
              <p className="font-semibold text-fg text-base group-hover:text-amber-600 dark:group-hover:text-amber-400 transition-colors">
                Restore from Backup
              </p>
              <p className="text-sm text-fg-muted mt-0.5">
                Recover your photos from an existing Simple Photos backup
                server on your network.
              </p>
            </div>
          </div>
        </button>
      </div>

      <div className="mt-8 pt-6 border-t border-edge">
        <button
          onClick={() => {
            setStep("server-role");
            setError("");
          }}
          className="text-sm text-gray-700 hover:text-fg-muted dark:hover:text-gray-200"
        >
          &larr; Back
        </button>
      </div>
    </div>
  );
}
