import { useState, useEffect, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { api, type DiagnosticsResponse } from "../api/client";
import AppHeader from "../components/AppHeader";
import AppIcon from "../components/AppIcon";
import { useIsAdmin } from "../hooks/useIsAdmin";
import OverviewTab from "../components/diagnostics/OverviewTab";
import ServerLogsTab from "../components/diagnostics/ServerLogsTab";
import ClientLogsTab from "../components/diagnostics/ClientLogsTab";

type Tab = "overview" | "server-logs" | "client-logs";

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

  // Redirect non-admins
  useEffect(() => {
    if (!isAdmin) {
      navigate("/settings", { replace: true });
    }
  }, [isAdmin, navigate]);

  const fetchMetrics = useCallback(async () => {
    try {
      const data = await api.diagnostics.getMetrics();
      setMetrics(data);
      setError("");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to load diagnostics");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchMetrics();
  }, [fetchMetrics]);

  // Auto-refresh every 10s when enabled
  useEffect(() => {
    if (autoRefresh) {
      refreshTimer.current = setInterval(fetchMetrics, 10000);
    }
    return () => {
      if (refreshTimer.current) clearInterval(refreshTimer.current);
    };
  }, [autoRefresh, fetchMetrics]);

  if (!isAdmin) return null;

  const tabs: { id: Tab; label: string }[] = [
    { id: "overview", label: "Overview" },
    { id: "server-logs", label: "Server Logs" },
    { id: "client-logs", label: "Client Logs" },
  ];

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
          </div>
        </div>

        {error && (
          <div className="p-3 mb-4 bg-red-50 dark:bg-red-900/30 rounded-lg text-sm text-red-600 dark:text-red-400">
            {error}
          </div>
        )}

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
      </main>
    </div>
  );
}
