import { useState, useEffect, useCallback } from "react";
import { api, AuditLogEntry } from "../../api/client";
import { Section, getDateCutoff, tryPrettyJson } from "./shared";
import { formatDate, relativeTime } from "../../utils/formatters";

const EVENT_COLORS: Record<string, string> = {
  login_success: "text-green-600 dark:text-green-400",
  login_failure: "text-red-600 dark:text-red-400",
  totp_login_failure: "text-red-600 dark:text-red-400",
  rate_limited: "text-orange-600 dark:text-orange-400",
  account_locked: "text-red-700 dark:text-red-300",
  register: "text-blue-600 dark:text-blue-400",
  admin_action: "text-purple-600 dark:text-purple-400",
  password_changed: "text-yellow-600 dark:text-yellow-400",
};

function ServerLogsTab() {
  const [logs, setLogs] = useState<AuditLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [total, setTotal] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);

  // Filters
  const [eventFilter, setEventFilter] = useState("");
  const [ipFilter, setIpFilter] = useState("");
  const [searchText, setSearchText] = useState("");
  const [dateRange, setDateRange] = useState<"all" | "1h" | "24h" | "7d" | "30d">("all");

  // Unique event types for filter dropdown
  const [eventTypes, setEventTypes] = useState<string[]>([]);

  const fetchLogs = useCallback(
    async (cursor?: string) => {
      try {
        const after = dateRange !== "all" ? getDateCutoff(dateRange) : undefined;
        const data = await api.diagnostics.listAuditLogs({
          event_type: eventFilter || undefined,
          ip_address: ipFilter || undefined,
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
    [eventFilter, ipFilter, dateRange]
  );

  useEffect(() => {
    setLoading(true);
    fetchLogs();
  }, [fetchLogs]);

  // Extract unique event types from fetched logs
  useEffect(() => {
    const types = new Set(logs.map((l) => l.event_type));
    setEventTypes(Array.from(types).sort());
  }, [logs]);

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
          l.user_agent.toLowerCase().includes(searchText.toLowerCase())
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
        </div>
        <div className="mt-2 text-xs text-gray-500 dark:text-gray-400">
          Showing {filtered.length} of {total.toLocaleString()} entries
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
                      colSpan={5}
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
        <td className="px-3 py-2 text-xs font-mono text-gray-600 dark:text-gray-400">
          {log.ip_address}
        </td>
        <td className="px-3 py-2 text-xs text-gray-500 dark:text-gray-400 max-w-xs truncate">
          {log.details !== "{}" ? log.details : "—"}
        </td>
      </tr>
      {expanded && (
        <tr className="bg-gray-50 dark:bg-gray-800/80">
          <td colSpan={5} className="px-4 py-3">
            <div className="text-xs space-y-1 text-gray-600 dark:text-gray-300">
              <p>
                <span className="font-medium">Full timestamp:</span>{" "}
                {formatDate(log.created_at)}
              </p>
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
