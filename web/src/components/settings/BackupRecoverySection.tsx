/**
 * Backup Recovery / Primary Server section for the Settings page.
 *
 * Handles: backup server listing, discovery/scan, manual add form,
 * recovery from backup, and force-sync for backup-mode servers.
 */
import { useState, useCallback } from "react";
import { api } from "../../api/client";
import { useBackupStore } from "../../store/backup";
import { useProcessingStore } from "../../store/processing";
import AppIcon from "../AppIcon";
import { getErrorMessage } from "../../utils/formatters";

interface BackupRecoverySectionProps {
  isBackupMode: boolean;
  primaryServerUrl: string | null;
  setError: (v: string) => void;
  setSuccess: (v: string) => void;
  loadBackupServers: () => Promise<void>;
}

export default function BackupRecoverySection({
  isBackupMode,
  primaryServerUrl,
  setError,
  setSuccess,
  loadBackupServers,
}: BackupRecoverySectionProps) {
  const { backupServers, loaded: backupLoaded, recovering, setRecovering } = useBackupStore();
  const { startTask, endTask } = useProcessingStore();

  // ── Recovery state ─────────────────────────────────────────────────────
  const [showRecoverWarning, setShowRecoverWarning] = useState(false);

  // ── Manual backup server form state ────────────────────────────────────
  const [showAddBackupServer, setShowAddBackupServer] = useState(false);
  const [backupServerName, setBackupServerName] = useState("");
  const [backupServerAddress, setBackupServerAddress] = useState("");
  const [backupServerApiKey, setBackupServerApiKey] = useState("");
  const [backupServerFrequency, setBackupServerFrequency] = useState("24");
  const [addingBackupServer, setAddingBackupServer] = useState(false);

  // ── Discovered servers (scan results) ──────────────────────────────────
  type DiscoveredEntry = { address: string; name: string; version: string; api_key?: string };
  const [discoveredServers, setDiscoveredServers] = useState<DiscoveredEntry[]>([]);
  const [discovering, setDiscovering] = useState(false);

  // ── Force sync (backup-mode only) ──────────────────────────────────────
  const [forceSyncing, setForceSyncing] = useState(false);

  const scanForBackupServers = useCallback(async () => {
    setDiscovering(true);
    setDiscoveredServers([]);
    try {
      const disc = await api.backup.discover();
      const registeredAddrs = new Set(backupServers.map((s) => s.address));
      const fresh = disc.servers.filter((s) => !registeredAddrs.has(s.address));
      setDiscoveredServers(fresh);
    } catch {
      // Discovery not available or network unreachable — ignore
    } finally {
      setDiscovering(false);
    }
  }, [backupServers]);

  async function handleRecover() {
    if (backupServers.length === 0) return;
    setShowRecoverWarning(false);
    setRecovering(true);
    startTask("recovery");
    setError("");
    try {
      const target = backupServers.find((s) => s.enabled) ?? backupServers[0];
      const res = await api.backup.recover(target.id);
      setSuccess(res.message);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setRecovering(false);
      endTask("recovery");
    }
  }

  async function handleAddBackupServer(e: React.FormEvent) {
    e.preventDefault();
    if (!backupServerName.trim() || !backupServerAddress.trim() || !backupServerApiKey.trim()) {
      setError("All backup server fields are required.");
      return;
    }
    const freq = parseInt(backupServerFrequency, 10);
    if (isNaN(freq) || freq < 1) {
      setError("Frequency must be a positive number of hours.");
      return;
    }
    setAddingBackupServer(true);
    setError("");
    try {
      await api.backup.addServer({
        name: backupServerName.trim(),
        address: backupServerAddress.trim(),
        api_key: backupServerApiKey.trim(),
        sync_frequency_hours: freq,
      });
      setSuccess("Backup server added successfully.");
      setShowAddBackupServer(false);
      setDiscoveredServers((prev) => prev.filter((s) => s.address !== backupServerAddress.trim()));
      setBackupServerName("");
      setBackupServerAddress("");
      setBackupServerApiKey("");
      setBackupServerFrequency("24");
      await loadBackupServers();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to add backup server."));
    } finally {
      setAddingBackupServer(false);
    }
  }

  // ── Backup-mode view: show paired primary server ───────────────────────
  if (isBackupMode) {
    return (
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Primary Server</h2>
        <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
          This server is running in <strong>backup mode</strong>. All photos, accounts, and settings are mirrored from the primary server.
        </p>
        <div className="flex items-center gap-3 p-4 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg">
          <div className="w-10 h-10 rounded-full bg-green-100 dark:bg-green-900/40 flex items-center justify-center flex-shrink-0">
            <svg className="w-5 h-5 text-green-600 dark:text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
          <div className="min-w-0">
            <p className="text-sm font-medium text-green-800 dark:text-green-300">
              Connected to Primary Server
            </p>
            <p className="text-xs text-green-600 dark:text-green-400 truncate font-mono">
              {primaryServerUrl ?? "Unknown address"}
            </p>
          </div>
        </div>
        <p className="text-xs text-gray-400 dark:text-gray-500 mt-3">
          Changes to photos, users, passwords, and 2FA should be made on the primary server. They will be synced automatically.
        </p>
        <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
          <button
            onClick={async () => {
              setForceSyncing(true);
              setError("");
              setSuccess("");
              try {
                const res = await api.backup.forceSyncFromPrimary();
                setSuccess(res.message || "Sync requested — the primary server will push updates shortly.");
              } catch (err: unknown) {
                setError(getErrorMessage(err, "Failed to request sync from primary server."));
              } finally {
                setForceSyncing(false);
              }
            }}
            disabled={forceSyncing}
            className="inline-flex items-center gap-2 bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-500 text-sm font-medium transition-colors disabled:opacity-50"
          >
            <AppIcon name="reload" size="w-4 h-4" className={forceSyncing ? "animate-spin" : ""} />
            {forceSyncing ? "Requesting Sync…" : "Force Sync from Primary"}
          </button>
          <p className="text-xs text-gray-400 dark:text-gray-500 mt-2">
            Request the primary server to immediately push all new photos and data to this backup.
          </p>
        </div>
      </section>
    );
  }

  // ── Primary-mode view: recovery + add/scan ─────────────────────────────
  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">Backup Recovery</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Recover photos from a configured backup server. Any photos on the backup
        that don't already exist on this server (by filename) will be downloaded and imported.
      </p>

      {!backupLoaded ? (
        <div className="text-gray-400 text-sm">Loading backup servers…</div>
      ) : backupServers.length === 0 ? (
        <div className="space-y-3">
          <div className="text-center py-4 border-2 border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
            <p className="text-gray-400 text-sm">No backup servers configured.</p>
          </div>

        </div>
      ) : !showRecoverWarning ? (
        <button
          onClick={() => {
            setShowRecoverWarning(true);
            setError("");
            setSuccess("");
          }}
          disabled={recovering}
          className="bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 disabled:opacity-50 text-sm"
        >
          {recovering ? (
            <span className="flex items-center gap-2">
              <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
              Recovering…
            </span>
          ) : (
            "Recover from Backup Server"
          )}
        </button>
      ) : (
        <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-4">
          <h4 className="text-sm font-semibold text-amber-800 dark:text-amber-300 mb-2">
            ⚠️ Confirm Recovery
          </h4>
          <p className="text-sm text-amber-700 dark:text-amber-400 mb-1">
            This will download <strong>all photos</strong> from the backup server
            {" "}<strong>"{backupServers.find((s) => s.enabled)?.name ?? backupServers[0]?.name}"</strong> to
            this server.
          </p>
          <ul className="text-sm text-amber-700 dark:text-amber-400 list-disc list-inside mb-3 space-y-0.5">
            <li>Photos with the same filename will be <strong>skipped</strong> (not overwritten).</li>
            <li>This process runs in the background and may take a while for large libraries.</li>
            <li>The backup server must be reachable and have its API key configured.</li>
          </ul>
          <div className="flex gap-2">
            <button
              onClick={handleRecover}
              disabled={recovering}
              className="bg-amber-600 text-white px-4 py-2 rounded-md hover:bg-amber-700 disabled:opacity-50 text-sm"
            >
              {recovering ? "Starting…" : "Confirm Recovery"}
            </button>
            <button
              onClick={() => setShowRecoverWarning(false)}
              className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* ── Add / Scan ─────────────────────────────────────────── */}
      <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
        {!showAddBackupServer ? (
          <div className="space-y-4">
            <div className="flex items-center gap-4 flex-wrap">
              <button
                onClick={scanForBackupServers}
                disabled={discovering}
                className="text-sm text-blue-600 dark:text-blue-400 hover:underline disabled:opacity-50"
              >
                {discovering ? "Scanning…" : "Scan network for backup servers"}
              </button>
              <button
                onClick={() => {
                  setBackupServerName("");
                  setBackupServerAddress("");
                  setBackupServerApiKey("");
                  setShowAddBackupServer(true);
                }}
                className="text-sm text-gray-600 dark:text-gray-400 hover:underline"
              >
                + Add backup server manually
              </button>
            </div>

            {/* Discovered servers — shown as suggestions, user adds manually */}
            {discoveredServers.length > 0 && (
              <div className="space-y-2 mt-4">
                <p className="text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wide">
                  Found on local network
                </p>
                {discoveredServers.map((srv) => (
                  <div
                    key={srv.address}
                    className="flex items-center justify-between gap-3 p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg border border-gray-200 dark:border-gray-600"
                  >
                    <div className="min-w-0">
                      <p className="text-sm font-medium text-gray-800 dark:text-gray-200 truncate">
                        {srv.name || "Simple Photos"}
                      </p>
                      <p className="text-xs text-gray-500 dark:text-gray-400 truncate">
                        {srv.address} &nbsp;·&nbsp; v{srv.version}
                      </p>
                    </div>
                    <button
                      onClick={() => {
                        setBackupServerName(srv.name || `Backup (${srv.address})`);
                        setBackupServerAddress(srv.address);
                        setBackupServerApiKey(srv.api_key ?? "");
                        setShowAddBackupServer(true);
                      }}
                      className="flex-shrink-0 text-xs bg-blue-600 text-white px-3 py-1.5 rounded hover:bg-blue-700"
                    >
                      Add
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : (
          <form onSubmit={handleAddBackupServer} className="space-y-3">
            <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-300">Add Backup Server</h4>
            <div>
              <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Name</label>
              <input
                type="text"
                value={backupServerName}
                onChange={(e) => setBackupServerName(e.target.value)}
                placeholder="My Backup Server"
                maxLength={200}
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Server Address</label>
              <input
                type="text"
                value={backupServerAddress}
                onChange={(e) => setBackupServerAddress(e.target.value)}
                placeholder="https://backup.example.com:8443"
                maxLength={500}
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">API Key</label>
              <input
                type="password"
                value={backupServerApiKey}
                onChange={(e) => setBackupServerApiKey(e.target.value)}
                placeholder="Backup server API key"
                maxLength={256}
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">Backup Frequency (hours)</label>
              <input
                type="number"
                min={1}
                value={backupServerFrequency}
                onChange={(e) => setBackupServerFrequency(e.target.value)}
                className="w-28 border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:bg-gray-700 dark:border-gray-600"
              />
            </div>
            <div className="flex gap-2">
              <button
                type="submit"
                disabled={addingBackupServer}
                className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
              >
                {addingBackupServer ? (
                  <span className="flex items-center gap-2">
                    <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                    Adding…
                  </span>
                ) : (
                  "Add Server"
                )}
              </button>
              <button
                type="button"
                onClick={() => setShowAddBackupServer(false)}
                className="bg-gray-200 dark:bg-gray-600 text-gray-800 dark:text-gray-200 px-4 py-2 rounded-md hover:bg-gray-300 dark:hover:bg-gray-500 text-sm"
              >
                Cancel
              </button>
            </div>
          </form>
        )}
      </div>
    </section>
  );
}
