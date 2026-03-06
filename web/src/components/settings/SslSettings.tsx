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
    </section>
  );
}
