/** SSL/TLS configuration panel.
 *
 * UX rules:
 *   • The "Enable TLS" toggle is the single source of truth — flipping it
 *     immediately persists `tls.enabled` to config.toml.  No separate
 *     save / disable buttons.
 *   • When TLS is on, a status card shows the URL the server is reachable
 *     at and the active certificate paths so the operator knows the setup
 *     is working without hunting through config.toml.
 *   • The three provisioning methods (Let's Encrypt, self-signed local
 *     CA, manual cert/key paths) live in collapsible disclosures so the
 *     panel doesn't sprawl.  Only one is open at a time.
 */
import { useState, useEffect, type ReactNode } from "react";
import { api } from "../../api/client";
import { downloadRaw } from "../../api/core";
import { getErrorMessage } from "../../utils/formatters";

interface SslSettingsProps {
  error: string;
  setError: (e: string) => void;
  success: string;
  setSuccess: (s: string) => void;
}

type ProvMethod = "letsencrypt" | "local_ca" | "manual" | null;

export default function SslSettings({ setError, setSuccess }: SslSettingsProps) {
  const [sslEnabled, setSslEnabled] = useState(false);
  const [sslCertPath, setSslCertPath] = useState("");
  const [sslKeyPath, setSslKeyPath] = useState("");
  const [sslLoaded, setSslLoaded] = useState(false);
  const [togglePending, setTogglePending] = useState(false);

  // Which provisioning section is open (accordion — one at a time).
  const [openMethod, setOpenMethod] = useState<ProvMethod>(null);

  // ── Manual cert state ──────────────────────────────────────────────
  const [manualCert, setManualCert] = useState("");
  const [manualKey, setManualKey] = useState("");
  const [manualSaving, setManualSaving] = useState(false);

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

  // ── Local CA state ─────────────────────────────────────────────────
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
  const [lcDownloading, setLcDownloading] = useState(false);

  useEffect(() => {
    void loadSslSettings();
  }, []);

  async function loadSslSettings() {
    try {
      const res = await api.admin.getSsl();
      setSslEnabled(res.enabled);
      setSslCertPath(res.cert_path ?? "");
      setSslKeyPath(res.key_path ?? "");
      setManualCert(res.cert_path ?? "");
      setManualKey(res.key_path ?? "");
      if (res.letsencrypt) {
        setLeExisting(res.letsencrypt);
        setLeDomain(res.letsencrypt.domain);
        setLeEmail(res.letsencrypt.email);
        setLeStaging(res.letsencrypt.staging);
        setLeChallengePort(String(res.letsencrypt.challenge_port ?? 80));
      } else {
        setLeExisting(null);
      }
      setLcExisting(res.local_ca ?? null);
      setSslLoaded(true);
    } catch {
      // Not admin or SSL endpoints not available — silently skip.
    }
  }

  /** Persist `enabled` flip immediately.  Cert/key paths are kept as-is
   *  so the operator can flip TLS off and on without re-entering them. */
  async function handleToggleEnabled(next: boolean) {
    setError("");
    setTogglePending(true);
    try {
      await api.admin.updateSsl({
        enabled: next,
        cert_path: sslCertPath || undefined,
        key_path: sslKeyPath || undefined,
      });
      setSslEnabled(next);
      setSuccess(
        next
          ? "TLS enabled. Restart the server to begin serving HTTPS."
          : "TLS disabled. Restart the server to revert to plain HTTP.",
      );
      void loadSslSettings();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to update TLS state"));
    } finally {
      setTogglePending(false);
    }
  }

  async function handleSaveManual() {
    if (!manualCert.trim() || !manualKey.trim()) {
      setError("Both certificate path and key path are required.");
      return;
    }
    setError("");
    setManualSaving(true);
    try {
      await api.admin.updateSsl({
        enabled: true,
        cert_path: manualCert.trim(),
        key_path: manualKey.trim(),
      });
      setSuccess("Certificate paths saved. Restart the server to apply.");
      void loadSslSettings();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to save certificate paths"));
    } finally {
      setManualSaving(false);
    }
  }

  async function handleProvisionLetsEncrypt() {
    setError("");
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
      setSuccess(
        `Let's Encrypt certificate issued for ${res.domain}${res.staging ? " (staging)" : ""}. Restart the server to apply.`,
      );
      void loadSslSettings();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to provision Let's Encrypt certificate"));
    } finally {
      setLeProvisioning(false);
    }
  }

  async function handleGenerateLocalCa() {
    setError("");
    const extras = lcExtraHosts
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    setLcGenerating(true);
    try {
      await api.admin.provisionLocalCa({
        label: lcLabel.trim() || undefined,
        extra_hosts: extras,
        dry_run: true,
      });
      await api.admin.provisionLocalCa({
        label: lcLabel.trim() || undefined,
        extra_hosts: extras,
        dry_run: false,
      });
      setSuccess(
        "Local CA generated. Download the bundle and install it on every device that connects to this server, then restart the server.",
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
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to download CA bundle"));
    } finally {
      setLcDownloading(false);
    }
  }

  if (!sslLoaded) return null;

  // The browser is currently talking to *some* address — show it as the
  // canonical "this is what your users hit" URL.  We swap the protocol to
  // https when TLS is enabled, since after restart that's what will work.
  const currentHost = window.location.host;
  const tlsUrl = `https://${currentHost}`;
  const httpUrl = `http://${currentHost}`;

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-1">SSL / TLS</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Serve your photos over HTTPS. Toggling this saves immediately;
        a server restart is required for the change to take effect.
      </p>

      {/* ── Enable toggle ─────────────────────────────────────────── */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">Enable TLS</h3>
          <p className="text-xs text-gray-500 dark:text-gray-400">
            {sslEnabled ? "HTTPS is enabled." : "Running on plain HTTP."}
          </p>
        </div>
        <button
          onClick={() => void handleToggleEnabled(!sslEnabled)}
          disabled={togglePending}
          className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:opacity-50 ${
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

      {/* ── TLS status card ───────────────────────────────────────── */}
      {sslEnabled ? (
        <div className="mb-5 p-3 rounded-md bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 text-xs space-y-1">
          <div className="flex items-center gap-2">
            <svg className="w-4 h-4 text-blue-600 dark:text-blue-300" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span className="font-medium text-blue-800 dark:text-blue-300">TLS configured</span>
          </div>
          <div className="text-blue-700 dark:text-blue-400">
            Server URL after restart:{" "}
            <a href={tlsUrl} className="font-mono underline" target="_blank" rel="noopener noreferrer">{tlsUrl}</a>
          </div>
          {sslCertPath && (
            <div className="text-blue-700 dark:text-blue-400">
              Certificate: <span className="font-mono break-all">{sslCertPath}</span>
            </div>
          )}
          {sslKeyPath && (
            <div className="text-blue-700 dark:text-blue-400">
              Private key: <span className="font-mono break-all">{sslKeyPath}</span>
            </div>
          )}
          <div className="text-blue-700 dark:text-blue-400 italic">
            Plain-HTTP requests to <span className="font-mono">{httpUrl}</span> will be 301-upgraded to HTTPS.
          </div>
        </div>
      ) : (
        <div className="mb-5 p-3 rounded-md bg-gray-50 dark:bg-gray-700/40 border border-gray-200 dark:border-gray-700 text-xs text-gray-600 dark:text-gray-400">
          TLS is disabled. The server is reachable at{" "}
          <span className="font-mono">{httpUrl}</span>. Pick one of the
          options below to obtain or install a certificate.
        </div>
      )}

      {/* ── Provisioning methods (accordion) ──────────────────────── */}
      <div className="space-y-2">
        <Disclosure
          title="Let's Encrypt (automatic, public domain)"
          subtitle="Issue or renew a free trusted certificate via the ACME-v2 HTTP-01 challenge."
          badge={leExisting ? `${leExisting.domain}${leExisting.staging ? " · staging" : ""}` : null}
          tone="blue"
          open={openMethod === "letsencrypt"}
          onToggle={() =>
            setOpenMethod(openMethod === "letsencrypt" ? null : "letsencrypt")
          }
        >
          <p className="text-xs text-gray-500 dark:text-gray-400 mb-3">
            Requires a public DNS name pointing at this server and inbound
            port {leChallengePort || "80"} reachable from the internet.
          </p>

          {leExisting && (
            <div className="mb-3 p-2 rounded bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 text-xs">
              <div className="font-medium text-blue-800 dark:text-blue-300">Active</div>
              <div className="text-blue-700 dark:text-blue-400">
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
          </div>
        </Disclosure>

        <Disclosure
          title="Self-signed local CA (LAN / offline)"
          subtitle="Generate your own root CA + leaf certificate. Best for LAN-only or air-gapped deployments."
          badge={lcExisting ? "Active" : null}
          tone="emerald"
          open={openMethod === "local_ca"}
          onToggle={() =>
            setOpenMethod(openMethod === "local_ca" ? null : "local_ca")
          }
        >
          <p className="text-xs text-gray-500 dark:text-gray-400 mb-3">
            Install the public root on each device using the bundled scripts
            and you'll get a real, trusted HTTPS connection on your LAN —
            no Let's Encrypt, no public DNS, no inbound firewall rules.
            The private keys never leave the server.
          </p>

          {lcExisting && (
            <div className="mb-3 p-3 rounded-md bg-emerald-50 dark:bg-emerald-900/20 border border-emerald-200 dark:border-emerald-800 text-xs">
              <div className="font-medium text-emerald-800 dark:text-emerald-300">
                Active local CA
              </div>
              <div className="text-emerald-700 dark:text-emerald-400 mt-1 break-all">
                Fingerprint (SHA-256):{" "}
                <span className="font-mono">{lcExisting.fingerprint_sha256}</span>
              </div>
              <div className="text-emerald-700 dark:text-emerald-400 mt-1">
                Generated: {new Date(lcExisting.generated_at).toLocaleString()}
              </div>
              <div className="text-emerald-700 dark:text-emerald-400">
                Leaf expires: {new Date(lcExisting.cert_expires_at).toLocaleDateString()}
                {" · "}CA expires: {new Date(lcExisting.ca_expires_at).toLocaleDateString()}
              </div>
              {lcExisting.hosts.length > 0 && (
                <div className="text-emerald-700 dark:text-emerald-400 mt-1">
                  Hosts:{" "}
                  <span className="font-mono break-all">
                    {lcExisting.hosts.join(", ")}
                  </span>
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
                Bundle contains <code className="font-mono">ca.pem</code>,
                <code className="font-mono"> install-linux.sh</code>,
                <code className="font-mono"> install-windows.ps1</code>, and
                <code className="font-mono"> install-android.txt</code>.
                Verify the fingerprint above before trusting the certificate.
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
                automatically.
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
          </div>
        </Disclosure>

        <Disclosure
          title="Manual certificate"
          subtitle="Point the server at certificate and key files you already have."
          badge={
            sslEnabled && sslCertPath && !lcExisting && !leExisting
              ? "In use"
              : null
          }
          tone="gray"
          open={openMethod === "manual"}
          onToggle={() =>
            setOpenMethod(openMethod === "manual" ? null : "manual")
          }
        >
          <p className="text-xs text-gray-500 dark:text-gray-400 mb-3">
            Use this if you have a certificate from another provider (corporate
            CA, Cloudflare Origin, etc.). Both files must be PEM-encoded and
            readable by the server process.
          </p>
          <div className="space-y-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Certificate Path (.crt / .pem)
              </label>
              <input
                type="text"
                value={manualCert}
                onChange={(e) => setManualCert(e.target.value)}
                placeholder="/etc/ssl/certs/my-cert.pem"
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                disabled={manualSaving}
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Private Key Path (.key / .pem)
              </label>
              <input
                type="text"
                value={manualKey}
                onChange={(e) => setManualKey(e.target.value)}
                placeholder="/etc/ssl/private/my-key.pem"
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                disabled={manualSaving}
              />
            </div>
            <button
              onClick={handleSaveManual}
              disabled={manualSaving}
              className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
            >
              {manualSaving ? "Saving…" : "Save certificate paths"}
            </button>
          </div>
        </Disclosure>
      </div>
    </section>
  );
}

// ── Disclosure component ────────────────────────────────────────────────
//
// Plain `<details>`-style accordion item.  Each provisioning method is
// wrapped in one of these so the panel collapses to three thin headers
// when nothing is being configured.

interface DisclosureProps {
  title: string;
  subtitle: string;
  badge: string | null;
  tone: "blue" | "emerald" | "gray";
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
}

function Disclosure({
  title,
  subtitle,
  badge,
  tone,
  open,
  onToggle,
  children,
}: DisclosureProps) {
  const badgeClass = {
    blue: "bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300",
    emerald:
      "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300",
    gray: "bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
  }[tone];

  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={onToggle}
        className="w-full flex items-center justify-between gap-3 px-4 py-3 text-left hover:bg-gray-50 dark:hover:bg-gray-700/40 transition-colors"
        aria-expanded={open}
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold text-gray-800 dark:text-gray-200">
              {title}
            </span>
            {badge && (
              <span className={`text-[10px] uppercase tracking-wide px-2 py-0.5 rounded-full ${badgeClass}`}>
                {badge}
              </span>
            )}
          </div>
          <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5 truncate">
            {subtitle}
          </p>
        </div>
        <svg
          className={`w-4 h-4 text-gray-400 flex-shrink-0 transition-transform ${open ? "rotate-180" : ""}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      {open && (
        <div className="px-4 pb-4 pt-1 border-t border-gray-200 dark:border-gray-700">
          {children}
        </div>
      )}
    </div>
  );
}
