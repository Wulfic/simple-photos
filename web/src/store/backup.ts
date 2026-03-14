/**
 * Zustand store for backup server UI state.
 *
 * Tracks the active backup server, view mode (main vs. backup gallery),
 * recovery status, and the loaded server list. Pure UI state — API calls
 * are made in page components, not here.
 */
import { create } from "zustand";

export type ViewMode = "main" | "backup";

interface BackupServer {
  id: string;
  name: string;
  address: string;
  enabled: boolean;
}

interface BackupViewState {
  /** Current view mode: "main" (local server) or "backup" (backup server) */
  viewMode: ViewMode;
  /** List of configured backup servers */
  backupServers: BackupServer[];
  /** The currently selected backup server ID (for backup view) */
  activeBackupServerId: string | null;
  /** Whether backup servers have been loaded */
  loaded: boolean;
  /** Whether a recovery is currently running */
  recovering: boolean;

  setViewMode: (mode: ViewMode) => void;
  toggleViewMode: () => void;
  setBackupServers: (servers: BackupServer[]) => void;
  setActiveBackupServerId: (id: string | null) => void;
  setLoaded: (loaded: boolean) => void;
  setRecovering: (recovering: boolean) => void;

  /** Whether a backup server is available (at least one configured) */
  hasBackupServer: () => boolean;
}

export const useBackupStore = create<BackupViewState>((set, get) => ({
  viewMode: "main",
  backupServers: [],
  activeBackupServerId: null,
  loaded: false,
  recovering: false,

  setViewMode: (mode) => set({ viewMode: mode }),

  toggleViewMode: () =>
    set((s) => ({
      viewMode: s.viewMode === "main" ? "backup" : "main",
    })),

  setBackupServers: (servers) =>
    set({
      backupServers: servers,
      // Auto-select the first enabled server if none selected
      activeBackupServerId:
        get().activeBackupServerId ??
        servers.find((s) => s.enabled)?.id ??
        servers[0]?.id ??
        null,
    }),

  setActiveBackupServerId: (id) => set({ activeBackupServerId: id }),
  setLoaded: (loaded) => set({ loaded }),
  setRecovering: (recovering) => set({ recovering }),

  hasBackupServer: () => get().backupServers.length > 0,
}));
