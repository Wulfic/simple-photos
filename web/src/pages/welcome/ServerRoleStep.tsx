import type { WizardStep, ServerRole } from "./types";

export interface ServerRoleStepProps {
  setStep: (step: WizardStep) => void;
  setServerRole: (role: ServerRole) => void;
  setError: (msg: string) => void;
}

export default function ServerRoleStep({
  setStep,
  setServerRole,
  setError,
}: ServerRoleStepProps) {
  function choose(role: "primary" | "backup") {
    setError("");
    setServerRole(role);
    if (role === "primary") {
      setStep("account");
    } else {
      setStep("pair");
    }
  }

  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">
        Server Role
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        Is this the primary server that stores your photos, or a backup that
        mirrors another Simple Photos instance?
      </p>

      <div className="space-y-3">
        {/* Primary server */}
        <button
          onClick={() => choose("primary")}
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
                  d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z"
                />
              </svg>
            </div>
            <div>
              <p className="font-semibold text-gray-900 dark:text-white text-base group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors">
                Primary Server
              </p>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-0.5">
                This is the main server where you upload and manage your photos.
                You can optionally add backup targets later.
              </p>
            </div>
          </div>
        </button>

        {/* Backup server */}
        <button
          onClick={() => choose("backup")}
          className="w-full p-5 text-left rounded-xl border-2 border-gray-200 dark:border-gray-600 hover:border-green-400 dark:hover:border-green-500 transition-colors group"
        >
          <div className="flex items-center gap-4">
            <div className="w-12 h-12 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center shrink-0">
              <svg
                className="w-6 h-6 text-green-600 dark:text-green-400"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={1.5}
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125"
                />
              </svg>
            </div>
            <div>
              <p className="font-semibold text-gray-900 dark:text-white text-base group-hover:text-green-600 dark:group-hover:text-green-400 transition-colors">
                Backup Server
              </p>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-0.5">
                This server will mirror an existing Simple Photos instance.
                You'll pair with the primary server and log in with its admin
                credentials.
              </p>
            </div>
          </div>
        </button>
      </div>

      <div className="mt-8 pt-6 border-t border-gray-200 dark:border-gray-700">
        <button
          onClick={() => {
            setStep("welcome");
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
