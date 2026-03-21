/** Wizard step — restore data from a backup server or local file. */
import { useState, useEffect } from "react";
import type { WizardStep, RestoreSource, DiscoveredServer } from "./types";

export interface RestoreStepProps {
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  error: string;
  onVerified: (source: RestoreSource) => void;
}

export default function RestoreStep({
  setStep,
  setError,
  error,
  onVerified,
}: RestoreStepProps) {
  const [serverAddress, setServerAddress] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [loading, setLoading] = useState(false);
  const [showPassword, setShowPassword] = useState(false);

  // Auto-discovery state
  const [discovering, setDiscovering] = useState(false);
  const [discovered, setDiscovered] = useState<DiscoveredServer[]>([]);
  const [hasScanned, setHasScanned] = useState(false);

  // Auto-discover on mount
  useEffect(() => {
    handleDiscover();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function handleDiscover() {
    setDiscovering(true);
    setError("");
    try {
      const res = await fetch("/api/setup/discover");
      if (!res.ok) throw new Error("Discovery failed");
      const data = await res.json();
      setDiscovered(data.servers ?? []);
    } catch {
      setDiscovered([]);
    } finally {
      setDiscovering(false);
      setHasScanned(true);
    }
  }

  function selectServer(address: string) {
    setServerAddress(address);
    setError("");
  }

  async function handleVerify(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (!serverAddress.trim()) {
      setError("Please select or enter the backup server address.");
      return;
    }
    if (!username.trim()) {
      setError("Please enter the admin username for the backup server.");
      return;
    }
    if (!password) {
      setError("Please enter the admin password for the backup server.");
      return;
    }

    setLoading(true);
    try {
      const res = await fetch("/api/setup/verify-backup", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          address: serverAddress.trim(),
          username: username.trim(),
          password,
        }),
      });

      if (!res.ok) {
        const body = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
        throw new Error(body.error || `Verification failed (HTTP ${res.status})`);
      }

      const data = await res.json();
      onVerified({
        address: data.address,
        name: data.name,
        version: data.version,
        api_key: data.api_key,
        photo_count: data.photo_count,
      });
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Verification failed");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">
        Restore from Backup
      </h2>
      <p className="text-gray-500 dark:text-gray-400 text-sm mb-6">
        Find your backup server on the network, then log in with admin
        credentials to verify access. Your photos will be synced after setup
        completes.
      </p>

      {/* Auto-Discovery Section */}
      <div className="mb-5">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300">
            Auto-Discover
          </h3>
          <button
            type="button"
            onClick={handleDiscover}
            disabled={discovering}
            className="text-xs text-blue-600 dark:text-blue-400 hover:underline disabled:opacity-50 flex items-center gap-1"
          >
            {discovering && (
              <span className="w-3 h-3 border-2 border-blue-600 border-t-transparent rounded-full animate-spin" />
            )}
            {discovering ? "Scanning\u2026" : "Rescan"}
          </button>
        </div>

        {discovering && !hasScanned && (
          <div className="flex items-center gap-3 p-4 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Scanning your network for Simple Photos servers&hellip;
            </p>
          </div>
        )}

        {hasScanned && !discovering && discovered.length === 0 && (
          <div className="p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg text-center">
            <svg
              className="w-8 h-8 mx-auto text-gray-300 dark:text-gray-600 mb-1"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1}
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M5.636 18.364a9 9 0 010-12.728m12.728 0a9 9 0 010 12.728m-9.9-2.829a5 5 0 010-7.07m7.072 0a5 5 0 010 7.07M13 12a1 1 0 11-2 0 1 1 0 012 0z"
              />
            </svg>
            <p className="text-xs text-gray-500 dark:text-gray-400">
              No servers found. Enter the address manually below.
            </p>
          </div>
        )}

        {discovered.length > 0 && (
          <div className="space-y-2">
            {discovered.map((server) => (
              <button
                key={server.address}
                type="button"
                onClick={() => selectServer(server.address)}
                className={`w-full flex items-center justify-between p-3 rounded-lg border-2 transition-colors text-left ${
                  serverAddress === server.address
                    ? "border-amber-500 bg-amber-50 dark:bg-amber-900/20"
                    : "border-gray-200 dark:border-gray-600 hover:border-amber-300 dark:hover:border-amber-500"
                }`}
              >
                <div className="flex items-center gap-3">
                  <div className="w-8 h-8 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center">
                    <svg
                      className="w-4 h-4 text-green-600 dark:text-green-400"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke="currentColor"
                      strokeWidth={1.5}
                    >
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z"
                      />
                    </svg>
                  </div>
                  <div>
                    <p className="font-medium text-gray-900 dark:text-white text-sm">
                      {server.name}
                    </p>
                    <p className="text-xs text-gray-500 dark:text-gray-400 font-mono">
                      {server.address} &middot; v{server.version}
                    </p>
                  </div>
                </div>
                {serverAddress === server.address && (
                  <svg
                    className="w-5 h-5 text-amber-500 shrink-0"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={2}
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      d="M4.5 12.75l6 6 9-13.5"
                    />
                  </svg>
                )}
              </button>
            ))}
          </div>
        )}
      </div>

      <form onSubmit={handleVerify} className="space-y-4">
        {/* Server address */}
        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
            Backup Server Address
          </label>
          <input
            type="text"
            value={serverAddress}
            onChange={(e) => setServerAddress(e.target.value)}
            placeholder="e.g. 192.168.1.20:8080"
            maxLength={500}
            className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-amber-500 focus:border-transparent font-mono"
          />
          <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
            IP address or hostname with port of the backup server
          </p>
        </div>

        {/* Divider */}
        <div className="relative py-2">
          <div className="absolute inset-0 flex items-center">
            <div className="w-full border-t border-gray-200 dark:border-gray-700" />
          </div>
          <div className="relative flex justify-center text-xs">
            <span className="px-2 bg-white dark:bg-gray-800 text-gray-400">
              admin credentials
            </span>
          </div>
        </div>

        {/* Username */}
        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
            Admin Username
          </label>
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="admin"
            autoComplete="username"
            className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-amber-500 focus:border-transparent"
          />
        </div>

        {/* Password */}
        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
            Admin Password
          </label>
          <div className="relative">
            <input
              type={showPassword ? "text" : "password"}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 pr-10 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-amber-500 focus:border-transparent"
            />
            <button
              type="button"
              onClick={() => setShowPassword(!showPassword)}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
              tabIndex={-1}
            >
              {showPassword ? (
                <svg
                  className="w-4 h-4"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={1.5}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M3.98 8.223A10.477 10.477 0 001.934 12C3.226 16.338 7.244 19.5 12 19.5c.993 0 1.953-.138 2.863-.395M6.228 6.228A10.45 10.45 0 0112 4.5c4.756 0 8.773 3.162 10.065 7.498a10.523 10.523 0 01-4.293 5.774M6.228 6.228L3 3m3.228 3.228l3.65 3.65m7.894 7.894L21 21m-3.228-3.228l-3.65-3.65m0 0a3 3 0 10-4.243-4.243m4.242 4.242L9.88 9.88"
                  />
                </svg>
              ) : (
                <svg
                  className="w-4 h-4"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={1.5}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178z"
                  />
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                  />
                </svg>
              )}
            </button>
          </div>
        </div>

        {/* Info box */}
        <div className="bg-amber-50 dark:bg-amber-900/20 rounded-lg p-3 text-sm">
          <div className="flex items-start gap-2">
            <svg
              className="w-4 h-4 text-amber-500 mt-0.5 shrink-0"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12V8.25z"
              />
            </svg>
            <p className="text-amber-700 dark:text-amber-300">
              We'll verify the connection to the backup server and check how
              many photos are available. After setup completes, photos will be
              synced automatically.
            </p>
          </div>
        </div>

        {error && (
          <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
            {error}
          </div>
        )}

        <div className="flex gap-3 pt-2">
          <button
            type="button"
            onClick={() => {
              setStep("install-type");
              setError("");
            }}
            className="flex-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 py-2.5 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 text-sm font-medium transition-colors"
          >
            &larr; Back
          </button>
          <button
            type="submit"
            disabled={
              loading || !serverAddress.trim() || !username.trim() || !password
            }
            className="flex-[2] bg-amber-600 text-white py-2.5 rounded-lg hover:bg-amber-700 disabled:opacity-50 text-sm font-medium transition-colors"
          >
            {loading ? (
              <span className="flex items-center justify-center gap-2">
                <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                Verifying&hellip;
              </span>
            ) : (
              "Verify & Continue \u2192"
            )}
          </button>
        </div>
      </form>
    </div>
  );
}
