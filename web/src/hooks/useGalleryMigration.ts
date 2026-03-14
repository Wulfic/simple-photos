/**
 * Hook for client-side encryption migration — encrypts plain photos to
 * AES-256-GCM blobs, uploads them, and marks the server record as encrypted.
 *
 * Prefers server-parallel migration when available, falls back to a dedicated
 * Web Worker for large libraries. Reports progress via the activity store.
 */
import { useRef, useEffect } from "react";
import { api } from "../api/client";
import { hasCryptoKey } from "../crypto/crypto";
import { useAuthStore } from "../store/auth";
import { useProcessingStore } from "../store/processing";
import type { PlainPhoto } from "../utils/gallery";
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
 * Handles encryption migration — server-side parallel with Web Worker fallback.
 *
 * When the server reports an active "encrypting" migration, we first try the
 * server-side parallel migration endpoint which encrypts all photos using
 * multiple CPU cores without any network round-trips. If the server endpoint
 * isn't available (older server), we fall back to the Web Worker.
 *
 * Progress is tracked via a polling fallback that queries the server's
 * authoritative migration state every 2 seconds. This is more reliable than
 * SSE alone because if the SSE stream fails, the banner would otherwise
 * stay stuck at 0/N forever.
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
  const migrationWorkerRef = useRef<Worker | null>(null);
  const migrationAbortRef = useRef<AbortController | null>(null);
  const migrationPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  /** Poll the server for migration progress (reliable fallback). */
  function startMigrationPolling() {
    // Clear any existing poller
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

  /** Fallback: run encryption migration in a Web Worker (sequential, one-at-a-time). */
  async function startWebWorkerMigration() {
    try {
      console.log("[Gallery Migration] Fetching photo list for worker...");
      const allPhotos: PlainPhoto[] = [];
      let cursor: string | undefined;
      do {
        const res = await api.photos.list({ after: cursor, limit: 200 });
        allPhotos.push(...res.photos);
        cursor = res.next_cursor ?? undefined;
      } while (cursor);

      if (allPhotos.length === 0) {
        console.log("[Gallery Migration] No photos to encrypt, marking done");
        await api.encryption.reportProgress({ completed_count: 0, done: true });
        setMigrationStatus("idle");
        migrationRunningRef.current = false;
        endTask("encryption");
        return;
      }

      setMigrationTotal(allPhotos.length);
      console.log(`[Gallery Migration] Spawning worker for ${allPhotos.length} photos`);

      const { accessToken, refreshToken } = useAuthStore.getState();
      const keyHex = sessionStorage.getItem("sp_key");
      if (!accessToken || !keyHex) {
        throw new Error("Missing auth token or encryption key");
      }

      const worker = new Worker(
        new URL("../workers/migrationWorker.ts", import.meta.url),
        { type: "module" }
      );
      migrationWorkerRef.current = worker;

      worker.onmessage = async (e) => {
        const msg = e.data;
        if (msg.type === "progress") {
          setMigrationCompleted(msg.completed);
        } else if (msg.type === "done") {
          console.log(
            `[Gallery Migration] Worker done: ${msg.succeeded}/${msg.total} succeeded, ${msg.failed} failed`
          );
          setMigrationStatus("idle");
          migrationRunningRef.current = false;
          migrationWorkerRef.current = null;
          worker.terminate();
          endTask("encryption");
          await loadPlainPhotos();
          await loadEncryptedPhotos();
        } else if (msg.type === "error") {
          console.error("[Gallery Migration] Worker error:", msg.message);
          migrationRunningRef.current = false;
          migrationWorkerRef.current = null;
          worker.terminate();
          endTask("encryption");
        } else if (msg.type === "tokenUpdate") {
          useAuthStore.getState().setTokens(msg.accessToken, msg.refreshToken);
        }
      };

      worker.onerror = (err) => {
        console.error("[Gallery Migration] Worker fatal error:", err);
        migrationRunningRef.current = false;
        migrationWorkerRef.current = null;
        endTask("encryption");
      };

      worker.postMessage({
        type: "start",
        accessToken,
        refreshToken: refreshToken || "",
        keyHex,
        photos: allPhotos,
      });
    } catch (err: unknown) {
      console.error("[Gallery Migration] Setup error:", getErrorMessage(err));
      await api.encryption.reportProgress({
        completed_count: 0,
        error: `Migration failed: ${getErrorMessage(err)}`,
      }).catch(() => {});
      migrationRunningRef.current = false;
      endTask("encryption");
    }
  }

  // ── Trigger migration when status changes to "encrypting" ──────────────

  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;
    if (!hasCryptoKey()) return;

    migrationRunningRef.current = true;
    startTask("encryption");

    const keyHex = sessionStorage.getItem("sp_key");
    if (!keyHex) {
      console.error("[Gallery Migration] No encryption key in session");
      migrationRunningRef.current = false;
      endTask("encryption");
      return;
    }

    // Always start polling as a reliable progress mechanism
    startMigrationPolling();

    // Try server-side migration first (parallel, no network overhead per photo)
    (async () => {
      try {
        console.log("[Gallery Migration] Attempting server-side parallel migration...");
        const result = await api.encryption.startServerMigration(keyHex);
        console.log(`[Gallery Migration] Server accepted: ${result.message}`);
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

        // Also listen to SSE progress stream for faster updates (non-critical)
        try {
          const controller = await api.encryption.streamMigrationProgress(
            (data) => {
              setMigrationCompleted(data.completed);
              setMigrationTotal(data.total);
            },
            async () => {
              console.log("[Gallery Migration] SSE: migration complete");
              // Polling will handle the final state transition —
              // just stop the SSE stream cleanly.
              migrationAbortRef.current = null;
            },
            (err) => {
              // SSE failed — polling will handle progress from here.
              console.warn("[Gallery Migration] SSE stream error (polling continues):", err);
              migrationAbortRef.current = null;
            },
          );
          migrationAbortRef.current = controller;
        } catch {
          // SSE connection failed — polling handles progress
          console.warn("[Gallery Migration] SSE unavailable, relying on polling");
        }
      } catch (serverErr: unknown) {
        // Server-side migration not available — fall back to Web Worker
        console.warn(
          "[Gallery Migration] Server-side migration unavailable, falling back to Web Worker:",
          getErrorMessage(serverErr)
        );
        await startWebWorkerMigration();
      }
    })();

    return () => {
      // Cleanup on unmount
      if (migrationAbortRef.current) {
        migrationAbortRef.current.abort();
        migrationAbortRef.current = null;
      }
      if (migrationWorkerRef.current) {
        migrationWorkerRef.current.terminate();
        migrationWorkerRef.current = null;
      }
      if (migrationPollRef.current) {
        clearInterval(migrationPollRef.current);
        migrationPollRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- Only re-run when migrationStatus
    // changes. The other deps (startTask, endTask, setMigration*, load*) are stable refs or
    // setters that never change identity; listing them would add noise without benefit.
  }, [migrationStatus]);
}
