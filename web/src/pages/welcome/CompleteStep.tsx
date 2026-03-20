/** Wizard step — setup complete confirmation with redirect to login. */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../../api/client";
import type { WizardStep, CreatedUser, ServerRole, InstallType, RestoreSource } from "./types";

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
  serverRole?: ServerRole;
  mainServerUrl?: string;
  installType?: InstallType;
  restoreSource?: RestoreSource | null;
}

export default function CompleteStep({
  setError,
  loading,
  setLoading,
  encryptionMode,
  createdUsers,
  serverPort,
  originalPort,
  serverRole,
  mainServerUrl,
  installType,
  restoreSource,
}: CompleteStepProps) {
  const navigate = useNavigate();
  const [restoreStatus, setRestoreStatus] = useState("");
  const isRestore = serverRole !== "backup" && installType === "restore" && restoreSource;

  return (
    <div className="text-center">
      <img src="/logo.png" alt="Simple Photos" className="w-20 h-20 mx-auto mb-4" />
      <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-2">
        You're All Set!
      </h2>
      <p className="text-gray-600 dark:text-gray-400 mb-6">
        {serverRole === "backup"
          ? "This server is paired as a backup and ready to receive synced photos."
          : isRestore
            ? "Your server is ready. Photos will be restored from the backup server first, then normal setup will complete."
            : "Your Simple Photos server is ready. Start uploading your encrypted photos and videos."}
      </p>

      <div className="bg-green-50 dark:bg-green-900/30 rounded-lg p-4 mb-6 text-sm text-left space-y-2">
        <div className="flex items-center gap-2">
          <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
          <span className="text-gray-700 dark:text-gray-300">Admin account created</span>
        </div>
        {serverRole === "backup" && mainServerUrl && (
          <div className="flex items-center gap-2">
            <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
            <span className="text-gray-700 dark:text-gray-300">
              Paired with {mainServerUrl}
            </span>
          </div>
        )}
        {serverRole === "backup" && (
          <div className="flex items-center gap-2">
            <span className="text-green-600 dark:text-green-400">{"\u2713"}</span>
            <span className="text-gray-700 dark:text-gray-300">
              Backup mode enabled
            </span>
          </div>
        )}
        {isRestore && (
          <div className="flex items-center gap-2">
            <span className="text-amber-500">{"\u21BB"}</span>
            <span className="text-gray-700 dark:text-gray-300">
              Restore from {restoreSource.name} ({restoreSource.photo_count} photos)
            </span>
          </div>
        )}
        {serverRole !== "backup" && (
          <>
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
          </>
        )}
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

      {restoreStatus && (
        <div className="bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg p-3 mb-4 text-xs text-blue-800 dark:text-blue-300 text-left">
          {restoreStatus}
        </div>
      )}

      <div className="space-y-3">
        <button
          onClick={async () => {
            setLoading(true);
            setError("");

            try {
              // ── Restore from backup: sync photos FIRST ────────────────
              if (isRestore) {
                setRestoreStatus("Registering backup server for recovery\u2026");

                // Step 1: Register the backup server so we can trigger recovery
                const addResult = await api.backup.addServer({
                  name: restoreSource.name,
                  address: restoreSource.address,
                  api_key: restoreSource.api_key ?? undefined,
                  sync_frequency_hours: 24,
                });

                const serverId = addResult.id;

                // Step 2: Trigger recovery — downloads all photos from the backup
                setRestoreStatus(
                  `Syncing photos from ${restoreSource.name} (${restoreSource.photo_count} photos)\u2026 This runs in the background.`
                );

                await api.backup.recover(serverId).catch((err: unknown) => {
                  console.warn("Recovery trigger warning:", err);
                  // Non-fatal: recovery may still start in background
                });

                setRestoreStatus("Recovery started! Photos will sync in the background.");
              }

              // ── Normal setup tasks ────────────────────────────────────
              // Fire scan in the background — don't await it, as it can take
              // a long time and blocks navigation. Encryption mode is set
              // immediately after; the server applies it to any photos the scan
              // registers. Autoscan will catch anything the background scan misses.
              api.admin.scanAndRegister()
                .then((scanResult) => {
                  console.log("[Setup] Background scan complete:", scanResult);
                  api.admin.triggerConvert().catch(() => {});
                })
                .catch((scanErr) => {
                  console.warn("[Setup] Scan failed — autoscan will catch files later:", scanErr);
                });

              // Set the encryption mode on the server, including the encryption
              // key so the server can run migration autonomously (even if the
              // browser is closed immediately after this point).
              // Re-read sp_key immediately before sending to ensure it's available.
              const keyHex = encryptionMode === "encrypted"
                ? sessionStorage.getItem("sp_key") ?? undefined
                : undefined;
              if (encryptionMode === "encrypted" && !keyHex) {
                console.error("[Setup] Encryption key missing from sessionStorage! Key derivation may have failed.");
              }
              await api.encryption.setMode(encryptionMode, keyHex);
            } catch (err: unknown) {
              // Non-fatal: mode may already be set or endpoint unavailable
              console.warn("Setup finalization:", err);
            }

            setRestoreStatus("");

            // Clear wizard persistence — setup is complete
            try {
              sessionStorage.removeItem("sp_wizard_step");
              sessionStorage.removeItem("sp_wizard_active");
            } catch { /* ignore */ }

            // Navigate to /gallery — the Gallery page handles migration
            // progress and restore sync status.
            const destination = "/gallery";

            if (serverPort !== originalPort) {
              // Port changed — trigger restart and redirect to new port
              setLoading(true);
              setError("");
              try {
                await api.admin.restart();
              } catch {
                // Expected: server may drop connection during shutdown
              }
              const newUrl = `${window.location.protocol}//${window.location.hostname}:${serverPort}${destination}`;
              const maxAttempts = 30;
              for (let i = 0; i < maxAttempts; i++) {
                await new Promise((r) => setTimeout(r, 2000));
                try {
                  const res = await fetch(
                    `${window.location.protocol}//${window.location.hostname}:${serverPort}/health`,
                    { mode: "no-cors" }
                  );
                  if (res.ok || res.type === "opaque") {
                    window.location.href = newUrl;
                    return;
                  }
                } catch {
                  // Server not ready yet, keep polling
                }
              }
              window.location.href = newUrl;
            } else {
              navigate(destination);
            }
          }}
          disabled={loading}
          className="w-full bg-blue-600 text-white py-3 rounded-lg hover:bg-blue-700 disabled:opacity-50 text-lg font-medium transition-colors"
        >
          {loading
            ? restoreStatus
              ? "Restoring\u2026"
              : serverPort !== originalPort
                ? "Restarting server\u2026"
                : "Loading\u2026"
            : isRestore
              ? "Restore & Go to Gallery \u2192"
              : serverPort !== originalPort
                ? "Restart & Go to Gallery \u2192"
                : "Go to Gallery \u2192"}
        </button>
        {loading && serverPort !== originalPort && !restoreStatus && (
          <p className="text-gray-500 dark:text-gray-400 text-xs animate-pulse">
            Waiting for server to restart on port {serverPort}\u2026
          </p>
        )}
        {loading && restoreStatus && (
          <p className="text-gray-500 dark:text-gray-400 text-xs animate-pulse">
            {restoreStatus}
          </p>
        )}
        <p className="text-gray-400 text-xs">
          You can manage users, 2FA, and storage in Settings.
        </p>
      </div>
    </div>
  );
}
