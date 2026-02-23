import { useState } from "react";
import { api } from "../../api/client";

export interface SslStepProps {
  setStep: (step: any) => void;
  setError: (msg: string) => void;
  error: string;
}

type SslMode = "skip" | "manual" | "letsencrypt";

export default function SslStep({ setStep, setError, error }: SslStepProps) {
  const [mode, setMode] = useState<SslMode>("skip");

  // Manual fields
  const [certPath, setCertPath] = useState("");
  const [keyPath, setKeyPath] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Let's Encrypt fields
  const [domain, setDomain] = useState("");
  const [email, setEmail] = useState("");
  const [staging, setStaging] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [generated, setGenerated] = useState(false);
  const [generatedPaths, setGeneratedPaths] = useState<{
    cert: string;
    key: string;
  } | null>(null);

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
    } catch (err: any) {
      setError(err.message || "Failed to save SSL configuration");
    } finally {
      setSaving(false);
    }
  }

  async function handleGenerateLe() {
    if (!domain.trim()) {
      setError("Domain name is required (e.g. photos.example.com).");
      return;
    }
    if (!email.trim() || !email.includes("@")) {
      setError("A valid contact e-mail is required.");
      return;
    }
    setGenerating(true);
    setError("");
    try {
      const res = await api.admin.generateLetsEncrypt({
        domain: domain.trim(),
        email: email.trim(),
        staging,
      });
      setGenerated(true);
      setGeneratedPaths({ cert: res.cert_path, key: res.key_path });
    } catch (err: any) {
      setError(err.message || "Let's Encrypt certificate generation failed");
    } finally {
      setGenerating(false);
    }
  }

  const isDone = saved || generated || mode === "skip";

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
          Secure your server with HTTPS.  You can skip this and configure it later in Settings.
        </p>
      </div>

      {/* Mode selector */}
      <div className="space-y-2 mb-5">
        {(
          [
            ["skip", "Skip for now",    "Run on plain HTTP (can be configured later)."],
            ["letsencrypt", "Let\u2019s Encrypt", "Automatically obtain a free certificate."],
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
                setGenerated(false);
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

      {/* ── Let's Encrypt form ─────────────────────────────────────── */}
      {mode === "letsencrypt" && !generated && (
        <div className="space-y-3 mb-5 bg-gray-50 dark:bg-gray-700/40 rounded-lg p-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Domain Name
            </label>
            <input
              type="text"
              value={domain}
              onChange={(e) => setDomain(e.target.value)}
              placeholder="photos.example.com"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Contact E-mail
            </label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
              className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
            <input
              type="checkbox"
              checked={staging}
              onChange={(e) => setStaging(e.target.checked)}
              className="accent-blue-600"
            />
            Use staging environment (for testing — cert won't be trusted by browsers)
          </label>

          <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-700 dark:text-amber-400">
            <strong>Requirements:</strong>
            <ul className="list-disc list-inside mt-1 space-y-0.5">
              <li>Port 80 must be available on this machine.</li>
              <li>The domain must point to this server's public IP.</li>
              <li>This process may take up to 60 seconds.</li>
            </ul>
          </div>

          <button
            onClick={handleGenerateLe}
            disabled={generating}
            className="w-full bg-green-600 text-white py-2 rounded-md hover:bg-green-700 disabled:opacity-50 text-sm font-medium"
          >
            {generating ? (
              <span className="flex items-center justify-center gap-2">
                <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                Generating certificate…
              </span>
            ) : (
              "Generate Certificate"
            )}
          </button>
        </div>
      )}

      {/* ── Let's Encrypt success ──────────────────────────────────── */}
      {mode === "letsencrypt" && generated && generatedPaths && (
        <div className="mb-5 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-4">
          <div className="flex items-center gap-2 mb-2">
            <svg className="w-5 h-5 text-green-600" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span className="text-sm font-semibold text-green-700 dark:text-green-300">
              Certificate generated!
            </span>
          </div>
          <p className="text-xs text-green-600 dark:text-green-400 mb-1">
            Certificate: <code className="bg-green-100 dark:bg-green-800 px-1 rounded">{generatedPaths.cert}</code>
          </p>
          <p className="text-xs text-green-600 dark:text-green-400">
            Key: <code className="bg-green-100 dark:bg-green-800 px-1 rounded">{generatedPaths.key}</code>
          </p>
          <p className="text-xs text-gray-500 dark:text-gray-400 mt-2">
            TLS has been enabled. Restart the server to serve HTTPS.
          </p>
        </div>
      )}

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
        >
          ← Back
        </button>
        <button
          onClick={() => {
            setError("");
            setStep("encryption");
          }}
          className="bg-blue-600 text-white px-6 py-2 rounded-md hover:bg-blue-700 text-sm font-medium"
        >
          {isDone ? "Continue →" : "Skip →"}
        </button>
      </div>
    </>
  );
}
