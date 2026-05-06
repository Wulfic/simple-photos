/** SSL/TLS certificate configuration panel — upload cert/key, configure
 *  Let's Encrypt, or generate a self-signed local CA for LAN-only HTTPS. */
import { useState, useEffect } from "react";
import { api } from "../../api/client";
import { downloadRaw } from "../../api/core";
import { getErrorMessage } from "../../utils/formatters";

interface SslSettingsProps {
  error: string;
  setError: (e: string) => void;
  success: string;
  setSuccess: (s: string) => void;
}

export default function SslSettings({ setError, setSuccess }: SslSettingsProps) {
  const [sslEnabled, setSslEnabled] = useState(false);
  const [sslCertPath, setSslCertPath] = useState("");
  const [sslKeyPath, setSslKeyPath] = useState("");
  const [sslLoaded, setSslLoaded] = useState(false);
  const [sslSaving, setSslSaving] = useState(false);
  const [sslSaved, setSslSaved] = useState(false);

  // ── Let's Encrypt state ────────────────────────────────────────────
  const [leExisting, setLeExisting] = useState<{
    domain: string;
    email: string;
    staging: boolean;
    challenge_port: number;
    last_issued_at?: string | null;
  } | null>(null);
  const [leDomain, setLeDomain] = useState("");
  const [leEmail, setLeEmail] = useState("");
  const [leAgreeTos, setLeAgreeTos] = useState(false);
  const [leStaging, setLeStaging] = useState(false);
  const [leChallengePort, setLeChallengePort] = useState("80");
  const [leProvisioning, setLeProvisioning] = useState(false);
  const [leMessage, setLeMessage] = useState("");

  // ── Local CA state ─────────────────────────────────────────────────
  // 4th TLS option: server generates its own root CA + leaf, the operator
  // installs the public root on each client device.  No third party, no
  // public DNS, no inbound firewall rules.
  const [lcExisting, setLcExisting] = useState<{
    generated_at: string;
    ca_expires_at: string;
    cert_expires_at: string;
    hosts: string[];
    fingerprint_sha256: string;
  } | null>(null);
  const [lcLabel, setLcLabel] = useState("");
  const [lcExtraHosts, setLcExtraHosts] = useState("");
  const [lcGenerating, setLcGenerating] = useState(false);
  const [lcMessage, setLcMessage] = useState("");
  const [lcDownloading, setLcDownloading] = useState(false);

  useEffect(() => {
    loadSslSettings();
  }, []);

  async function loadSslSettings() {
    try {
      const res = await api.admin.getSsl();
      setSslEnabled(res.enabled);
      setSslCertPath(res.cert_path ?? "");
      setSslKeyPath(res.key_path ?? "");
      if (res.letsencrypt) {
        setLeExisting(res.letsencrypt);
        setLeDomain(res.letsencrypt.domain);
        setLeEmail(res.letsencrypt.email);
        setLeStaging(res.letsencrypt.staging);
        setLeChallengePort(String(res.letsencrypt.challenge_port ?? 80));
      }
      if (res.local_ca) {
        setLcExisting(res.local_ca);
      }
      setSslLoaded(true);
    } catch {
      // Not admin or SSL endpoints not available — silently skip
    }
  }

  async function handleProvisionLetsEncrypt() {
    setError("");
    setLeMessage("");

    const port = parseInt(leChallengePort, 10);
    if (isNaN(port) || port < 1 || port > 65535) {
      setError("Challenge port must be between 1 and 65535.");
      return;
    }
    if (!leAgreeTos) {
      setError("You must agree to the Let's Encrypt Subscriber Agreement.");
      return;
    }
    if (!leDomain.trim() || !leEmail.trim()) {
      setError("Domain and email are required.");
      return;
    }

    setLeProvisioning(true);
    try {
      // Dry-run validates inputs without contacting the CA.
      await api.admin.provisionLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        agree_tos: leAgreeTos,
        staging: leStaging,
        challenge_port: port,
        dry_run: true,
      });
      const res = await api.admin.provisionLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        agree_tos: leAgreeTos,
        staging: leStaging,
        challenge_port: port,
        dry_run: false,
      });
      setLeMessage(
        `Certificate issued for ${res.domain}${res.staging ? " (staging)" : ""}. Restart the server to begin serving HTTPS.`,
      );
      setSuccess("Let's Encrypt certificate issued. Restart the server to apply.");
      // Refresh status so the existing-cert badge appears.
      void loadSslSettings();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to provision Let's Encrypt certificate"));
    } finally {
      setLeProvisioning(false);
    }
  }

  async function handleSaveSsl() {
    setSslSaving(true);
    setError("");
    try {
      await api.admin.updateSsl({
        enabled: sslEnabled,
        cert_path: sslCertPath || undefined,
        key_path: sslKeyPath || undefined,
      });
      setSslSaved(true);
      setSuccess("TLS configuration saved. Restart the server to apply changes.");
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setSslSaving(false);
    }
  }

  async function handleGenerateLocalCa() {
    setError("");
    setLcMessage("");

    const extras = lcExtraHosts
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    setLcGenerating(true);
    try {
      // Dry-run validates the inputs before mutating the filesystem.
      await api.admin.provisionLocalCa({
        label: lcLabel.trim() || undefined,
        extra_hosts: extras,
        dry_run: true,
      });
      const res = await api.admin.provisionLocalCa({
        label: lcLabel.trim() || undefined,
        extra_hosts: extras,
        dry_run: false,
      });
      setLcMessage(
        `Local CA generated. Fingerprint: ${res.fingerprint_sha256}. Restart the server to begin serving HTTPS.`,
      );
      setSuccess(
        "Self-signed local CA generated. Download the bundle and install it on every device that connects to this server.",
      );
      void loadSslSettings();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to generate local CA"));
    } finally {
      setLcGenerating(false);
    }
  }

  async function handleDownloadLocalCaBundle() {
    setError("");
    setLcDownloading(true);
    try {
      const buf = await downloadRaw(api.admin.localCaBundleUrl());
      const blob = new Blob([buf], { type: "application/zip" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "simple-photos-ca-bundle.zip";
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      // Revoke after a tick so Safari has time to start the download.
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to download CA bundle"));
    } finally {
      setLcDownloading(false);
    }
  }

  if (!sslLoaded) return null;

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">SSL / TLS</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Serve your photos over HTTPS with a TLS certificate.
        Changes require a server restart.
      </p>

      {/* Enable toggle */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">Enable TLS</h3>
          <p className="text-xs text-gray-500 dark:text-gray-400">
            {sslEnabled ? "HTTPS is enabled." : "Running on plain HTTP."}
          </p>
        </div>
        <button
          onClick={() => {
            setSslEnabled(!sslEnabled);
            setSslSaved(false);
          }}
          className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
            sslEnabled ? "bg-blue-600" : "bg-gray-300 dark:bg-gray-600"
          }`}
          role="switch"
          aria-checked={sslEnabled}
        >
          <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
              sslEnabled ? "translate-x-6" : "translate-x-1"
            }`}
          />
        </button>
      </div>

      {/* Certificate fields */}
      {sslEnabled && (
        <div className="space-y-3">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Certificate Path
            </label>
            <input
              type="text"
              value={sslCertPath}
              onChange={(e) => { setSslCertPath(e.target.value); setSslSaved(false); }}
              placeholder="/etc/ssl/certs/my-cert.pem"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Private Key Path
            </label>
            <input
              type="text"
              value={sslKeyPath}
              onChange={(e) => { setSslKeyPath(e.target.value); setSslSaved(false); }}
              placeholder="/etc/ssl/private/my-key.pem"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <button
            onClick={handleSaveSsl}
            disabled={sslSaving}
            className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
          >
            {sslSaving ? "Saving…" : sslSaved ? "✓ Saved" : "Save TLS Configuration"}
          </button>
        </div>
      )}

      {/* Disable save btn */}
      {!sslEnabled && (
        <button
          onClick={handleSaveSsl}
          disabled={sslSaving}
          className="mt-2 bg-gray-600 text-white px-4 py-2 rounded-md hover:bg-gray-700 disabled:opacity-50 text-sm"
        >
          {sslSaving ? "Saving…" : "Disable TLS & Save"}
        </button>
      )}

      {/* ── Let's Encrypt panel ──────────────────────────────────── */}
      <div className="mt-6 pt-6 border-t border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-semibold mb-2">Let's Encrypt (automatic)</h3>
        <p className="text-xs text-gray-500 dark:text-gray-400 mb-3">
          Issue or renew a free trusted certificate via the ACME-v2 HTTP-01
          challenge.  Requires a public DNS name pointing at this server and
          inbound port {leChallengePort || "80"} reachable from the internet.
        </p>

        {leExisting && (
          <div className="mb-3 p-3 rounded-md bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 text-xs">
            <div className="font-medium text-blue-800 dark:text-blue-300">
              Active Let's Encrypt certificate
            </div>
            <div className="text-blue-700 dark:text-blue-400 mt-1">
              Domain: <span className="font-mono">{leExisting.domain}</span>
              {leExisting.staging ? " (staging)" : ""}
            </div>
            {leExisting.last_issued_at && (
              <div className="text-blue-700 dark:text-blue-400">
                Last issued: {new Date(leExisting.last_issued_at).toLocaleString()}
              </div>
            )}
          </div>
        )}

        <div className="space-y-3">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Domain (FQDN)
            </label>
            <input
              type="text"
              value={leDomain}
              onChange={(e) => setLeDomain(e.target.value)}
              placeholder="photos.example.com"
              autoComplete="off"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              disabled={leProvisioning}
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Contact email
            </label>
            <input
              type="email"
              value={leEmail}
              onChange={(e) => setLeEmail(e.target.value)}
              placeholder="admin@example.com"
              autoComplete="off"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              disabled={leProvisioning}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                HTTP-01 port
              </label>
              <input
                type="number"
                min={1}
                max={65535}
                value={leChallengePort}
                onChange={(e) => setLeChallengePort(e.target.value)}
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                disabled={leProvisioning}
              />
            </div>
            <label className="flex items-start gap-2 mt-6 text-sm">
              <input
                type="checkbox"
                checked={leStaging}
                onChange={(e) => setLeStaging(e.target.checked)}
                className="mt-0.5 accent-blue-600"
                disabled={leProvisioning}
              />
              <span className="text-gray-700 dark:text-gray-300">
                Staging directory (testing)
              </span>
            </label>
          </div>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="checkbox"
              checked={leAgreeTos}
              onChange={(e) => setLeAgreeTos(e.target.checked)}
              className="mt-0.5 accent-blue-600"
              disabled={leProvisioning}
            />
            <span className="text-gray-700 dark:text-gray-300">
              I agree to the{" "}
              <a
                href="https://letsencrypt.org/repository/"
                target="_blank"
                rel="noopener noreferrer"
                className="text-blue-600 dark:text-blue-400 underline"
              >
                Let's Encrypt Subscriber Agreement
              </a>
              .
            </span>
          </label>
          <button
            onClick={handleProvisionLetsEncrypt}
            disabled={leProvisioning}
            className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
          >
            {leProvisioning
              ? "Requesting certificate from Let's Encrypt…"
              : leExisting
                ? "Renew certificate"
                : "Issue certificate"}
          </button>
          {leMessage && (
            <p className="text-sm text-green-700 dark:text-green-400">{leMessage}</p>
          )}
        </div>
      </div>

      {/* ── Self-signed Local CA panel ────────────────────────────── */}
      <div className="mt-6 pt-6 border-t border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-semibold mb-2">Self-signed local CA (no third-party)</h3>
        <p className="text-xs text-gray-500 dark:text-gray-400 mb-3">
          Generate a long-lived root CA and a server certificate signed by it.
          Install the public root on each device (Linux, Windows, Android) using
          the bundled scripts and you'll get a real, trusted HTTPS connection on
          your LAN — no Let's Encrypt, no public DNS, no inbound firewall rules.
          The private keys never leave the server.
        </p>

        {lcExisting && (
          <div className="mb-3 p-3 rounded-md bg-emerald-50 dark:bg-emerald-900/20 border border-emerald-200 dark:border-emerald-800 text-xs">
            <div className="font-medium text-emerald-800 dark:text-emerald-300">
              Active local CA
            </div>
            <div className="text-emerald-700 dark:text-emerald-400 mt-1 break-all">
              Fingerprint (SHA-256): <span className="font-mono">{lcExisting.fingerprint_sha256}</span>
            </div>
            <div className="text-emerald-700 dark:text-emerald-400 mt-1">
              Generated: {new Date(lcExisting.generated_at).toLocaleString()}
            </div>
            <div className="text-emerald-700 dark:text-emerald-400">
              Leaf expires: {new Date(lcExisting.cert_expires_at).toLocaleDateString()}{" · "}
              CA expires: {new Date(lcExisting.ca_expires_at).toLocaleDateString()}
            </div>
            {lcExisting.hosts.length > 0 && (
              <div className="text-emerald-700 dark:text-emerald-400 mt-1">
                Hosts: <span className="font-mono">{lcExisting.hosts.join(", ")}</span>
              </div>
            )}
            <button
              onClick={handleDownloadLocalCaBundle}
              disabled={lcDownloading}
              className="mt-3 bg-emerald-600 text-white px-4 py-2 rounded-md hover:bg-emerald-700 disabled:opacity-50 text-sm"
            >
              {lcDownloading ? "Preparing download…" : "⬇ Download CA install bundle (.zip)"}
            </button>
            <p className="text-emerald-700 dark:text-emerald-400 mt-2 text-[11px] leading-snug">
              The zip contains <code className="font-mono">ca.pem</code>, plus install
              scripts for Linux (<code className="font-mono">install-linux.sh</code>),
              Windows (<code className="font-mono">install-windows.ps1</code>), and
              Android (<code className="font-mono">install-android.txt</code>).
              Verify the fingerprint above matches the one printed by the install
              script before trusting the certificate.
            </p>
          </div>
        )}

        <div className="space-y-3">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              CA label (optional)
            </label>
            <input
              type="text"
              value={lcLabel}
              onChange={(e) => setLcLabel(e.target.value)}
              placeholder="Simple Photos Local CA — kitchen-NAS"
              maxLength={128}
              autoComplete="off"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-emerald-500"
              disabled={lcGenerating}
            />
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
              Shown in the OS trust store. Defaults to the server's hostname.
            </p>
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Extra hosts (optional, space- or comma-separated)
            </label>
            <input
              type="text"
              value={lcExtraHosts}
              onChange={(e) => setLcExtraHosts(e.target.value)}
              placeholder="photos.local 192.168.1.50"
              autoComplete="off"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-emerald-500"
              disabled={lcGenerating}
            />
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
              Localhost, the server hostname, and detected LAN IPs are added
              automatically. Add custom DNS names or static IPs here.
            </p>
          </div>
          <button
            onClick={handleGenerateLocalCa}
            disabled={lcGenerating}
            className="bg-emerald-600 text-white px-4 py-2 rounded-md hover:bg-emerald-700 disabled:opacity-50 text-sm"
          >
            {lcGenerating
              ? "Generating CA…"
              : lcExisting
                ? "Re-generate local CA"
                : "Generate local CA"}
          </button>
          {lcMessage && (
            <p className="text-sm text-emerald-700 dark:text-emerald-400 break-all">
              {lcMessage}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
