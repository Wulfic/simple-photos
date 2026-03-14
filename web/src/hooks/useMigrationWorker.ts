/**
 * Hook that spawns a dedicated Web Worker for encryption migration.
 *
 * The worker runs in a separate thread to avoid blocking the UI while
 * re-encrypting photos. Used as a fallback when server-parallel migration
 * is unavailable. Communicates progress back via `postMessage`.
 */
import { useRef, useEffect } from "react";
import { api } from "../api/client";
import { hasCryptoKey } from "../crypto/crypto";
import { useAuthStore } from "../store/auth";
import { getErrorMessage } from "../utils/formatters";

/**
 * Hook that spawns a dedicated Web Worker for the encryption migration.
 *
 * Web Workers run on a separate thread that is NOT throttled when the
 * browser tab is backgrounded — so encryption continues at full speed
 * even if the user switches tabs or minimizes the browser.
 */
export function useMigrationWorker(
  migrationStatus: string,
  loadEncryptionSettings: () => Promise<void>,
) {
  const migrationRunningRef = useRef(false);
  const workerRef = useRef<Worker | null>(null);

  useEffect(() => {
    if (migrationStatus !== "encrypting") return;
    if (migrationRunningRef.current) return;
    if (!hasCryptoKey()) return;

    migrationRunningRef.current = true;

    (async () => {
      try {
        console.log("[Migration Hook] Fetching photo list for worker...");

        // Fetch ALL plain photos that need encrypting
        const allPhotos: Array<{
          id: string; filename: string; file_path: string; mime_type: string;
          media_type: string; size_bytes: number; width: number; height: number;
          duration_secs: number | null; taken_at: string | null;
          latitude: number | null; longitude: number | null;
          thumb_path: string | null; created_at: string;
        }> = [];
        let cursor: string | undefined;
        do {
          const res = await api.photos.list({ after: cursor, limit: 200 });
          allPhotos.push(...res.photos);
          cursor = res.next_cursor ?? undefined;
        } while (cursor);

        if (allPhotos.length === 0) {
          console.log("[Migration Hook] No photos to encrypt");
          migrationRunningRef.current = false;
          await loadEncryptionSettings();
          return;
        }

        console.log(`[Migration Hook] Spawning worker for ${allPhotos.length} photos`);

        // Get auth tokens and encryption key for the worker
        const { accessToken, refreshToken } = useAuthStore.getState();
        const keyHex = sessionStorage.getItem("sp_key");
        if (!accessToken || !keyHex) {
          throw new Error("Missing auth token or encryption key");
        }

        // Spawn the migration Web Worker
        const worker = new Worker(
          new URL("../workers/migrationWorker.ts", import.meta.url),
          { type: "module" }
        );
        workerRef.current = worker;

        worker.onmessage = async (e) => {
          const msg = e.data;

          if (msg.type === "progress") {
            // Progress updates are handled by the Settings polling
          } else if (msg.type === "done") {
            console.log(
              `[Migration Hook] Worker done: ${msg.succeeded}/${msg.total} succeeded`
            );
            migrationRunningRef.current = false;
            workerRef.current = null;
            worker.terminate();
            await loadEncryptionSettings();
          } else if (msg.type === "error") {
            console.error("[Migration Hook] Worker error:", msg.message);
            migrationRunningRef.current = false;
            workerRef.current = null;
            worker.terminate();
          } else if (msg.type === "tokenUpdate") {
            // Worker refreshed the token — update main thread stores
            useAuthStore.getState().setTokens(msg.accessToken, msg.refreshToken);
          }
        };

        worker.onerror = (err) => {
          console.error("[Migration Hook] Worker fatal error:", err);
          migrationRunningRef.current = false;
          workerRef.current = null;
        };

        // Start the worker
        worker.postMessage({
          type: "start",
          accessToken,
          refreshToken: refreshToken || "",
          keyHex,
          photos: allPhotos,
        });
      } catch (err: unknown) {
        console.error("[Migration Hook] Setup error:", getErrorMessage(err));
        migrationRunningRef.current = false;
      }
    })();

    return () => {
      if (workerRef.current) {
        workerRef.current.terminate();
        workerRef.current = null;
      }
    };
  }, [migrationStatus, loadEncryptionSettings]);
}
