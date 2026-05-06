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

type SslMode = "skip" | "manual" | "letsencrypt";

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

  const isDone = saved || mode === "skip" || !!leSuccess;

  return (
    <>
      <div className="flex flex-col items-center mb-6">
        <div className="w-14 h-14 bg-green-100 dark:bg-green-900/40 rounded-full flex items-center justify-center mb-3">
          <svg className="w-7 h-7 text-green-600 dark:text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
          </svg>
        </div>
        <h2 className="text-xl font-bold">SSL / TLS</h2>
        <p className="text-sm text-gray-500 dark:text-gray-400 text-center mt-1">
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
            ["manual", "Manual certificate", "I already have a certificate and key file."],
          ] as const
        ).map(([value, label, desc]) => (
          <label
            key={value}
            className={`flex items-start gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
              mode === value
                ? "border-blue-500 bg-blue-50 dark:bg-blue-900/20"
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
              }}
              className="mt-1 accent-blue-600"
            />
            <div>
              <span className="font-medium text-sm">{label}</span>
              <p className="text-xs text-gray-500 dark:text-gray-400">{desc}</p>
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
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
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
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <button
            onClick={handleSaveManual}
            disabled={saving}
            className="w-full bg-blue-600 text-white py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm font-medium"
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
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              disabled={leProvisioning}
            />
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
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
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              disabled={leProvisioning}
            />
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
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
                className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                disabled={leProvisioning}
              />
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Default 80. Forward port 80 → here if the server runs unprivileged.
              </p>
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
                Use staging directory
                <span className="block text-xs text-gray-500 dark:text-gray-400">
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
            className="w-full bg-blue-600 text-white py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm font-medium"
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
          className="text-gray-500 hover:text-gray-700 dark:hover:text-gray-300 text-sm"
          disabled={leProvisioning}
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
          className="bg-blue-600 text-white px-6 py-2 rounded-md hover:bg-blue-700 text-sm font-medium disabled:opacity-50"
          disabled={leProvisioning}
        >
          {isDone ? "Continue →" : "Skip →"}
        </button>
      </div>
    </>
  );
}
