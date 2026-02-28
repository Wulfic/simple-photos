import { useState, useEffect } from "react";
import { api } from "../../api/client";

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
  const [sslMode, setSslMode] = useState<"manual" | "letsencrypt">("manual");
  const [leDomain, setLeDomain] = useState("");
  const [leEmail, setLeEmail] = useState("");
  const [leStaging, setLeStaging] = useState(false);
  const [leGenerating, setLeGenerating] = useState(false);
  const [leGenerated, setLeGenerated] = useState(false);
  const [leError, setLeError] = useState<string | null>(null);
  const [leStatusLog, setLeStatusLog] = useState<string[]>([]);

  useEffect(() => {
    loadSslSettings();
  }, []);

  async function loadSslSettings() {
    try {
      const res = await api.admin.getSsl();
      setSslEnabled(res.enabled);
      setSslCertPath(res.cert_path ?? "");
      setSslKeyPath(res.key_path ?? "");
      setSslLoaded(true);
    } catch {
      // Not admin or SSL endpoints not available — silently skip
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
    } catch (err: any) {
      setError(err.message);
    } finally {
      setSslSaving(false);
    }
  }

  async function handleGenerateLeCert() {
    if (!leDomain.trim() || !leEmail.trim()) {
      setError("Domain and e-mail are both required.");
      return;
    }
    setLeGenerating(true);
    setLeError(null);
    setError("");
    setLeStatusLog([
      "Starting Let's Encrypt certificate generation...",
      `Domain: ${leDomain.trim()}`,
      `Contact: ${leEmail.trim()}`,
      leStaging ? "Mode: Staging (testing)" : "Mode: Production",
      "Creating ACME account...",
    ]);
    try {
      const res = await api.admin.generateLetsEncrypt({
        domain: leDomain.trim(),
        email: leEmail.trim(),
        staging: leStaging,
      });
      setLeGenerated(true);
      setSslEnabled(true);
      setSslCertPath(res.cert_path);
      setSslKeyPath(res.key_path);
      setSuccess(res.message);
      setLeStatusLog((prev) => [...prev, "Certificate generated successfully!", `Cert: ${res.cert_path}`, `Key: ${res.key_path}`]);
    } catch (err: any) {
      let msg = err.message || "Certificate generation failed";
      // Provide a more descriptive error when fetch itself fails
      if (msg === "Failed to fetch" || msg === "NetworkError when attempting to fetch resource.") {
        msg = "Could not reach the server. The request may have timed out, or the server encountered an error during certificate generation. " +
          "Ensure the server is running, the domain resolves to this server, and port 80 is accessible.";
      }
      setLeError(msg);
      setError(msg);
      setLeStatusLog((prev) => [...prev, `ERROR: ${msg}`]);
    } finally {
      setLeGenerating(false);
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

      {/* Mode tabs */}
      {sslEnabled && (
        <div className="space-y-4">
          <div className="flex gap-2 mb-3">
            <button
              onClick={() => setSslMode("manual")}
              className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                sslMode === "manual"
                  ? "bg-blue-600 text-white"
                  : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600"
              }`}
            >
              Manual Certificate
            </button>
            <button
              onClick={() => setSslMode("letsencrypt")}
              className={`px-3 py-1.5 rounded-md text-sm font-medium transition-colors ${
                sslMode === "letsencrypt"
                  ? "bg-green-600 text-white"
                  : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600"
              }`}
            >
              Let's Encrypt
            </button>
          </div>

          {/* Manual cert fields */}
          {sslMode === "manual" && (
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

          {/* Let's Encrypt */}
          {sslMode === "letsencrypt" && !leGenerated && (
            <div className="space-y-3">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Domain Name
                </label>
                <input
                  type="text"
                  value={leDomain}
                  onChange={(e) => setLeDomain(e.target.value)}
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
                  value={leEmail}
                  onChange={(e) => setLeEmail(e.target.value)}
                  placeholder="you@example.com"
                  className="w-full border rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                />
              </div>
              <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
                <input
                  type="checkbox"
                  checked={leStaging}
                  onChange={(e) => setLeStaging(e.target.checked)}
                  className="accent-blue-600"
                />
                Use staging environment (testing only)
              </label>
              <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg p-3 text-xs text-amber-700 dark:text-amber-400">
                Port 80 must be available and the domain must resolve to this server.
              </div>
              <button
                onClick={handleGenerateLeCert}
                disabled={leGenerating}
                className="bg-green-600 text-white px-4 py-2 rounded-md hover:bg-green-700 disabled:opacity-50 text-sm"
              >
                {leGenerating ? (
                  <span className="flex items-center gap-2">
                    <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                    Generating…
                  </span>
                ) : (
                  "Generate Let's Encrypt Certificate"
                )}
              </button>

              {/* Status log — real-time feedback during generation */}
              {leStatusLog.length > 0 && (
                <div className="mt-3 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-lg p-3 max-h-48 overflow-y-auto font-mono text-xs space-y-0.5">
                  {leStatusLog.map((line, i) => (
                    <div
                      key={i}
                      className={
                        line.startsWith("ERROR")
                          ? "text-red-600 dark:text-red-400 font-semibold"
                          : line.includes("successfully")
                            ? "text-green-600 dark:text-green-400"
                            : "text-gray-600 dark:text-gray-400"
                      }
                    >
                      {line}
                    </div>
                  ))}
                </div>
              )}

              {/* Persistent error banner */}
              {leError && !leGenerating && (
                <div className="mt-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-3 text-sm text-red-700 dark:text-red-400">
                  <strong>Error:</strong> {leError}
                  <p className="text-xs mt-1 text-red-500 dark:text-red-500">
                    Common causes: port 80 blocked, domain doesn't resolve to this server, or rate limited by Let's Encrypt.
                  </p>
                </div>
              )}
            </div>
          )}

          {/* LE success */}
          {sslMode === "letsencrypt" && leGenerated && (
            <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-4 flex items-start gap-2">
              <svg className="w-5 h-5 text-green-600 mt-0.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              <div>
                <p className="text-sm font-medium text-green-700 dark:text-green-300">Certificate generated!</p>
                <p className="text-xs text-green-600 dark:text-green-400 mt-1">
                  Restart the server to start serving HTTPS on {leDomain}.
                </p>
              </div>
            </div>
          )}
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
    </section>
  );
}
