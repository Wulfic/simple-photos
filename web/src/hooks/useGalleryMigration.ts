/**
 * Hook for monitoring server-side encryption migration progress.
 *
 * All encryption work is now done server-side — the server reads plain photos
 * from disk, encrypts them, writes encrypted blobs, and updates the DB.
 * This hook simply polls the server for progress and updates the UI.
 *
 * If the server reports an active "encrypting" migration but hasn't started
 * the actual work yet (e.g. the key was provided during setup but the scan
 * hadn't found files yet), this hook sends the key to kick-start migration.
 */
import { useRef, useEffect } from "react";
import { api } from "../api/client";
import { hasCryptoKey } from "../crypto/crypto";
import { useProcessingStore } from "../store/processing";
import { getErrorMessage } from "../utils/formatters";

// ── Types ─────────────────────────────────────────────────────────────────────

export interface MigrationDeps {
  migrationStatus: string;
  setMigrationStatus: (s: string) => void;
  setMigrationTotal: (n: number) => void;
  setMigrationCompleted: (n: number) => void;
  loadEncryptedPhotos: () => Promise<void>;
  loadPlainPhotos: () => Promise<void>;
}

// ── Hook ──────────────────────────────────────────────────────────────────────

/**
 * Monitors server-side encryption migration and updates UI with progress.
 *
 * Migration runs entirely on the server. This hook:
 * 1. Polls every 2 seconds for authoritative migration state
 * 2. Optionally kicks off server-side migration if the key is available
 *    and migration hasn't started yet
 * 3. Opens an SSE stream for faster real-time progress updates
 * 4. Reloads gallery data when migration completes
 */
export function useGalleryMigration({
  migrationStatus,
  setMigrationStatus,
  setMigrationTotal,
  setMigrationCompleted,
  loadEncryptedPhotos,
  loadPlainPhotos,
}: MigrationDeps) {
  const { startTask, endTask } = useProcessingStore();
  const migrationRunningRef = useRef(false);
  const migrationAbortRef = useRef<AbortController | null>(null);
  const migrationPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  /** Poll the server for migration progress. */
  function startMigrationPolling() {
    if (migrationPollRef.current) clearInterval(migrationPollRef.current);
    migrationPollRef.current = setInterval(async () => {
      try {
        const settings = await api.encryption.getSettings();
        setMigrationCompleted(settings.migration_completed);
        setMigrationTotal(settings.migration_total);

        if (settings.migration_status !== "encrypting") {
          // Migration finished on the server — clean up
          console.log("[Gallery Migration] Polling detected migration complete");
          if (migrationPollRef.current) {
            clearInterval(migrationPollRef.current);
            migrationPollRef.current = null;
          }
          setMigrationStatus("idle");
          migrationRunningRef.current = false;
          endTask("encryption");
          // Reload the gallery since photos are now encrypted
          if (settings.encryption_mode === "encrypted") {
            await loadPlainPhotos();
            await loadEncryptedPhotos();
          }
        }
      } catch {
        // Network error during poll — keep trying
      }
    }, 2000);
  }

  // ── Monitor migration when status is "encrypting" ─────────────────────

  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;

    migrationRunningRef.current = true;
    startTask("encryption");

    // Start polling for reliable progress tracking
    startMigrationPolling();

    // If we have the encryption key, ensure the server-side migration is running.
    // This handles the case where the key was sent during setup but the server
    // hadn't found any files yet (scan was still in progress). Now that the
    // Gallery has loaded and migration_status is "encrypting", we send the key
    // again to make sure the server has it and can proceed.
    (async () => {
      if (!hasCryptoKey()) {
        console.log("[Gallery Migration] No encryption key available — server should already have it stored");
        return;
      }

      const keyHex = sessionStorage.getItem("sp_key");
      if (!keyHex) return;

      try {
        console.log("[Gallery Migration] Ensuring server-side migration is running...");
        const result = await api.encryption.startServerMigration(keyHex);
        console.log(`[Gallery Migration] Server response: ${result.message}`);
        setMigrationTotal(result.total);

        if (result.total === 0) {
          if (migrationPollRef.current) {
            clearInterval(migrationPollRef.current);
            migrationPollRef.current = null;
          }
          setMigrationStatus("idle");
          migrationRunningRef.current = false;
          endTask("encryption");
          return;
        }
      } catch (err: unknown) {
        // Server may already be running the migration (409 Conflict) or
        // migration was already started during setMode — that's fine,
        // polling will track progress.
        console.log(
          "[Gallery Migration] Server-side migration already running or started:",
          getErrorMessage(err)
        );
      }

      // Open SSE stream for faster progress updates (non-critical, polling is primary)
      try {
        const controller = await api.encryption.streamMigrationProgress(
          (data) => {
            setMigrationCompleted(data.completed);
            setMigrationTotal(data.total);
          },
          async () => {
            console.log("[Gallery Migration] SSE: stream ended");
            migrationAbortRef.current = null;
          },
          (err) => {
            console.warn("[Gallery Migration] SSE stream error (polling continues):", err);
            migrationAbortRef.current = null;
          },
        );
        migrationAbortRef.current = controller;
      } catch {
        console.warn("[Gallery Migration] SSE unavailable, relying on polling");
      }
    })();

    return () => {
      // Cleanup on unmount — note: this does NOT stop the server-side migration,
      // which continues independently. Only UI monitoring is stopped.
      if (migrationAbortRef.current) {
        migrationAbortRef.current.abort();
        migrationAbortRef.current = null;
      }
      if (migrationPollRef.current) {
        clearInterval(migrationPollRef.current);
        migrationPollRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [migrationStatus]);
}
