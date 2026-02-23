import { useState } from "react";
import { api } from "../../api/client";
import type { WizardStep } from "./types";

interface DiscoveredServer {
  address: string;
  name: string;
  version: string;
}

interface BackupServer {
  id: string;
  name: string;
  address: string;
  sync_frequency_hours: number;
}

export interface BackupStepProps {
  setStep: (step: WizardStep) => void;
  setError: (error: string) => void;
  error: string;
}

export default function BackupStep({ setStep, setError, error }: BackupStepProps) {
  const [mode, setMode] = useState<"choice" | "discover" | "manual">("choice");
  const [discovering, setDiscovering] = useState(false);
  const [discovered, setDiscovered] = useState<DiscoveredServer[]>([]);
  const [addedServers, setAddedServers] = useState<BackupServer[]>([]);
  const [loading, setLoading] = useState(false);

  // Manual form
  const [manualName, setManualName] = useState("");
  const [manualAddress, setManualAddress] = useState("");
  const [manualApiKey, setManualApiKey] = useState("");
  const [manualFrequency, setManualFrequency] = useState("24");

  async function handleDiscover() {
    setDiscovering(true);
    setError("");
    try {
      const resp = await api.backup.discover();
      setDiscovered(resp.servers);
      setMode("discover");
    } catch (e: any) {
      setError(e.message || "Network discovery failed");
    } finally {
      setDiscovering(false);
    }
  }

  async function handleAddDiscovered(server: DiscoveredServer) {
    setLoading(true);
    setError("");
    try {
      const resp = await api.backup.addServer({
        name: server.name,
        address: server.address,
        sync_frequency_hours: 24,
      });
      setAddedServers((prev) => [
        ...prev,
        {
          id: resp.id,
          name: resp.name,
          address: resp.address,
          sync_frequency_hours: resp.sync_frequency_hours,
        },
      ]);
      setDiscovered((prev) => prev.filter((s) => s.address !== server.address));
    } catch (e: any) {
      setError(e.message || "Failed to add backup server");
    } finally {
      setLoading(false);
    }
  }

  async function handleManualAdd(e: React.FormEvent) {
    e.preventDefault();
    if (!manualName.trim() || !manualAddress.trim()) {
      setError("Name and address are required");
      return;
    }
    setLoading(true);
    setError("");
    try {
      const freq = parseInt(manualFrequency, 10) || 24;
      const resp = await api.backup.addServer({
        name: manualName.trim(),
        address: manualAddress.trim(),
        api_key: manualApiKey.trim() || undefined,
        sync_frequency_hours: freq,
      });
      setAddedServers((prev) => [
        ...prev,
        {
          id: resp.id,
          name: resp.name,
          address: resp.address,
          sync_frequency_hours: resp.sync_frequency_hours,
        },
      ]);
      setManualName("");
      setManualAddress("");
      setManualApiKey("");
      setManualFrequency("24");
      setMode("choice");
    } catch (e: any) {
      setError(e.message || "Failed to add backup server");
    } finally {
      setLoading(false);
    }
  }

  async function handleRemoveServer(id: string) {
    try {
      await api.backup.removeServer(id);
      setAddedServers((prev) => prev.filter((s) => s.id !== id));
    } catch (e: any) {
      setError(e.message || "Failed to remove server");
    }
  }

  return (
    <>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">
        Backup Server
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        Set up a secondary Simple Photos instance to automatically mirror all your photos.
        Your backup server will keep an exact copy, including the trash.
      </p>

      {error && (
        <div className="mb-4 p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 rounded-lg text-sm">
          {error}
        </div>
      )}

      {/* Added servers */}
      {addedServers.length > 0 && (
        <div className="mb-6">
          <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
            Configured Backup Servers
          </h3>
          <div className="space-y-2">
            {addedServers.map((s) => (
              <div
                key={s.id}
                className="flex items-center justify-between p-3 bg-green-50 dark:bg-green-900/20 rounded-lg border border-green-200 dark:border-green-800"
              >
                <div>
                  <p className="font-medium text-gray-900 dark:text-white text-sm">
                    {s.name}
                  </p>
                  <p className="text-xs text-gray-500 dark:text-gray-400">
                    {s.address} · Every {s.sync_frequency_hours}h
                  </p>
                </div>
                <button
                  onClick={() => handleRemoveServer(s.id)}
                  className="text-red-500 hover:text-red-700 text-sm"
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Choice mode: discover or manual */}
      {mode === "choice" && (
        <div className="space-y-3">
          <button
            onClick={handleDiscover}
            disabled={discovering}
            className="w-full p-4 text-left rounded-lg border-2 border-gray-200 dark:border-gray-600 hover:border-blue-400 dark:hover:border-blue-500 transition-colors"
          >
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-full bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center">
                <svg className="w-5 h-5 text-blue-600 dark:text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M21 21l-5.197-5.197m0 0A7.5 7.5 0 105.196 5.196a7.5 7.5 0 0010.607 10.607z" />
                </svg>
              </div>
              <div>
                <p className="font-medium text-gray-900 dark:text-white">
                  {discovering ? "Scanning network..." : "Auto-Discover"}
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Scan your local network for Simple Photos servers
                </p>
              </div>
              {discovering && (
                <div className="ml-auto w-5 h-5 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
              )}
            </div>
          </button>

          <button
            onClick={() => { setMode("manual"); setError(""); }}
            className="w-full p-4 text-left rounded-lg border-2 border-gray-200 dark:border-gray-600 hover:border-blue-400 dark:hover:border-blue-500 transition-colors"
          >
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-full bg-purple-100 dark:bg-purple-900/30 flex items-center justify-center">
                <svg className="w-5 h-5 text-purple-600 dark:text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931zm0 0L19.5 7.125M18 14v4.75A2.25 2.25 0 0115.75 21H5.25A2.25 2.25 0 013 18.75V8.25A2.25 2.25 0 015.25 6H10" />
                </svg>
              </div>
              <div>
                <p className="font-medium text-gray-900 dark:text-white">
                  Enter Manually
                </p>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Enter the IP address or DNS name of your backup server
                </p>
              </div>
            </div>
          </button>
        </div>
      )}

      {/* Discovery results */}
      {mode === "discover" && (
        <div>
          {discovered.length === 0 ? (
            <div className="text-center py-8">
              <svg className="w-12 h-12 mx-auto text-gray-300 dark:text-gray-600 mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5.636 18.364a9 9 0 010-12.728m12.728 0a9 9 0 010 12.728m-9.9-2.829a5 5 0 010-7.07m7.072 0a5 5 0 010 7.07M13 12a1 1 0 11-2 0 1 1 0 012 0z" />
              </svg>
              <p className="text-gray-500 dark:text-gray-400 text-sm">
                No Simple Photos servers found on your network.
              </p>
              <p className="text-gray-400 dark:text-gray-500 text-xs mt-1">
                Make sure the backup server is running and on the same network.
              </p>
            </div>
          ) : (
            <div className="space-y-2">
              <p className="text-sm text-gray-600 dark:text-gray-400 mb-3">
                Found {discovered.length} server{discovered.length !== 1 ? "s" : ""}:
              </p>
              {discovered.map((server) => (
                <div
                  key={server.address}
                  className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg"
                >
                  <div>
                    <p className="font-medium text-gray-900 dark:text-white text-sm">
                      {server.name}
                    </p>
                    <p className="text-xs text-gray-500 dark:text-gray-400">
                      {server.address} · v{server.version}
                    </p>
                  </div>
                  <button
                    onClick={() => handleAddDiscovered(server)}
                    disabled={loading}
                    className="px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
                  >
                    Add
                  </button>
                </div>
              ))}
            </div>
          )}

          <button
            onClick={() => { setMode("choice"); setError(""); }}
            className="mt-4 text-sm text-blue-600 dark:text-blue-400 hover:underline"
          >
            &larr; Back
          </button>
        </div>
      )}

      {/* Manual entry form */}
      {mode === "manual" && (
        <form onSubmit={handleManualAdd} className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Server Name
            </label>
            <input
              type="text"
              value={manualName}
              onChange={(e) => setManualName(e.target.value)}
              placeholder="e.g., Living Room NAS"
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Address
            </label>
            <input
              type="text"
              value={manualAddress}
              onChange={(e) => setManualAddress(e.target.value)}
              placeholder="e.g., 192.168.1.100:8080 or backup.local:8080"
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
            <p className="text-xs text-gray-400 mt-1">
              IP address or hostname with port (e.g., 192.168.1.50:8080)
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              API Key <span className="text-gray-400">(optional)</span>
            </label>
            <input
              type="password"
              value={manualApiKey}
              onChange={(e) => setManualApiKey(e.target.value)}
              placeholder="Shared secret for authentication"
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Sync Frequency (hours)
            </label>
            <input
              type="number"
              min="1"
              max="168"
              value={manualFrequency}
              onChange={(e) => setManualFrequency(e.target.value)}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
            <p className="text-xs text-gray-400 mt-1">
              How often to sync photos to this backup server (default: every 24 hours)
            </p>
          </div>

          <div className="flex gap-3">
            <button
              type="button"
              onClick={() => { setMode("choice"); setError(""); }}
              className="flex-1 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 bg-gray-100 dark:bg-gray-700 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={loading || !manualName.trim() || !manualAddress.trim()}
              className="flex-1 px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
            >
              {loading ? "Adding..." : "Add Server"}
            </button>
          </div>
        </form>
      )}

      {/* Navigation */}
      <div className="flex justify-between mt-8 pt-6 border-t border-gray-200 dark:border-gray-700">
        <button
          onClick={() => { setStep("storage"); setError(""); }}
          className="text-sm text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
        >
          &larr; Back
        </button>
        <button
          onClick={() => { setStep("encryption"); setError(""); }}
          className="px-6 py-2 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors"
        >
          {addedServers.length > 0 ? "Continue" : "Skip"}
        </button>
      </div>
    </>
  );
}
