/** Wizard step — configure SSL/TLS (manual cert, Let's Encrypt, or skip).
 *
 * Used by both the **primary** and **backup** server flows.  The same
 * component is mounted from `Welcome.tsx` regardless of `serverRole`
 * because TLS is a transport concern, not a role concern — every server
 * benefits from HTTPS, and `[tls] redirect_http = true` ensures plain
 * HTTP requests are 301-upgraded automatically once TLS is enabled.
 */
import { useState } from "react";
import { api } from "../../api/client";
import { downloadRaw } from "../../api/core";
import type { WizardStep, ServerRole } from "./types";
import { getErrorMessage } from "../../utils/formatters";

export interface SslStepProps {
  setStep: (step: WizardStep) => void;
  setError: (msg: string) => void;
  error: string;
  /**
   * Optional — when `"backup"`, the "Continue" / "Skip" button advances
   * directly to `"complete"` (backup servers don't have a `"users"` step).
   * Defaults to primary-flow behaviour.
   */
  serverRole?: ServerRole;
}

type SslMode = "skip" | "manual" | "letsencrypt" | "local_ca";

export default function SslStep({ setStep, setError, error, serverRole }: SslStepProps) {
  const [mode, setMode] = useState<SslMode>("skip");

  // Manual fields
  const [certPath, setCertPath] = useState("");
  const [keyPath, setKeyPath] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Let's Encrypt fields
  const [leDomain, setLeDomain] = useState("");
  const [leEmail, setLeEmail] = useState("");
  const [leAgreeTos, setLeAgreeTos] = useState(false);
  const [leStaging, setLeStaging] = useState(false);
  const [leChallengePort, setLeChallengePort] = useState("80");
  const [leProvisioning, setLeProvisioning] = useState(false);
  const [leSuccess, setLeSuccess] = useState<{ domain: string; staging: boolean } | null>(null);

  // Self-signed local CA fields
  const [lcLabel, setLcLabel] = useState("");
  const [lcExtraHosts, setLcExtraHosts] = useState("");
  const [lcGenerating, setLcGenerating] = useState(false);
  const [lcDownloading, setLcDownloading] = useState(false);
  const [lcSuccess, setLcSuccess] = useState<
    { fingerprint: string; hosts: string[] } | null
  >(null);

  async function handleSaveManual() {
    if (!certPath.trim() || !keyPath.trim()) {
      setError("Both certificate path and key path are required.");
      return;
    }
    setSaving(true);
    setError("");
    try {
      await api.admin.updateSsl({
        enabled: true,
        cert_path: certPath.trim(),
        key_path: keyPath.trim(),
      });
      setSaved(true);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to save SSL configuration"));
    } finally {
      setSaving(false);
    }
  }

  async function handleProvisionLetsEncrypt() {
    setError("");

    // Front-end-side sanity checks — server re-validates everything.
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
      // First a dry-run — surfaces "domain malformed" / "bad email" without
      // burning a Let's Encrypt rate-limit slot.
      await api.admin.provisionLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        agree_tos: leAgreeTos,
        staging: leStaging,
        challenge_port: port,
        dry_run: true,
      });

      // Real provisioning.  This contacts the CA, spins up a temporary
      // HTTP-01 listener on `port`, polls the order, and writes the cert.
      // Can take 30-90 s in production.
      const res = await api.admin.provisionLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        agree_tos: leAgreeTos,
        staging: leStaging,
        challenge_port: port,
        dry_run: false,
      });
      setLeSuccess({ domain: res.domain, staging: res.staging });
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to provision Let's Encrypt certificate"));
    } finally {
      setLeProvisioning(false);
    }
  }

  /** Generate a self-signed root CA + leaf cert for LAN / offline use.
   *
   * Mirrors the flow in `SslSettings.tsx` but trimmed for the wizard:
   * a dry-run validates the operator-supplied label / extra hosts, then
   * a real generate writes the PEM files and a downloadable bundle.
   * The success block exposes a one-click "Download install bundle"
   * button so the operator can immediately push the CA to their
   * devices and avoid browser warnings on first connect.
   */
  async function handleGenerateLocalCa() {
    setError("");

    const extras = lcExtraHosts
      .split(/[\s,]+/)
      .map((h) => h.trim())
      .filter((h) => h.length > 0);

    setLcGenerating(true);
    try {
      // Validate inputs first — cheap and avoids touching the filesystem
      // if the operator typed a bad host.
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
      setLcSuccess({
        fingerprint: res.fingerprint_sha256,
        hosts: res.hosts,
      });
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to generate local CA"));
    } finally {
      setLcGenerating(false);
    }
  }

  /** Authenticated bundle download — the endpoint requires the admin
   *  bearer token, so a plain `<a href>` won't work; we fetch the bytes,
   *  wrap them in a Blob, and trigger a synthetic anchor click. */
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
      // Safari needs a beat before revoke or the download aborts.
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to download CA install bundle"));
    } finally {
      setLcDownloading(false);
    }
  }

  const isDone = saved || mode === "skip" || !!leSuccess || !!lcSuccess;

  return (
    <>
      <div className="flex flex-col items-center mb-6">
        <div className="w-14 h-14 bg-green-100 dark:bg-green-900/40 rounded-full flex items-center justify-center mb-3">
          <svg className="w-7 h-7 text-green-600 dark:text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
          </svg>
        </div>
        <h2 className="text-xl font-bold">SSL / TLS</h2>
        <p className="text-sm text-gray-700 dark:text-gray-400 text-center mt-1">
          Secure your server with HTTPS.  When TLS is on, plain-HTTP requests
          are automatically redirected to HTTPS.  You can skip this and
          configure it later in Settings.
        </p>
      </div>

      {/* Mode selector */}
      <div className="space-y-2 mb-5">
        {(
          [
            ["skip", "Skip for now",    "Run on plain HTTP (can be configured later)."],
            ["letsencrypt", "Let's Encrypt (recommended)", "Automatically issue a free trusted certificate. Requires a public domain and port 80 reachable."],
            ["local_ca", "Self-signed local CA", "Best for LAN-only or offline servers. Generates a private root CA you install on your devices once — no warnings, no public DNS required."],
            ["manual", "Manual certificate", "I already have a certificate and key file."],
          ] as const
        ).map(([value, label, desc]) => (
          <label
            key={value}
            className={`flex items-start gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
              mode === value
                ? "border-accent-500 bg-accent-50 dark:bg-accent-900/20"
                : "border-gray-200 dark:border-gray-700 hover:border-gray-300 dark:hover:border-gray-600"
            }`}
          >
            <input
              type="radio"
              name="ssl-mode"
              value={value}
              checked={mode === value}
              onChange={() => {
                setMode(value);
                setError("");
                setSaved(false);
                setLeSuccess(null);
                setLcSuccess(null);
              }}
              className="mt-1 accent-indigo-600"
            />
            <div>
              <span className="font-medium text-sm">{label}</span>
              <p className="text-xs text-gray-700 dark:text-gray-400">{desc}</p>
            </div>
          </label>
        ))}
      </div>

      {/* ── Manual cert form ───────────────────────────────────────── */}
      {mode === "manual" && !saved && (
        <div className="space-y-3 mb-5 bg-gray-50 dark:bg-gray-700/40 rounded-lg p-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Certificate Path (.crt / .pem)
            </label>
            <input
              type="text"
              value={certPath}
              onChange={(e) => setCertPath(e.target.value)}
              placeholder="/etc/ssl/certs/my-cert.pem"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Private Key Path (.key / .pem)
            </label>
            <input
              type="text"
              value={keyPath}
              onChange={(e) => setKeyPath(e.target.value)}
              placeholder="/etc/ssl/private/my-key.pem"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
            />
          </div>
          <button
            onClick={handleSaveManual}
            disabled={saving}
            className="btn btn-primary btn-md w-full"
          >
            {saving ? "Saving…" : "Save & Enable TLS"}
          </button>
        </div>
      )}

      {/* ── Manual saved confirmation ──────────────────────────────── */}
      {mode === "manual" && saved && (
        <div className="mb-5 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-4 flex items-center gap-2">
          <svg className="w-5 h-5 text-green-600" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <span className="text-sm font-medium text-green-700 dark:text-green-300">
            TLS enabled. Restart the server to serve HTTPS.
          </span>
        </div>
      )}

      {/* ── Let's Encrypt form ────────────────────────────────────── */}
      {mode === "letsencrypt" && !leSuccess && (
        <div className="space-y-3 mb-5 bg-gray-50 dark:bg-gray-700/40 rounded-lg p-4">
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
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
              disabled={leProvisioning}
            />
            <p className="text-xs text-gray-700 dark:text-gray-400 mt-1">
              Must resolve to this server's public IP. Wildcards and raw IPs are not supported.
            </p>
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
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
              disabled={leProvisioning}
            />
            <p className="text-xs text-gray-700 dark:text-gray-400 mt-1">
              Used by Let's Encrypt for renewal reminders only.
            </p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                HTTP-01 challenge port
              </label>
              <input
                type="number"
                min={1}
                max={65535}
                value={leChallengePort}
                onChange={(e) => setLeChallengePort(e.target.value)}
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
                disabled={leProvisioning}
              />
              <p className="text-xs text-gray-700 dark:text-gray-400 mt-1">
                Default 80. Forward port 80 → here if the server runs unprivileged.
              </p>
            </div>
            <label className="flex items-start gap-2 mt-6 text-sm">
              <input
                type="checkbox"
                checked={leStaging}
                onChange={(e) => setLeStaging(e.target.checked)}
                className="mt-0.5 accent-indigo-600"
                disabled={leProvisioning}
              />
              <span className="text-gray-700 dark:text-gray-300">
                Use staging directory
                <span className="block text-xs text-gray-700 dark:text-gray-400">
                  Test only — issues untrusted certs with relaxed rate limits.
                </span>
              </span>
            </label>
          </div>
          <label className="flex items-start gap-2 text-sm">
            <input
              type="checkbox"
              checked={leAgreeTos}
              onChange={(e) => setLeAgreeTos(e.target.checked)}
              className="mt-0.5 accent-indigo-600"
              disabled={leProvisioning}
            />
            <span className="text-gray-700 dark:text-gray-300">
              I agree to the{" "}
              <a
                href="https://letsencrypt.org/repository/"
                target="_blank"
                rel="noopener noreferrer"
                className="text-accent-600 dark:text-accent-400 underline"
              >
                Let's Encrypt Subscriber Agreement
              </a>
              .
            </span>
          </label>
          <button
            onClick={handleProvisionLetsEncrypt}
            disabled={leProvisioning}
            className="btn btn-primary btn-md w-full"
          >
            {leProvisioning
              ? "Requesting certificate from Let's Encrypt…"
              : "Issue certificate"}
          </button>
        </div>
      )}

      {/* ── Let's Encrypt success ─────────────────────────────────── */}
      {mode === "letsencrypt" && leSuccess && (
        <div className="mb-5 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-4">
          <div className="flex items-center gap-2 mb-1">
            <svg className="w-5 h-5 text-green-600" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span className="text-sm font-medium text-green-700 dark:text-green-300">
              Let's Encrypt certificate issued for {leSuccess.domain}
              {leSuccess.staging ? " (staging)" : ""}.
            </span>
          </div>
          <p className="text-xs text-green-700 dark:text-green-400 ml-7">
            Restart the server to begin serving HTTPS.
          </p>
        </div>
      )}
      {/* ── Self-signed local CA form ────────────────────── */}
      {mode === "local_ca" && !lcSuccess && (
        <div className="space-y-3 mb-5 bg-gray-50 dark:bg-gray-700/40 rounded-lg p-4">
          <p className="text-xs text-gray-600 dark:text-gray-400">
            Creates a private root CA + leaf certificate covering
            <span className="font-mono">&nbsp;localhost</span>, this server's hostname,
            and any LAN IPs detected automatically. Install the bundled CA on
            each device once and Simple Photos will load with no browser warnings.
          </p>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Friendly label <span className="text-gray-600 dark:text-gray-400 font-normal">(optional)</span>
            </label>
            <input
              type="text"
              value={lcLabel}
              onChange={(e) => setLcLabel(e.target.value)}
              placeholder="Simple Photos — home NAS"
              maxLength={128}
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
              disabled={lcGenerating}
            />
            <p className="text-xs text-gray-700 dark:text-gray-400 mt-1">
              Shown in your browser / OS as the certificate’s common name.
            </p>
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Extra hostnames / IPs <span className="text-gray-600 dark:text-gray-400 font-normal">(optional)</span>
            </label>
            <input
              type="text"
              value={lcExtraHosts}
              onChange={(e) => setLcExtraHosts(e.target.value)}
              placeholder="photos.lan, 192.168.1.50, nas.local"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent-500"
              disabled={lcGenerating}
            />
            <p className="text-xs text-gray-700 dark:text-gray-400 mt-1">
              Comma- or space-separated. Up to 32 entries. Each must be a DNS
              label or an IP address (no wildcards).
            </p>
          </div>
          <button
            onClick={handleGenerateLocalCa}
            disabled={lcGenerating}
            className="btn btn-primary btn-md w-full"
          >
            {lcGenerating ? "Generating local CA…" : "Generate local CA"}
          </button>
        </div>
      )}

      {/* ── Self-signed local CA success ───────────────────── */}
      {mode === "local_ca" && lcSuccess && (
        <div className="mb-5 bg-emerald-50 dark:bg-emerald-900/20 border border-emerald-200 dark:border-emerald-800 rounded-lg p-4 space-y-3">
          <div className="flex items-center gap-2">
            <svg className="w-5 h-5 text-emerald-600" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span className="text-sm font-medium text-emerald-700 dark:text-emerald-300">
              Local CA generated. Restart the server to serve HTTPS.
            </span>
          </div>
          <div className="text-xs text-emerald-800 dark:text-emerald-300 ml-7 space-y-1">
            <div>
              <span className="font-semibold">Hosts covered:</span>{" "}
              <span className="font-mono break-all">
                {lcSuccess.hosts.join(", ")}
              </span>
            </div>
            <div>
              <span className="font-semibold">SHA-256 fingerprint:</span>{" "}
              <span className="font-mono break-all">{lcSuccess.fingerprint}</span>
            </div>
          </div>
          <button
            onClick={handleDownloadLocalCaBundle}
            disabled={lcDownloading}
            className="w-full bg-emerald-600 text-white py-2 rounded-md hover:bg-emerald-700 disabled:opacity-50 text-sm font-medium"
          >
            {lcDownloading
              ? "Preparing download…"
              : "Download CA install bundle (.zip)"}
          </button>
          <p className="text-xs text-emerald-700 dark:text-emerald-400 ml-1">
            The bundle includes <code>install-linux.sh</code>,{" "}
            <code>install-windows.ps1</code>, and{" "}
            <code>install-android.txt</code>. Run the appropriate script on
            each device that should trust this server.
          </p>
        </div>
      )}
      {error && (
        <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">
          {error}
        </p>
      )}

      {/* Navigation */}
      <div className="flex justify-between mt-2">
        <button
          onClick={() => {
            setError("");
            setStep("storage");
          }}
          className="text-gray-700 dark:text-gray-500 hover:text-gray-700 dark:hover:text-gray-300 text-sm"
          disabled={leProvisioning || lcGenerating}
        >
          ← Back
        </button>
        <button
          onClick={() => {
            setError("");
            // Backup servers skip the "users" step — there are no local
            // users to create on a backup instance.
            setStep(serverRole === "backup" ? "complete" : "users");
          }}
          className="btn btn-primary btn-md"
          disabled={leProvisioning || lcGenerating}
        >
          {isDone ? "Continue →" : "Skip →"}
        </button>
      </div>
    </>
  );
}
