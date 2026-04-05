/** Audit log viewer tab — filterable, paginated, real-time server event log. */
import { useState, useEffect, useCallback, useRef } from "react";
import { api, AuditLogEntry } from "../../api/client";
import { BASE } from "../../api/core";
import { useAuthStore } from "../../store/auth";
import { getDateCutoff, tryPrettyJson } from "./shared";
import { formatDate, relativeTime } from "../../utils/formatters";

const EVENT_COLORS: Record<string, string> = {
  // Auth
  login_success: "text-green-600 dark:text-green-400",
  login_failure: "text-red-600 dark:text-red-400",
  totp_login_success: "text-green-600 dark:text-green-400",
  totp_login_failure: "text-red-600 dark:text-red-400",
  rate_limited: "text-orange-600 dark:text-orange-400",
  account_locked: "text-red-700 dark:text-red-300",
  register: "text-blue-600 dark:text-blue-400",
  password_changed: "text-yellow-600 dark:text-yellow-400",
  totp_setup: "text-indigo-600 dark:text-indigo-400",
  totp_enabled: "text-indigo-600 dark:text-indigo-400",
  totp_disabled: "text-indigo-600 dark:text-indigo-400",
  backup_code_used: "text-yellow-600 dark:text-yellow-400",
  token_refresh: "text-gray-500 dark:text-gray-400",
  logout: "text-gray-500 dark:text-gray-400",
  // Blobs
  blob_upload: "text-cyan-600 dark:text-cyan-400",
  blob_delete: "text-red-500 dark:text-red-400",
  // Photos
  photo_register: "text-cyan-600 dark:text-cyan-400",
  photo_favorite: "text-pink-500 dark:text-pink-400",
  photo_crop_set: "text-teal-600 dark:text-teal-400",
  // Tags
  tag_add: "text-emerald-600 dark:text-emerald-400",
  tag_remove: "text-orange-500 dark:text-orange-400",
  // Trash
  trash_soft_delete: "text-red-500 dark:text-red-400",
  trash_restore: "text-green-600 dark:text-green-400",
  trash_permanent_delete: "text-red-700 dark:text-red-300",
  trash_empty: "text-red-700 dark:text-red-300",
  // Sharing
  shared_album_create: "text-violet-600 dark:text-violet-400",
  shared_album_delete: "text-violet-600 dark:text-violet-400",
  shared_album_add_member: "text-violet-500 dark:text-violet-400",
  shared_album_remove_member: "text-violet-500 dark:text-violet-400",
  shared_album_add_photo: "text-violet-500 dark:text-violet-400",
  shared_album_remove_photo: "text-violet-500 dark:text-violet-400",
  // Backup
  backup_server_add: "text-sky-600 dark:text-sky-400",
  backup_server_update: "text-sky-600 dark:text-sky-400",
  backup_server_remove: "text-sky-600 dark:text-sky-400",
  backup_mode_change: "text-amber-600 dark:text-amber-400",
  audio_backup_toggle: "text-amber-600 dark:text-amber-400",
  // Sync & Recovery
  sync_trigger: "text-blue-500 dark:text-blue-400",
  sync_force_from_primary: "text-blue-500 dark:text-blue-400",
  recovery_start: "text-red-600 dark:text-red-400",
  // Background tasks
  auto_scan_complete: "text-teal-600 dark:text-teal-400",
  trash_purge_complete: "text-orange-600 dark:text-orange-400",
  housekeeping_complete: "text-gray-500 dark:text-gray-400",
  encryption_migration_complete: "text-emerald-600 dark:text-emerald-400",
  backup_sync_cycle_complete: "text-sky-600 dark:text-sky-400",
  // Admin
  admin_action: "text-purple-600 dark:text-purple-400",
};

function ServerLogsTab() {
  const [logs, setLogs] = useState<AuditLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [total, setTotal] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  const [streaming, setStreaming] = useState(false);

  // Filters
  const [eventFilter, setEventFilter] = useState("");
  const [ipFilter, setIpFilter] = useState("");
  const [searchText, setSearchText] = useState("");
  const [dateRange, setDateRange] = useState<"all" | "1h" | "24h" | "7d" | "30d">("all");
  const [serverFilter, setServerFilter] = useState("");

  // Unique event types for filter dropdown
  const [eventTypes, setEventTypes] = useState<string[]>([]);
  // Unique source servers for filter dropdown
  const [sourceServers, setSourceServers] = useState<string[]>([]);

  const fetchLogs = useCallback(
    async (cursor?: string) => {
      try {
        const after = dateRange !== "all" ? getDateCutoff(dateRange) : undefined;
        const data = await api.diagnostics.listAuditLogs({
          event_type: eventFilter || undefined,
          ip_address: ipFilter || undefined,
          source_server: serverFilter || undefined,
          after,
          before: cursor,
          limit: 100,
        });
        if (cursor) {
          setLogs((prev) => [...prev, ...data.logs]);
        } else {
          setLogs(data.logs);
        }
        setNextCursor(data.next_cursor);
        setTotal(data.total);
        setError("");
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : "Failed to load audit logs");
      } finally {
        setLoading(false);
        setLoadingMore(false);
      }
    },
    [eventFilter, ipFilter, dateRange, serverFilter]
  );

  useEffect(() => {
    setLoading(true);
    fetchLogs();
  }, [fetchLogs]);

  // Extract unique event types and source servers from fetched logs
  useEffect(() => {
    const types = new Set(logs.map((l) => l.event_type));
    setEventTypes(Array.from(types).sort());
    const servers = new Set(
      logs.map((l) => l.source_server).filter((s): s is string => s !== null)
    );
    setSourceServers(Array.from(servers).sort());
  }, [logs]);

  // Real-time SSE subscription — prepends new entries as they arrive
  const seenIds = useRef(new Set<string>());
  useEffect(() => {
    // Seed the dedup set with initially-fetched log IDs
    seenIds.current = new Set(logs.map((l) => l.id));
  }, []); // only on mount

  useEffect(() => {
    const token = useAuthStore.getState().accessToken;
    if (!token) return;

    const url = `${BASE}/admin/audit-logs/stream?token=${encodeURIComponent(token)}`;
    const es = new EventSource(url);

    es.onopen = () => setStreaming(true);

    es.onmessage = (event) => {
      try {
        const entry = JSON.parse(event.data) as AuditLogEntry;
        if (seenIds.current.has(entry.id)) return; // dedup
        seenIds.current.add(entry.id);
        setLogs((prev) => [entry, ...prev]);
        setTotal((prev) => prev + 1);
      } catch {
        // ignore malformed messages
      }
    };

    es.onerror = () => {
      setStreaming(false);
      // EventSource automatically reconnects — no manual retry needed
    };

    return () => {
      es.close();
      setStreaming(false);
    };
  }, []); // single connection for the lifetime of the tab

  function loadMore() {
    if (!nextCursor || loadingMore) return;
    setLoadingMore(true);
    fetchLogs(nextCursor);
  }

  // Client-side text search over already-fetched logs
  const filtered = searchText
    ? logs.filter(
        (l) =>
          l.event_type.includes(searchText.toLowerCase()) ||
          l.ip_address.includes(searchText) ||
          (l.username && l.username.toLowerCase().includes(searchText.toLowerCase())) ||
          l.details.toLowerCase().includes(searchText.toLowerCase()) ||
          l.user_agent.toLowerCase().includes(searchText.toLowerCase()) ||
          (l.source_server && l.source_server.toLowerCase().includes(searchText.toLowerCase()))
      )
    : logs;

  return (
    <div className="space-y-4">
      {/* Filters */}
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
        <div className="flex flex-wrap gap-3 items-end">
          {/* Text Search */}
          <div className="flex-1 min-w-[200px]">
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Search
            </label>
            <input
              type="text"
              value={searchText}
              onChange={(e) => setSearchText(e.target.value)}
              placeholder="Filter by text..."
              className="w-full px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
            />
          </div>
          {/* Event Type */}
          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Event Type
            </label>
            <select
              value={eventFilter}
              onChange={(e) => setEventFilter(e.target.value)}
              className="px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500"
            >
              <option value="">All Events</option>
              {eventTypes.map((t) => (
                <option key={t} value={t}>
                  {t.replace(/_/g, " ")}
                </option>
              ))}
            </select>
          </div>
          {/* IP Filter */}
          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              IP Address
            </label>
            <input
              type="text"
              value={ipFilter}
              onChange={(e) => setIpFilter(e.target.value)}
              placeholder="e.g. 192.168.1.1"
              className="w-36 px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500"
            />
          </div>
          {/* Date Range */}
          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Time Range
            </label>
            <select
              value={dateRange}
              onChange={(e) =>
                setDateRange(e.target.value as typeof dateRange)
              }
              className="px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500"
            >
              <option value="all">All Time</option>
              <option value="1h">Last Hour</option>
              <option value="24h">Last 24h</option>
              <option value="7d">Last 7 Days</option>
              <option value="30d">Last 30 Days</option>
            </select>
          </div>
          {/* Source Server Filter */}
          {sourceServers.length > 0 && (
            <div>
              <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
                Source
              </label>
              <select
                value={serverFilter}
                onChange={(e) => setServerFilter(e.target.value)}
                className="px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500"
              >
                <option value="">All Servers</option>
                <option value="local">This Server</option>
                {sourceServers.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>
        <div className="mt-2 flex items-center gap-3 text-xs text-gray-500 dark:text-gray-400">
          <span>Showing {filtered.length} of {total.toLocaleString()} entries</span>
          <span className="flex items-center gap-1">
            <span className={`inline-block w-2 h-2 rounded-full ${streaming ? "bg-green-500 animate-pulse" : "bg-gray-400"}`} />
            {streaming ? "Live" : "Connecting…"}
          </span>
        </div>
      </div>

      {error && (
        <div className="p-3 bg-red-50 dark:bg-red-900/30 rounded-lg text-sm text-red-600 dark:text-red-400">
          {error}
        </div>
      )}

      {/* Log Table */}
      {loading ? (
        <div className="flex justify-center py-12">
          <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
        </div>
      ) : (
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow overflow-hidden">
          <div className="overflow-x-auto max-h-[65vh] overflow-y-auto">
            <table className="w-full text-sm">
              <thead className="bg-gray-50 dark:bg-gray-700 sticky top-0 z-10">
                <tr>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Time
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Event
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    User
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Source
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    IP
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Details
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
                {filtered.map((log) => (
                  <AuditLogRow key={log.id} log={log} />
                ))}
                {filtered.length === 0 && (
                  <tr>
                    <td
                      colSpan={6}
                      className="text-center py-8 text-gray-400 dark:text-gray-500"
                    >
                      No audit log entries found
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
          {nextCursor && (
            <div className="border-t border-gray-100 dark:border-gray-700 px-4 py-3 text-center">
              <button
                onClick={loadMore}
                disabled={loadingMore}
                className="px-4 py-1.5 text-sm font-medium text-blue-600 dark:text-blue-400 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-md transition-colors disabled:opacity-50"
              >
                {loadingMore ? "Loading..." : "Load More"}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function AuditLogRow({ log }: { log: AuditLogEntry }) {
  const [expanded, setExpanded] = useState(false);
  const colorClass = EVENT_COLORS[log.event_type] || "text-gray-700 dark:text-gray-300";

  return (
    <>
      <tr
        className="hover:bg-gray-50 dark:hover:bg-gray-800/50 cursor-pointer"
        onClick={() => setExpanded((v) => !v)}
      >
        <td className="px-3 py-2 text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap" title={formatDate(log.created_at)}>
          {relativeTime(log.created_at)}
        </td>
        <td className={`px-3 py-2 text-xs font-mono font-medium ${colorClass}`}>
          {log.event_type.replace(/_/g, " ")}
        </td>
        <td className="px-3 py-2 text-xs text-gray-700 dark:text-gray-300">
          {log.username || log.user_id || "—"}
        </td>
        <td className="px-3 py-2 text-xs whitespace-nowrap">
          {log.source_server ? (
            <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-sky-100 dark:bg-sky-900/30 text-sky-700 dark:text-sky-300 font-medium">
              <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2" />
              </svg>
              {log.source_server}
            </span>
          ) : (
            <span className="text-gray-400 dark:text-gray-500">local</span>
          )}
        </td>
        <td className="px-3 py-2 text-xs font-mono text-gray-600 dark:text-gray-400">
          {log.ip_address}
        </td>
        <td className="px-3 py-2 text-xs text-gray-500 dark:text-gray-400 max-w-xs truncate">
          {log.details !== "{}" ? log.details : "—"}
        </td>
      </tr>
      {expanded && (
        <tr className="bg-gray-50 dark:bg-gray-800/80">
          <td colSpan={6} className="px-4 py-3">
            <div className="text-xs space-y-1 text-gray-600 dark:text-gray-300">
              <p>
                <span className="font-medium">Full timestamp:</span>{" "}
                {formatDate(log.created_at)}
              </p>
              {log.source_server && (
                <p>
                  <span className="font-medium">Source server:</span>{" "}
                  <span className="font-mono">{log.source_server}</span>
                </p>
              )}
              <p>
                <span className="font-medium">User Agent:</span>{" "}
                <span className="font-mono break-all">{log.user_agent}</span>
              </p>
              {log.details !== "{}" && (
                <div>
                  <span className="font-medium">Details:</span>
                  <pre className="mt-1 p-2 bg-gray-100 dark:bg-gray-900 rounded text-xs font-mono whitespace-pre-wrap break-all">
                    {tryPrettyJson(log.details)}
                  </pre>
                </div>
              )}
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

export default ServerLogsTab;
