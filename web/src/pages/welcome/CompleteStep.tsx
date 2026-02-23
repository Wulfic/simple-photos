import { useNavigate } from "react-router-dom";
import { api } from "../../api/client";
import type { WizardStep, CreatedUser } from "./types";

export interface CompleteStepProps {
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  loading: boolean;
  setLoading: (v: boolean) => void;
  error: string;
  encryptionMode: "plain" | "encrypted";
  createdUsers: CreatedUser[];
  serverPort: number;
  originalPort: number;
}

export default function CompleteStep({
  setError,
  loading,
  setLoading,
  encryptionMode,
  createdUsers,
  serverPort,
  originalPort,
}: CompleteStepProps) {
  const navigate = useNavigate();

  return (
    <div className="text-center">
      <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mx-auto mb-4" />
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-2">
        You're All Set!
      </h2>
      <p className="text-gray-600 dark:text-gray-400 mb-6">
        Your Simple Photos server is ready. Start uploading your
        encrypted photos and videos.
      </p>

      <div className="bg-green-50 dark:bg-green-900/30 rounded-lg p-4 mb-6 text-sm text-left space-y-2">
        <div className="flex items-center gap-2">
          <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
          <span className="text-gray-700 dark:text-gray-300">Admin account created</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
          <span className="text-gray-700 dark:text-gray-300">
            Encryption key derived
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
          <span className="text-gray-700 dark:text-gray-300">
            Storage: {encryptionMode === "encrypted" ? "All photos encrypted" : "Standard (unencrypted)"}
          </span>
        </div>
        {createdUsers.length > 0 && (
          <div className="flex items-center gap-2">
            <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
            <span className="text-gray-700 dark:text-gray-300">
              {createdUsers.length} additional user
              {createdUsers.length > 1 ? "s" : ""} created
            </span>
          </div>
        )}
        {serverPort !== originalPort && (
          <div className="flex items-center gap-2">
            <span className="text-amber-500">{"\u21BB"}</span>
            <span className="text-gray-700 dark:text-gray-300">
              Port changed to {serverPort} — restart pending
            </span>
          </div>
        )}
        <div className="flex items-center gap-2">
          <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
          <span className="text-gray-700 dark:text-gray-300">Ready to upload</span>
        </div>
      </div>

      {serverPort !== originalPort && (
        <div className="bg-amber-50 dark:bg-amber-900/30 border border-amber-200 dark:border-amber-800 rounded-lg p-3 mb-4 text-xs text-amber-800 dark:text-amber-300 text-left">
          <strong>Port changed:</strong> The server will restart on port{" "}
          <span className="font-mono font-bold">{serverPort}</span>.
          You'll be redirected automatically after the restart.
        </div>
      )}

      <div className="space-y-3">
        <button
          onClick={async () => {
            if (serverPort !== originalPort) {
              // Port changed — trigger restart and redirect to new port
              setLoading(true);
              setError("");
              try {
                await api.admin.restart();
              } catch {
                // Expected: server may drop connection during shutdown
              }
              // Build the new URL with the updated port
              const newUrl = `${window.location.protocol}//${window.location.hostname}:${serverPort}/gallery`;
              // Poll the new port until the server is back up
              const maxAttempts = 30;
              for (let i = 0; i < maxAttempts; i++) {
                await new Promise((r) => setTimeout(r, 2000));
                try {
                  const res = await fetch(
                    `${window.location.protocol}//${window.location.hostname}:${serverPort}/health`,
                    { mode: "no-cors" }
                  );
                  // no-cors gives opaque response (status 0), but if fetch succeeds the server is up
                  if (res.ok || res.type === "opaque") {
                    window.location.href = newUrl;
                    return;
                  }
                } catch {
                  // Server not ready yet, keep polling
                }
              }
              // Fallback: redirect anyway and let the user refresh
              window.location.href = newUrl;
            } else {
              navigate("/gallery");
            }
          }}
          disabled={loading}
          className="w-full bg-blue-600 text-white py-3 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-lg font-medium transition-colors"
        >
          {loading
            ? serverPort !== originalPort
              ? "Restarting server\u2026"
              : "Loading\u2026"
            : serverPort !== originalPort
              ? "Restart & Go to Gallery →"
              : "Go to Gallery →"}
        </button>
        {loading && serverPort !== originalPort && (
          <p className="text-gray-500 dark:text-gray-400 text-xs animate-pulse">
            Waiting for server to restart on port {serverPort}\u2026
          </p>
        )}
        <p className="text-gray-400 text-xs">
          You can manage users, 2FA, and storage in Settings.
        </p>
      </div>
    </div>
  );
}
