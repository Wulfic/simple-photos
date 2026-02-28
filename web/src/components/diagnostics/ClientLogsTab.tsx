import { useState, useEffect, useCallback } from "react";
import { api, ClientLogEntry } from "../../api/client";
import { Section, getDateCutoff, tryPrettyJson } from "./shared";
import { formatDate, relativeTime } from "../../utils/formatters";

const LEVEL_COLORS: Record<string, string> = {
  debug: "bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
  info: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300",
  warn: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300",
  error: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300",
};

function ClientLogsTab() {
  const [logs, setLogs] = useState<ClientLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);

  // Filters
  const [levelFilter, setLevelFilter] = useState("");
  const [sessionFilter, setSessionFilter] = useState("");
  const [searchText, setSearchText] = useState("");
  const [dateRange, setDateRange] = useState<"all" | "1h" | "24h" | "7d">("all");

  const fetchLogs = useCallback(
    async (cursor?: string) => {
      try {
        const data = await api.diagnostics.listClientLogs({
          level: levelFilter || undefined,
          session_id: sessionFilter || undefined,
          after: cursor,
          limit: 200,
        });
        if (cursor) {
          setLogs((prev) => [...prev, ...data.logs]);
        } else {
          setLogs(data.logs);
        }
        setNextCursor(data.next_cursor);
        setError("");
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : "Failed to load client logs");
      } finally {
        setLoading(false);
        setLoadingMore(false);
      }
    },
    [levelFilter, sessionFilter]
  );

  useEffect(() => {
    setLoading(true);
    fetchLogs();
  }, [fetchLogs]);

  function loadMore() {
    if (!nextCursor || loadingMore) return;
    setLoadingMore(true);
    fetchLogs(nextCursor);
  }

  // Client-side filtering
  let filtered = logs;
  if (searchText) {
    const q = searchText.toLowerCase();
    filtered = filtered.filter(
      (l) =>
        l.message.toLowerCase().includes(q) ||
        l.tag.toLowerCase().includes(q) ||
        l.session_id.toLowerCase().includes(q)
    );
  }
  if (dateRange !== "all") {
    const cutoff = new Date(getDateCutoff(dateRange)).getTime();
    filtered = filtered.filter(
      (l) => new Date(l.created_at).getTime() > cutoff
    );
  }

  // Unique sessions for filter
  const sessions = [...new Set(logs.map((l) => l.session_id))];

  return (
    <div className="space-y-4">
      {/* Filters */}
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
        <div className="flex flex-wrap gap-3 items-end">
          <div className="flex-1 min-w-[200px]">
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Search
            </label>
            <input
              type="text"
              value={searchText}
              onChange={(e) => setSearchText(e.target.value)}
              placeholder="Filter by message, tag, session..."
              className="w-full px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Level
            </label>
            <select
              value={levelFilter}
              onChange={(e) => setLevelFilter(e.target.value)}
              className="px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500"
            >
              <option value="">All Levels</option>
              <option value="debug">Debug</option>
              <option value="info">Info</option>
              <option value="warn">Warn</option>
              <option value="error">Error</option>
            </select>
          </div>
          <div>
            <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
              Session
            </label>
            <select
              value={sessionFilter}
              onChange={(e) => setSessionFilter(e.target.value)}
              className="px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-1 focus:ring-blue-500"
            >
              <option value="">All Sessions</option>
              {sessions.map((s) => (
                <option key={s} value={s}>
                  {s.length > 20 ? s.slice(0, 20) + "…" : s}
                </option>
              ))}
            </select>
          </div>
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
            </select>
          </div>
        </div>
        <div className="mt-2 text-xs text-gray-500 dark:text-gray-400">
          Showing {filtered.length} of {logs.length} entries
        </div>
      </div>

      {error && (
        <div className="p-3 bg-red-50 dark:bg-red-900/30 rounded-lg text-sm text-red-600 dark:text-red-400">
          {error}
        </div>
      )}

      {/* Log List */}
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
                    Level
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Tag
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Message
                  </th>
                  <th className="text-left px-3 py-2 font-medium text-gray-500 dark:text-gray-400">
                    Session
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
                {filtered.map((log) => (
                  <ClientLogRow key={log.id} log={log} />
                ))}
                {filtered.length === 0 && (
                  <tr>
                    <td
                      colSpan={5}
                      className="text-center py-8 text-gray-400 dark:text-gray-500"
                    >
                      No client log entries found
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

function ClientLogRow({ log }: { log: ClientLogEntry }) {
  const [expanded, setExpanded] = useState(false);
  const levelClass = LEVEL_COLORS[log.level] || LEVEL_COLORS["debug"];

  return (
    <>
      <tr
        className="hover:bg-gray-50 dark:hover:bg-gray-800/50 cursor-pointer"
        onClick={() => setExpanded((v) => !v)}
      >
        <td
          className="px-3 py-2 text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap"
          title={formatDate(log.client_ts)}
        >
          {relativeTime(log.created_at)}
        </td>
        <td className="px-3 py-2">
          <span
            className={`inline-block px-1.5 py-0.5 rounded text-xs font-medium uppercase ${levelClass}`}
          >
            {log.level}
          </span>
        </td>
        <td className="px-3 py-2 text-xs font-mono text-gray-600 dark:text-gray-400">
          {log.tag}
        </td>
        <td className="px-3 py-2 text-xs text-gray-700 dark:text-gray-300 max-w-md truncate">
          {log.message}
        </td>
        <td className="px-3 py-2 text-xs font-mono text-gray-400 max-w-[100px] truncate">
          {log.session_id.slice(0, 8)}…
        </td>
      </tr>
      {expanded && (
        <tr className="bg-gray-50 dark:bg-gray-800/80">
          <td colSpan={5} className="px-4 py-3">
            <div className="text-xs space-y-1.5 text-gray-600 dark:text-gray-300">
              <p>
                <span className="font-medium">Full Message:</span>{" "}
                <span className="break-all">{log.message}</span>
              </p>
              <p>
                <span className="font-medium">Client Timestamp:</span>{" "}
                {formatDate(log.client_ts)}
              </p>
              <p>
                <span className="font-medium">Server Received:</span>{" "}
                {formatDate(log.created_at)}
              </p>
              <p>
                <span className="font-medium">Session ID:</span>{" "}
                <span className="font-mono">{log.session_id}</span>
              </p>
              <p>
                <span className="font-medium">User ID:</span>{" "}
                <span className="font-mono">{log.user_id}</span>
              </p>
              {log.context != null && (
                <div>
                  <span className="font-medium">Context:</span>
                  <pre className="mt-1 p-2 bg-gray-100 dark:bg-gray-900 rounded text-xs font-mono whitespace-pre-wrap break-all">
                    {tryPrettyJson(
                      typeof log.context === "string"
                        ? log.context
                        : JSON.stringify(log.context)
                    )}
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

export default ClientLogsTab;
