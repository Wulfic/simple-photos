/**
 * Diagnostics dashboard (admin only) — server metrics, audit logs,
 * client logs, and an external API reference section for monitoring
 * integrations (Uptime Kuma, Grafana, etc.).
 */
import { useState, useEffect, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import {
  api,
  type DiagnosticsResponse,
  type DiagnosticsResponseUnion,
  type DiagnosticsConfig,
} from "../api/client";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { useIsAdmin } from "../hooks/useIsAdmin";
import OverviewTab from "../components/diagnostics/OverviewTab";
import ServerLogsTab from "../components/diagnostics/ServerLogsTab";
import ClientLogsTab from "../components/diagnostics/ClientLogsTab";
import { diagnosticLogger } from "../utils/diagnosticLogger";
import { formatUptime } from "../utils/formatters";

type Tab = "overview" | "server-logs" | "client-logs";

// ── External API Reference Section ───────────────────────────────────────────

function ExternalApiSection() {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const baseUrl = window.location.origin;

  const endpoints = [
    {
      id: "health",
      method: "GET",
      path: "/api/external/diagnostics/health",
      description: "Lightweight health check — uptime, memory, CPU, disk, DB ping",
    },
    {
      id: "full",
      method: "GET",
      path: "/api/external/diagnostics",
      description: "Full server metrics — database, storage, users, photos, audit, backups",
    },
    {
      id: "storage",
      method: "GET",
      path: "/api/external/diagnostics/storage",
      description: "Storage-focused — disk usage, photo sizes, database size",
    },
    {
      id: "audit",
      method: "GET",
      path: "/api/external/diagnostics/audit",
      description: "Audit & security — login events, failures, client logs",
    },
  ];

  const copyToClipboard = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopied(id);
    setTimeout(() => setCopied(null), 2000);
  };

  return (
    <div className="mb-4 bg-white dark:bg-gray-800 rounded-lg shadow">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center justify-between p-5 text-left"
      >
        <div className="flex items-center gap-3">
          <div className="inline-flex items-center justify-center w-8 h-8 rounded-md bg-indigo-100 dark:bg-indigo-900/30">
            <svg
              className="w-4 h-4 text-indigo-600 dark:text-indigo-400"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
              />
            </svg>
          </div>
          <div>
            <h2 className="text-base font-semibold text-gray-900 dark:text-white">
              External API
            </h2>
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
              Integrate diagnostics into your monitoring or other servers via HTTP Basic Auth
            </p>
          </div>
        </div>
        <svg
          className={`w-5 h-5 text-gray-400 transition-transform ${
            expanded ? "rotate-180" : ""
          }`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>

      {expanded && (
        <div className="px-5 pb-5 border-t border-gray-100 dark:border-gray-700 pt-4">
          {/* Auth note */}
          <div className="mb-4 p-3 bg-amber-50 dark:bg-amber-900/20 rounded-md border border-amber-200 dark:border-amber-800">
            <p className="text-xs text-amber-700 dark:text-amber-400">
              <strong>Authentication:</strong> All endpoints require HTTP Basic Auth
              with an admin account's username and password.
              External API endpoints work regardless of the diagnostics toggle above.
            </p>
          </div>

          {/* Endpoints */}
          <div className="space-y-3">
            {endpoints.map((ep) => {
              const curlCmd = `curl -s -u "admin:password" ${baseUrl}${ep.path} | jq .`;
              return (
                <div
                  key={ep.id}
                  className="p-3 bg-gray-50 dark:bg-gray-900/50 rounded-lg border border-gray-200 dark:border-gray-700"
                >
                  <div className="flex items-center gap-2 mb-1">
                    <span className="px-1.5 py-0.5 text-[10px] font-bold bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400 rounded">
                      {ep.method}
                    </span>
                    <code className="text-sm font-mono text-gray-900 dark:text-gray-100">
                      {ep.path}
                    </code>
                  </div>
                  <p className="text-xs text-gray-500 dark:text-gray-400 mb-2">
                    {ep.description}
                  </p>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 text-xs font-mono text-gray-600 dark:text-gray-300 bg-gray-100 dark:bg-gray-800 px-2 py-1.5 rounded overflow-x-auto whitespace-nowrap">
                      {curlCmd}
                    </code>
                    <button
                      onClick={() => copyToClipboard(curlCmd, ep.id)}
                      className="shrink-0 p-1.5 text-gray-400 hover:text-gray-600 dark:hover:text-gray-200 transition-colors"
                      title="Copy curl command"
                    >
                      {copied === ep.id ? (
                        <svg
                          className="w-4 h-4 text-green-500"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M5 13l4 4L19 7"
                          />
                        </svg>
                      ) : (
                        <svg
                          className="w-4 h-4"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                          />
                        </svg>
                      )}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

// ══════════════════════════════════════════════════════════════════════════
// Main Diagnostics Page Component
// ══════════════════════════════════════════════════════════════════════════

export default function Diagnostics() {
  const navigate = useNavigate();
  const isAdmin = useIsAdmin();
  const [activeTab, setActiveTab] = useState<Tab>("overview");
  const [metrics, setMetrics] = useState<DiagnosticsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [autoRefresh, setAutoRefresh] = useState(false);
  const refreshTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  // ── Diagnostics config state ──
  const [config, setConfig] = useState<DiagnosticsConfig | null>(null);
  const [configLoading, setConfigLoading] = useState(true);
  const [toggling, setToggling] = useState(false);
  const [disabledInfo, setDisabledInfo] = useState<{
    version: string;
    uptime_seconds: number;
    started_at: string;
  } | null>(null);

  // ── Backup-mode detection (hide client diagnostics on backup servers) ──
  const [isBackupMode, setIsBackupMode] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const mode = await api.backup.getMode();
        setIsBackupMode(mode.mode === "backup");
      } catch {
        // Not admin or endpoint unavailable — default to primary
      }
    })();
  }, []);

  // Redirect non-admins
  useEffect(() => {
    if (!isAdmin) {
      navigate("/settings", { replace: true });
    }
  }, [isAdmin, navigate]);

  // Fetch diagnostics config on mount
  const fetchConfig = useCallback(async () => {
    try {
      const data = await api.diagnostics.getConfig();
      setConfig(data);

      // Sync client-side diagnostic logger with server config
      if (data.client_diagnostics_enabled) {
        diagnosticLogger.enable();
      } else {
        diagnosticLogger.disable();
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to load diagnostics config");
    } finally {
      setConfigLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchConfig();
  }, [fetchConfig]);

  const fetchMetrics = useCallback(async () => {
    try {
      const data: DiagnosticsResponseUnion = await api.diagnostics.getMetrics();
      if (data.enabled) {
        setMetrics(data as DiagnosticsResponse);
        setDisabledInfo(null);
      } else {
        setMetrics(null);
        // Store basic info from disabled response
        const disabled = data as import("../api/client").DisabledDiagnosticsResponse;
        setDisabledInfo(disabled.server);
      }
      setError("");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to load diagnostics");
    } finally {
      setLoading(false);
    }
  }, []);

  // Fetch metrics after config is loaded and diagnostics is enabled
  useEffect(() => {
    if (config !== null) {
      fetchMetrics();
    }
  }, [config, fetchMetrics]);

  // Auto-refresh every 10s when enabled
  useEffect(() => {
    if (autoRefresh && config?.diagnostics_enabled) {
      refreshTimer.current = setInterval(fetchMetrics, 10000);
    }
    return () => {
      if (refreshTimer.current) clearInterval(refreshTimer.current);
    };
  }, [autoRefresh, config?.diagnostics_enabled, fetchMetrics]);

  // ── Toggle handlers ──

  const toggleDiagnostics = useCallback(
    async (enabled: boolean) => {
      setToggling(true);
      try {
        const updated = await api.diagnostics.updateConfig({
          diagnostics_enabled: enabled,
        });
        setConfig(updated);
        // Refetch metrics to get full or disabled response
        setLoading(true);
        await fetchMetrics();
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : "Failed to update diagnostics config");
      } finally {
        setToggling(false);
      }
    },
    [fetchMetrics]
  );

  const toggleClientDiagnostics = useCallback(
    async (enabled: boolean) => {
      try {
        const updated = await api.diagnostics.updateConfig({
          client_diagnostics_enabled: enabled,
        });
        setConfig(updated);
        // Immediately sync local logger
        if (enabled) {
          diagnosticLogger.enable();
        } else {
          diagnosticLogger.disable();
        }
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : "Failed to update client diagnostics config");
      }
    },
    []
  );

  if (!isAdmin) return null;

  const tabs: { id: Tab; label: string }[] = [
    { id: "overview", label: "Overview" },
    { id: "server-logs", label: "Server Logs" },
    // Client logs are only accepted on primary servers
    ...(!isBackupMode ? [{ id: "client-logs" as Tab, label: "Client Logs" }] : []),
  ];

  const diagnosticsEnabled = config?.diagnostics_enabled ?? false;

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />
      <main className="max-w-6xl mx-auto p-4">
        {/* ── Page Title + Controls ── */}
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-3">
            <button
              onClick={() => navigate("/settings")}
              className="p-1.5 rounded-md text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
              title="Back to Settings"
            >
              <AppIcon name="back-arrow" />
            </button>
            <h1 className="text-2xl font-bold text-gray-900 dark:text-white">
              Diagnostics
            </h1>
          </div>
          <div className="flex items-center gap-3">
            {diagnosticsEnabled && (
              <>
                <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400 cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={autoRefresh}
                    onChange={(e) => setAutoRefresh(e.target.checked)}
                    className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                  />
                  Auto-refresh
                </label>
                <button
                  onClick={fetchMetrics}
                  className="flex items-center gap-1.5 px-3 py-1.5 bg-blue-600 text-white text-sm font-medium rounded-md hover:bg-blue-700 transition-colors"
                >
                  <AppIcon name="reload" themed={false} />
                  Refresh
                </button>
              </>
            )}
          </div>
        </div>

        {error && (
          <div className="p-3 mb-4 bg-red-50 dark:bg-red-900/30 rounded-lg text-sm text-red-600 dark:text-red-400">
            {error}
          </div>
        )}

        {/* ── Diagnostics Master Toggle ── */}
        {!configLoading && config && (
          <div className="mb-4 bg-white dark:bg-gray-800 rounded-lg shadow p-5">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
              <div className="flex-1">
                <div className="flex items-center gap-3 mb-1">
                  <div
                    className={`w-2.5 h-2.5 rounded-full ${
                      diagnosticsEnabled
                        ? "bg-green-500 animate-pulse"
                        : "bg-gray-400"
                    }`}
                  />
                  <h2 className="text-base font-semibold text-gray-900 dark:text-white">
                    Server Diagnostics
                  </h2>
                </div>
                <p className="text-sm text-gray-500 dark:text-gray-400 ml-5.5">
                  {diagnosticsEnabled
                    ? "Collecting server metrics, database stats, storage analysis, and performance data."
                    : "Disabled to save server resources. Enable to view full system metrics and performance data."}
                </p>
              </div>
              <button
                onClick={() => toggleDiagnostics(!diagnosticsEnabled)}
                disabled={toggling}
                className={`shrink-0 px-5 py-2 text-sm font-medium rounded-lg transition-all shadow-sm disabled:opacity-50 ${
                  diagnosticsEnabled
                    ? "bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-300 dark:hover:bg-gray-600"
                    : "bg-blue-600 text-white hover:bg-blue-700 shadow-blue-200 dark:shadow-blue-900/30"
                }`}
              >
                {toggling
                  ? "Updating..."
                  : diagnosticsEnabled
                  ? "Disable"
                  : "Enable Diagnostics"}
              </button>
            </div>

            {/* Client diagnostics sub-toggle (only show when server diagnostics enabled AND on primary server) */}
            {diagnosticsEnabled && !isBackupMode && (
              <div className="mt-4 pt-4 border-t border-gray-100 dark:border-gray-700">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm font-medium text-gray-700 dark:text-gray-300">
                      Client Diagnostics
                    </p>
                    <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                      Collect diagnostic logs from web and mobile clients (errors, performance, API timing)
                    </p>
                  </div>
                  <button
                    onClick={() =>
                      toggleClientDiagnostics(!config.client_diagnostics_enabled)
                    }
                    className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                      config.client_diagnostics_enabled
                        ? "bg-blue-600"
                        : "bg-gray-300 dark:bg-gray-600"
                    }`}
                    role="switch"
                    aria-checked={config.client_diagnostics_enabled}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                        config.client_diagnostics_enabled
                          ? "translate-x-5"
                          : "translate-x-0"
                      }`}
                    />
                  </button>
                </div>
              </div>
            )}
          </div>
        )}

        {/* ── External API Reference ── */}
        {!configLoading && config && (
          <ExternalApiSection />
        )}

        {/* ── Disabled State ── */}
        {!diagnosticsEnabled && !loading && (
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-12 text-center">
            <div className="inline-flex items-center justify-center w-16 h-16 rounded-full bg-gray-100 dark:bg-gray-700 mb-4">
              <svg
                className="w-8 h-8 text-gray-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
                />
              </svg>
            </div>
            <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-2">
              Diagnostics Collection Disabled
            </h3>
            <p className="text-sm text-gray-500 dark:text-gray-400 max-w-md mx-auto mb-1">
              Server diagnostics are disabled by default to conserve resources.
              Enable above to view detailed system metrics, database statistics,
              storage analysis, and performance monitoring.
            </p>
            {disabledInfo && (
              <p className="text-xs text-gray-400 dark:text-gray-500 mt-3">
                Server v{disabledInfo.version} &middot; Uptime{" "}
                {formatUptime(disabledInfo.uptime_seconds)}
              </p>
            )}
          </div>
        )}

        {/* ── Enabled: Tab Bar + Content ── */}
        {diagnosticsEnabled && (
          <>
            {/* ── Tab Bar ── */}
            <div className="flex gap-1 mb-4 bg-white dark:bg-gray-800 rounded-lg shadow p-1">
              {tabs.map((tab) => (
                <button
                  key={tab.id}
                  onClick={() => setActiveTab(tab.id)}
                  className={`flex-1 py-2 px-4 text-sm font-medium rounded-md transition-colors ${
                    activeTab === tab.id
                      ? "bg-blue-600 text-white shadow"
                      : "text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700"
                  }`}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            {/* ── Tab Content ── */}
            {loading ? (
              <div className="flex justify-center py-20">
                <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
              </div>
            ) : (
              <>
                {activeTab === "overview" && metrics && (
                  <OverviewTab metrics={metrics} />
                )}
                {activeTab === "server-logs" && <ServerLogsTab />}
                {activeTab === "client-logs" && <ClientLogsTab />}
              </>
            )}
          </>
        )}
      </main>
    </div>
  );
}
