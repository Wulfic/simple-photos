/** Wizard step — display a pairing code so a backup server can connect. */
import { useState, useEffect } from "react";
import type { WizardStep, DiscoveredServer } from "./types";

export interface PairStepProps {
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  error: string;
  /** Called on successful pairing with the local auth tokens + username */
  onPaired: (data: {
    access_token: string;
    refresh_token: string;
    username: string;
    main_server_url: string;
    password?: string;
  }) => void;
}

/**
 * Heuristic: does the given host look like an RFC1918/private/loopback
 * address? Used to decide whether to warn the operator that the primary
 * server may not be able to dial back to this backup over the WAN.
 */
function isPrivateOrLoopbackHost(host: string): boolean {
  const h = host.trim().toLowerCase().replace(/:\d+$/, "");
  if (!h) return true;
  if (h === "localhost" || h.endsWith(".local") || h.endsWith(".lan")) return true;
  if (h === "::1" || h === "0.0.0.0" || h === "::") return true;
  if (h.startsWith("127.")) return true;
  if (h.startsWith("10.")) return true;
  if (h.startsWith("192.168.")) return true;
  if (h.startsWith("169.254.")) return true;
  // 172.16.0.0 — 172.31.255.255
  const m = h.match(/^172\.(\d+)\./);
  if (m) {
    const second = parseInt(m[1], 10);
    if (second >= 16 && second <= 31) return true;
  }
  return false;
}

/** Extract the host[:port] portion of a server address or URL. */
function extractHost(addr: string): string {
  let s = addr.trim();
  s = s.replace(/^https?:\/\//i, "");
  s = s.split("/")[0];
  return s;
}

export default function PairStep({
  setStep,
  setError,
  error,
  onPaired,
}: PairStepProps) {
  const [serverAddress, setServerAddress] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [requiresTotp, setRequiresTotp] = useState(false);
  const [loading, setLoading] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  // Backup's externally-reachable URL — what the primary will dial back to.
  // Pre-filled with the URL the operator is currently using to reach this
  // backup in their browser; that is, by definition, a working address —
  // though if it's a LAN address and the primary is on the WAN it will
  // need to be replaced. The UI surfaces a warning when that mismatch is
  // detected.
  const [backupPublicUrl, setBackupPublicUrl] = useState<string>(
    typeof window !== "undefined" ? window.location.origin : "",
  );

  // Auto-discovery state
  const [discovering, setDiscovering] = useState(false);
  const [discovered, setDiscovered] = useState<DiscoveredServer[]>([]);
  const [hasScanned, setHasScanned] = useState(false);

  // Auto-discover on mount
  useEffect(() => {
    handleDiscover();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps -- Intentionally runs once on mount.
  // handleDiscover is a local function that fires a one-time discovery request.

  async function handleDiscover() {
    setDiscovering(true);
    setError("");
    try {
      const res = await fetch("/api/setup/discover");
      if (!res.ok) throw new Error("Discovery failed");
      const data = await res.json();
      setDiscovered(data.servers ?? []);
    } catch {
      // Non-critical — user can still enter manually
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

  async function handlePair(e: React.FormEvent) {
    e.preventDefault();
    setError("");

    if (!serverAddress.trim()) {
      setError("Please enter the primary server address.");
      return;
    }
    if (!username.trim()) {
      setError("Please enter the admin username for the primary server.");
      return;
    }
    if (!password) {
      setError("Please enter the admin password for the primary server.");
      return;
    }

    setLoading(true);
    try {
      const payload: Record<string, string> = {
        main_server_url: serverAddress.trim(),
        username: username.trim(),
        password,
      };
      if (requiresTotp && totpCode.trim()) {
        payload.totp_code = totpCode.trim();
      }
      if (backupPublicUrl.trim()) {
        payload.backup_public_url = backupPublicUrl.trim();
      }

      const res = await fetch("/api/setup/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });

      if (!res.ok) {
        const body = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
        throw new Error(body.error || `Pairing failed (HTTP ${res.status})`);
      }

      const data = await res.json();

      // If the primary requires 2FA, show the TOTP input
      if (data.requires_totp) {
        setRequiresTotp(true);
        setTotpCode("");
        setError("");
        return;
      }
      onPaired({
        access_token: data.access_token,
        refresh_token: data.refresh_token,
        username: data.username,
        main_server_url: data.main_server_url,
        password,
      });
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Pairing failed");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div>
      <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">
        Pair with Primary Server
      </h2>
      <p className="text-gray-700 dark:text-gray-400 text-sm mb-6">
        Find your primary Simple Photos server on the network or enter its
        address manually, then log in with its admin credentials.
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
            className="text-xs text-accent-600 dark:text-accent-400 hover:underline disabled:opacity-50 flex items-center gap-1"
          >
            {discovering && (
              <span className="w-3 h-3 border-2 border-accent-200 dark:border-accent-900 border-t-accent-600 dark:border-t-accent-400 rounded-full animate-spin" />
            )}
            {discovering ? "Scanning…" : "Rescan"}
          </button>
        </div>

        {discovering && !hasScanned && (
          <div className="flex items-center gap-3 p-4 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Scanning your network for Simple Photos servers…
            </p>
          </div>
        )}

        {hasScanned && !discovering && discovered.length === 0 && (
          <div className="p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg text-center">
            <svg className="w-8 h-8 mx-auto text-gray-300 dark:text-gray-600 mb-1" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z" />
            </svg>
            <p className="text-xs text-gray-700 dark:text-gray-400">
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
                    ? "border-accent-500 bg-accent-50 dark:bg-accent-900/20"
                    : "border-gray-200 dark:border-gray-600 hover:border-accent-300 dark:hover:border-accent-500"
                }`}
              >
                <div className="flex items-center gap-3">
                  <div className="w-8 h-8 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center">
                    <svg className="w-4 h-4 text-green-600 dark:text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z" />
                    </svg>
                  </div>
                  <div>
                    <p className="font-medium text-gray-900 dark:text-white text-sm">
                      {server.name}
                    </p>
                    <p className="text-xs text-gray-700 dark:text-gray-400 font-mono">
                      {server.address} · v{server.version}
                    </p>
                  </div>
                </div>
                {serverAddress === server.address && (
                  <svg className="w-5 h-5 text-accent-500 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M4.5 12.75l6 6 9-13.5" />
                  </svg>
                )}
              </button>
            ))}
          </div>
        )}
      </div>

      <form onSubmit={handlePair} className="space-y-4">
        {/* Server address */}
        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
            Primary Server Address
          </label>
          <input
            type="text"
            value={serverAddress}
            onChange={(e) => setServerAddress(e.target.value)}
            placeholder="e.g. 192.168.1.10:8080 or photos.example.com:8080"
            maxLength={500}
            className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-accent-500 focus:border-transparent font-mono"
          />
          <p className="text-xs text-gray-600 dark:text-gray-500 mt-1">
            IP address or hostname (DNS name) with port of the primary server
          </p>
        </div>

        {/* Backup public URL — what the primary will connect *back* to. */}
        {(() => {
          const primaryHost = extractHost(serverAddress);
          const backupHost = extractHost(backupPublicUrl);
          const primaryRemote =
            primaryHost.length > 0 && !isPrivateOrLoopbackHost(primaryHost);
          const backupPrivate =
            backupHost.length > 0 && isPrivateOrLoopbackHost(backupHost);
          const mismatchWarning = primaryRemote && backupPrivate;
          return (
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                This Backup Server&rsquo;s Public URL
              </label>
              <input
                type="text"
                value={backupPublicUrl}
                onChange={(e) => setBackupPublicUrl(e.target.value)}
                placeholder="e.g. https://backup.example.com or http://203.0.113.5:8080"
                maxLength={500}
                className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-accent-500 focus:border-transparent font-mono"
              />
              <p className="text-xs text-gray-600 dark:text-gray-500 mt-1">
                The URL the <strong>primary</strong> server will use to push
                photos to this backup. The primary must be able to reach this
                URL over the network.
              </p>
              <div
                className={`mt-2 rounded-lg p-3 text-xs ${
                  mismatchWarning
                    ? "bg-amber-50 dark:bg-amber-900/20 text-amber-800 dark:text-amber-200 border border-amber-300 dark:border-amber-700"
                    : "bg-gray-50 dark:bg-gray-700/40 text-gray-600 dark:text-gray-400"
                }`}
              >
                <p className="font-semibold mb-1">
                  {mismatchWarning
                    ? "⚠ Action required: this address looks unreachable from your primary server"
                    : "Port forwarding may be required"}
                </p>
                <ul className="list-disc list-inside space-y-1">
                  <li>
                    Backup replication is <strong>push-only</strong>: the
                    primary server connects out to this backup. If the primary
                    cannot reach the URL above, no photos will sync.
                  </li>
                  <li>
                    For a backup behind NAT/a home router, you must
                    {" "}
                    <strong>open and forward the port</strong> on your router
                    to this machine, then use your{" "}
                    <strong>public IP or DNS hostname</strong> here (not a
                    LAN/private address).
                  </li>
                  <li>
                    Tunnels (Tailscale, Cloudflare Tunnel, WireGuard, ngrok,
                    etc.) work too — use the tunnel&rsquo;s public hostname.
                  </li>
                  <li>
                    Self-signed TLS? The primary&rsquo;s{" "}
                    <code>backup.accept_invalid_certs</code> config flag must
                    be enabled, otherwise use plain <code>http://</code> or a
                    valid certificate (e.g. Let&rsquo;s Encrypt).
                  </li>
                </ul>
                {mismatchWarning && (
                  <p className="mt-2">
                    The primary at <code>{primaryHost}</code> is on the public
                    internet, but the backup URL points at{" "}
                    <code>{backupHost}</code>, which looks like a private/LAN
                    address. The primary will likely fail to connect. Replace
                    it with the backup&rsquo;s externally-reachable address.
                  </p>
                )}
              </div>
            </div>
          );
        })()}

        {/* Divider */}
        <div className="relative py-2">
          <div className="absolute inset-0 flex items-center">
            <div className="w-full border-t border-gray-200 dark:border-gray-700" />
          </div>
          <div className="relative flex justify-center text-xs">
            <span className="px-2 bg-white dark:bg-gray-800 text-gray-600 dark:text-gray-400">
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
            className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-accent-500 focus:border-transparent"
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
              className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 pr-10 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-accent-500 focus:border-transparent"
            />
            <button
              type="button"
              onClick={() => setShowPassword(!showPassword)}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-600 dark:text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
              tabIndex={-1}
            >
              {showPassword ? (
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M3.98 8.223A10.477 10.477 0 001.934 12C3.226 16.338 7.244 19.5 12 19.5c.993 0 1.953-.138 2.863-.395M6.228 6.228A10.45 10.45 0 0112 4.5c4.756 0 8.773 3.162 10.065 7.498a10.523 10.523 0 01-4.293 5.774M6.228 6.228L3 3m3.228 3.228l3.65 3.65m7.894 7.894L21 21m-3.228-3.228l-3.65-3.65m0 0a3 3 0 10-4.243-4.243m4.242 4.242L9.88 9.88" />
                </svg>
              ) : (
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178z" />
                  <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                </svg>
              )}
            </button>
          </div>
        </div>

        {/* Info box */}
        <div className="bg-accent-50 dark:bg-accent-900/20 rounded-lg p-3 text-sm">
          <div className="flex items-start gap-2">
            <svg className="w-4 h-4 text-accent-500 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12V8.25z" />
            </svg>
            <p className="text-accent-700 dark:text-accent-300">
              {requiresTotp
                ? "The primary server has 2FA enabled. Enter the 6-digit code from your authenticator app."
                : "Please log in using the admin username and password from your primary server."}
            </p>
          </div>
        </div>

        {/* 2FA Code — shown when primary requires TOTP */}
        {requiresTotp && (
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              2FA Code
            </label>
            <input
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              maxLength={6}
              value={totpCode}
              onChange={(e) => setTotpCode(e.target.value.replace(/\D/g, ""))}
              placeholder="000000"
              autoFocus
              autoComplete="one-time-code"
              className="w-full border border-gray-300 dark:border-gray-600 rounded-lg px-4 py-2.5 bg-white dark:bg-gray-700 text-gray-900 dark:text-white text-sm focus:outline-none focus:ring-2 focus:ring-accent-500 focus:border-transparent font-mono tracking-widest text-center text-lg"
            />
          </div>
        )}

        {error && (
          <div className="text-red-600 dark:text-red-400 text-sm p-3 bg-red-50 dark:bg-red-900/30 rounded-lg">
            {error}
          </div>
        )}

        <div className="flex gap-3 pt-2">
          <button
            type="button"
            onClick={() => {
              setStep("server-role");
              setError("");
            }}
            className="btn btn-secondary btn-md flex-1"
          >
            &larr; Back
          </button>
          <button
            type="submit"
            disabled={loading || !serverAddress.trim() || !username.trim() || !password || !backupPublicUrl.trim() || (requiresTotp && totpCode.length !== 6)}
            className="btn btn-success btn-md flex-[2]"
          >
            {loading ? (
              <span className="flex items-center justify-center gap-2">
                <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                {requiresTotp ? "Verifying…" : "Pairing…"}
              </span>
            ) : requiresTotp ? (
              "Verify & Pair →"
            ) : (
              "Pair & Continue →"
            )}
          </button>
        </div>
      </form>
    </div>
  );
}
