import { api } from "../../api/client";
import type { WizardStep } from "./types";

export interface EncryptionStepProps {
  encryptionMode: "plain" | "encrypted";
  setEncryptionMode: (mode: "plain" | "encrypted") => void;
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  loading: boolean;
  setLoading: (v: boolean) => void;
  error: string;
}

export default function EncryptionStep({
  encryptionMode,
  setEncryptionMode,
  setStep,
  setError,
  loading,
  setLoading,
  error,
}: EncryptionStepProps) {
  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-1">
        Photo Storage Mode
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        Choose how your photos are stored on the server.
      </p>

      <div className="space-y-4">
        {/* Encrypted mode — recommended, shown first */}
        <button
          onClick={() => setEncryptionMode("encrypted")}
          className={`w-full text-left p-4 rounded-lg border-2 transition-colors ${
            encryptionMode === "encrypted"
              ? "border-blue-500 bg-blue-50 dark:bg-blue-900/20"
              : "border-gray-200 dark:border-gray-700 hover:border-gray-300 dark:hover:border-gray-600"
          }`}
        >
          <div className="flex items-center gap-3 mb-2">
            <span className="inline-block w-5 h-5 min-w-[1.25rem] min-h-[1.25rem] rounded-full border-2 shrink-0" style={{ aspectRatio: '1/1', display: 'flex', alignItems: 'center', justifyContent: 'center', borderColor: encryptionMode === 'encrypted' ? '#3b82f6' : '#9ca3af' }}>
              {encryptionMode === "encrypted" && (
                <span className="inline-block w-2.5 h-2.5 min-w-[0.625rem] min-h-[0.625rem] rounded-full bg-blue-500" style={{ aspectRatio: '1/1' }} />
              )}
            </span>
            <h3 className="font-semibold text-gray-900 dark:text-gray-100">
              Encrypt All Photos
            </h3>
            <span className="text-xs bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-400 px-2 py-0.5 rounded-full">
              Recommended
            </span>
          </div>
          <p className="text-sm text-gray-600 dark:text-gray-400 ml-8">
            All photos are encrypted client-side before being stored as opaque blobs.
            Nobody — including the server administrator — can view them without your password.
            Photos cannot be browsed via the file system.
          </p>
        </button>

        {/* Plain mode */}
        <button
          onClick={() => setEncryptionMode("plain")}
          className={`w-full text-left p-4 rounded-lg border-2 transition-colors ${
            encryptionMode === "plain"
              ? "border-blue-500 bg-blue-50 dark:bg-blue-900/20"
              : "border-gray-200 dark:border-gray-700 hover:border-gray-300 dark:hover:border-gray-600"
          }`}
        >
          <div className="flex items-center gap-3 mb-2">
            <span className="inline-block w-5 h-5 min-w-[1.25rem] min-h-[1.25rem] rounded-full border-2 shrink-0" style={{ aspectRatio: '1/1', display: 'flex', alignItems: 'center', justifyContent: 'center', borderColor: encryptionMode === 'plain' ? '#3b82f6' : '#9ca3af' }}>
              {encryptionMode === "plain" && (
                <span className="inline-block w-2.5 h-2.5 min-w-[0.625rem] min-h-[0.625rem] rounded-full bg-blue-500" style={{ aspectRatio: '1/1' }} />
              )}
            </span>
            <h3 className="font-semibold text-gray-900 dark:text-gray-100">
              Standard Storage
            </h3>
          </div>
          <p className="text-sm text-gray-600 dark:text-gray-400 ml-8">
            Photos are stored as regular files on disk. They can be browsed via the file system
            and are automatically imported when placed in the storage folder.
          </p>
          {encryptionMode === "plain" && (
            <div className="mt-2 ml-8 flex items-start gap-2 text-xs text-amber-700 dark:text-amber-400 bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-2">
              <span className="text-base leading-none">⚠️</span>
              <span>Photos will <strong>not</strong> be encrypted. Anyone with access to the server's file system can view them. Only choose this if you trust your hosting environment.</span>
            </div>
          )}
        </button>
      </div>

      <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg p-3 mt-4 text-xs text-blue-800 dark:text-blue-300">
        <strong>Tip:</strong> Regardless of this choice, you can always create individual
        <strong> Encrypted Galleries</strong> that are password-protected. This setting only
        controls the default storage for your main photo library.
        You can change this later in Settings.
      </div>

      {error && (
        <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg mt-4">
          {error}
        </div>
      )}

      <div className="flex gap-3 mt-6">
        <button
          onClick={() => {
            setError("");
            setStep("ssl");
          }}
          className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors"
        >
          ← Back
        </button>
        <button
          onClick={() => {
            setError("");
            // Don't set encryption mode on server yet — defer to CompleteStep
            // so we can scan for existing files in plain mode first,
            // then switch to encrypted (triggering migration).
            setStep("users");
          }}
          disabled={loading}
          className="flex-[2] bg-blue-600 text-white py-2.5 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-sm font-medium transition-colors"
        >
          Continue →
        </button>
      </div>
    </div>
  );
}
