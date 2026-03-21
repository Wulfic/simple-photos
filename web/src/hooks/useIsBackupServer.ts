import { useEffect } from "react";
import { useBackupStore } from "../store/backup";
import { api } from "../api/client";

/**
 * Returns `true` when the current server instance is running in backup mode.
 *
 * The value is fetched once from `GET /api/backup/mode` on first call and
 * cached in the Zustand backup store so every component reads the same
 * value without duplicate network requests.
 *
 * Components use this to hide mutating UI (upload, edit, delete, album
 * creation, etc.) that should not be available on a read-only backup mirror.
 */
export function useIsBackupServer(): boolean {
  const isBackupServer = useBackupStore((s) => s.isBackupServer);
  const isLoaded = useBackupStore((s) => s.isBackupServerLoaded);
  const setIsBackupServer = useBackupStore((s) => s.setIsBackupServer);
  const setIsBackupServerLoaded = useBackupStore((s) => s.setIsBackupServerLoaded);

  useEffect(() => {
    if (isLoaded) return; // Already fetched — don't re-fetch
    let cancelled = false;

    (async () => {
      try {
        const mode = await api.backup.getMode();
        if (!cancelled) {
          setIsBackupServer(mode.mode === "backup");
          setIsBackupServerLoaded(true);
        }
      } catch {
        // Endpoint unavailable or not admin — default to primary (false)
        if (!cancelled) {
          setIsBackupServerLoaded(true);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [isLoaded, setIsBackupServer, setIsBackupServerLoaded]);

  return isBackupServer;
}
