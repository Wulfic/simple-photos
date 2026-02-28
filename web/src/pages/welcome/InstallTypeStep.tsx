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
      <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">
        Installation Type
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        Is this a brand new installation, or are you restoring from an existing
        backup server?
      </p>

      <div className="space-y-3">
        {/* Fresh Install */}
        <button
          onClick={() => choose("fresh")}
          className="w-full p-5 text-left rounded-xl border-2 border-gray-200 dark:border-gray-600 hover:border-blue-400 dark:hover:border-blue-500 transition-colors group"
        >
          <div className="flex items-center gap-4">
            <div className="w-12 h-12 rounded-full bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center shrink-0">
              <svg
                className="w-6 h-6 text-blue-600 dark:text-blue-400"
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
              <p className="font-semibold text-gray-900 dark:text-white text-base group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors">
                Fresh Install
              </p>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-0.5">
                Start from scratch with a new, empty photo library.
              </p>
            </div>
          </div>
        </button>

        {/* Restore from Backup */}
        <button
          onClick={() => choose("restore")}
          className="w-full p-5 text-left rounded-xl border-2 border-gray-200 dark:border-gray-600 hover:border-amber-400 dark:hover:border-amber-500 transition-colors group"
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
              <p className="font-semibold text-gray-900 dark:text-white text-base group-hover:text-amber-600 dark:group-hover:text-amber-400 transition-colors">
                Restore from Backup
              </p>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-0.5">
                Recover your photos from an existing Simple Photos backup
                server on your network.
              </p>
            </div>
          </div>
        </button>
      </div>

      <div className="mt-8 pt-6 border-t border-gray-200 dark:border-gray-700">
        <button
          onClick={() => {
            setStep("server-role");
            setError("");
          }}
          className="text-sm text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
        >
          &larr; Back
        </button>
      </div>
    </div>
  );
}
