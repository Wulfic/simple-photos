/**
 * Server metrics overview tab for the admin Diagnostics page.
 *
 * Displays uptime, storage usage, user/photo counts, recent audit events,
 * and system health data fetched from `/api/admin/diagnostics`.
 */
import { DiagnosticsResponse } from "../../api/client";
import { Section, StatCard } from "./shared";
import { formatBytes, formatUptime, formatDate, relativeTime } from "../../utils/formatters";

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

const LEVEL_COLORS: Record<string, string> = {
  debug: "bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
  info: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300",
  warn: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300",
  error: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300",
};

function OverviewTab({ metrics }: { metrics: DiagnosticsResponse }) {
  const { server, database, storage, users, photos, audit, client_logs, backup, performance } =
    metrics;

  return (
    <div className="space-y-4">
      {/* ── Server Info ── */}
      <Section title="Server">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <StatCard label="Version" value={server.version} />
          <StatCard label="Uptime" value={formatUptime(server.uptime_seconds)} />
          <StatCard label="PID" value={server.pid.toString()} />
          <StatCard
            label="Memory (RSS)"
            value={formatBytes(server.memory_rss_bytes)}
          />
          <StatCard label="CPU Time" value={`${server.cpu_seconds.toFixed(1)}s`} />
          <StatCard label="OS / Arch" value={`${server.os} / ${server.arch}`} />
          <StatCard label="TLS" value={server.tls_enabled ? "Enabled" : "Disabled"} />
          <StatCard
            label="Max Blob"
            value={`${server.max_blob_size_mb} MiB`}
          />
        </div>
        <div className="mt-3 text-xs text-gray-500 dark:text-gray-400 space-y-0.5">
          <p>
            <span className="font-medium">Storage root:</span>{" "}
            <code className="bg-gray-100 dark:bg-gray-700 px-1 rounded">
              {server.storage_root}
            </code>
          </p>
          <p>
            <span className="font-medium">Database:</span>{" "}
            <code className="bg-gray-100 dark:bg-gray-700 px-1 rounded">
              {server.db_path}
            </code>
          </p>
          <p>
            <span className="font-medium">Started:</span> {formatDate(server.started_at)}
          </p>
        </div>
      </Section>

      {/* ── Performance ── */}
      <Section title="Performance">
        <div className="grid grid-cols-2 md:grid-cols-3 gap-4">
          <StatCard
            label="DB Ping"
            value={`${performance.db_ping_ms.toFixed(2)} ms`}
            color={performance.db_ping_ms < 5 ? "green" : performance.db_ping_ms < 20 ? "yellow" : "red"}
          />
          <StatCard label="DB Page Size" value={formatBytes(database.page_size)} />
          <StatCard
            label="DB Size"
            value={formatBytes(database.size_bytes + database.wal_size_bytes)}
            subtitle={`WAL: ${formatBytes(database.wal_size_bytes)}`}
          />
          <StatCard label="Pages" value={database.page_count.toLocaleString()} />
          <StatCard label="Free Pages" value={database.freelist_count.toLocaleString()} />
          <StatCard label="Journal Mode" value={database.journal_mode.toUpperCase()} />
        </div>
      </Section>

      {/* ── Storage ── */}
      <Section title="Storage">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <StatCard
            label="Photos on Disk"
            value={formatBytes(storage.total_bytes)}
          />
          <StatCard label="Files" value={storage.file_count.toLocaleString()} />
          <StatCard
            label="Disk Total"
            value={formatBytes(storage.disk_total_bytes)}
          />
          <StatCard
            label="Disk Available"
            value={formatBytes(storage.disk_available_bytes)}
            color={storage.disk_used_percent > 90 ? "red" : storage.disk_used_percent > 70 ? "yellow" : "green"}
          />
        </div>
        {/* Disk usage bar */}
        {storage.disk_total_bytes > 0 && (
          <div className="mt-3">
            <div className="flex justify-between text-xs text-gray-500 dark:text-gray-400 mb-1">
              <span>Disk Usage</span>
              <span>{storage.disk_used_percent.toFixed(1)}%</span>
            </div>
            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2.5">
              <div
                className={`h-2.5 rounded-full transition-all ${
                  storage.disk_used_percent > 90
                    ? "bg-red-500"
                    : storage.disk_used_percent > 70
                    ? "bg-yellow-500"
                    : "bg-green-500"
                }`}
                style={{ width: `${Math.min(storage.disk_used_percent, 100)}%` }}
              />
            </div>
          </div>
        )}
      </Section>

      {/* ── Users ── */}
      <Section title="Users">
        <div className="grid grid-cols-3 gap-4">
          <StatCard label="Total Users" value={users.total_users.toString()} />
          <StatCard label="Admins" value={users.admin_count.toString()} />
          <StatCard label="2FA Enabled" value={users.totp_enabled_count.toString()} />
        </div>
      </Section>

      {/* ── Photos ── */}
      <Section title="Photos">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <StatCard label="Total" value={photos.total_photos.toLocaleString()} />
          <StatCard label="Encrypted" value={photos.encrypted_count.toLocaleString()} />
          <StatCard label="With Thumbs" value={photos.photos_with_thumbs.toLocaleString()} />
          <StatCard label="File Data" value={formatBytes(photos.total_file_bytes)} />
          <StatCard label="Thumb Data" value={formatBytes(photos.total_thumb_bytes)} />
          <StatCard label="Favorited" value={photos.favorited_count.toLocaleString()} />
          <StatCard label="Tagged" value={photos.tagged_count.toLocaleString()} />
        </div>
        {/* Media type breakdown */}
        {Object.keys(photos.photos_by_media_type).length > 0 && (
          <div className="mt-3">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
              By Media Type
            </p>
            <div className="flex flex-wrap gap-2">
              {Object.entries(photos.photos_by_media_type)
                .sort(([, a], [, b]) => b - a)
                .map(([type, count]) => (
                  <span
                    key={type}
                    className="inline-flex items-center gap-1 px-2 py-1 bg-gray-100 dark:bg-gray-700 rounded text-xs text-gray-700 dark:text-gray-300"
                  >
                    <span className="font-medium">{type}</span>
                    <span className="text-gray-400">{count.toLocaleString()}</span>
                  </span>
                ))}
            </div>
          </div>
        )}
        {/* Date range */}
        <div className="mt-3 text-xs text-gray-500 dark:text-gray-400 space-y-0.5">
          {photos.oldest_photo && (
            <p>
              <span className="font-medium">Oldest:</span> {formatDate(photos.oldest_photo)}
            </p>
          )}
          {photos.newest_photo && (
            <p>
              <span className="font-medium">Newest:</span> {formatDate(photos.newest_photo)}
            </p>
          )}
        </div>
      </Section>

      {/* ── Audit Summary ── */}
      <Section title="Audit Log Summary">
        <div className="grid grid-cols-3 gap-4">
          <StatCard label="Total Events" value={audit.total_entries.toLocaleString()} />
          <StatCard label="Last 24h" value={audit.entries_last_24h.toLocaleString()} />
          <StatCard label="Last 7d" value={audit.entries_last_7d.toLocaleString()} />
        </div>
        {/* Event type breakdown */}
        {Object.keys(audit.events_by_type).length > 0 && (
          <div className="mt-3">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
              Events by Type
            </p>
            <div className="flex flex-wrap gap-2">
              {Object.entries(audit.events_by_type)
                .sort(([, a], [, b]) => b - a)
                .map(([type, count]) => (
                  <span
                    key={type}
                    className={`inline-flex items-center gap-1 px-2 py-1 rounded text-xs ${
                      EVENT_COLORS[type] || "text-gray-700 dark:text-gray-300"
                    } bg-gray-100 dark:bg-gray-700`}
                  >
                    <span className="font-medium">{type.replace(/_/g, " ")}</span>
                    <span className="opacity-60">{count.toLocaleString()}</span>
                  </span>
                ))}
            </div>
          </div>
        )}
        {/* Recent failures */}
        {audit.recent_failures.length > 0 && (
          <div className="mt-4">
            <p className="text-xs font-medium text-red-600 dark:text-red-400 mb-2">
              Recent Security Events ({audit.recent_failures.length})
            </p>
            <div className="max-h-48 overflow-y-auto rounded border border-gray-200 dark:border-gray-700">
              <table className="w-full text-xs">
                <thead className="bg-gray-50 dark:bg-gray-800 sticky top-0">
                  <tr>
                    <th className="text-left px-2 py-1 font-medium">Event</th>
                    <th className="text-left px-2 py-1 font-medium">IP</th>
                    <th className="text-left px-2 py-1 font-medium">Time</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
                  {audit.recent_failures.slice(0, 20).map((f, i) => (
                    <tr key={i} className="hover:bg-gray-50 dark:hover:bg-gray-800/50">
                      <td className="px-2 py-1 text-red-600 dark:text-red-400 font-mono">
                        {f.event_type.replace(/_/g, " ")}
                      </td>
                      <td className="px-2 py-1 font-mono">{f.ip_address}</td>
                      <td className="px-2 py-1 text-gray-500" title={formatDate(f.created_at)}>
                        {relativeTime(f.created_at)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </Section>

      {/* ── Client Logs Summary ── */}
      <Section title="Client Log Summary">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <StatCard label="Total Entries" value={client_logs.total_entries.toLocaleString()} />
          <StatCard label="Last 24h" value={client_logs.entries_last_24h.toLocaleString()} />
          <StatCard label="Last 7d" value={client_logs.entries_last_7d.toLocaleString()} />
          <StatCard label="Sessions" value={client_logs.unique_sessions.toLocaleString()} />
        </div>
        {Object.keys(client_logs.by_level).length > 0 && (
          <div className="mt-3 flex flex-wrap gap-2">
            {Object.entries(client_logs.by_level)
              .sort(([, a], [, b]) => b - a)
              .map(([level, count]) => (
                <span
                  key={level}
                  className={`inline-flex items-center gap-1 px-2 py-1 rounded text-xs font-medium ${
                    LEVEL_COLORS[level] || "bg-gray-100 dark:bg-gray-700"
                  }`}
                >
                  {level.toUpperCase()} {count.toLocaleString()}
                </span>
              ))}
          </div>
        )}
      </Section>

      {/* ── Backup ── */}
      <Section title="Backup">
        <div className="grid grid-cols-3 gap-4">
          <StatCard label="Servers" value={backup.server_count.toString()} />
          <StatCard label="Sync Logs" value={backup.total_sync_logs.toLocaleString()} />
          <StatCard
            label="Last Sync"
            value={backup.last_sync_at ? relativeTime(backup.last_sync_at) : "Never"}
          />
        </div>
      </Section>

      {/* ── Database Table Counts ── */}
      <Section title="Database Tables">
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          {Object.entries(database.table_counts)
            .sort(([, a], [, b]) => b - a)
            .map(([table, count]) => (
              <div
                key={table}
                className="flex items-center justify-between bg-gray-50 dark:bg-gray-700/50 rounded px-3 py-2 text-xs"
              >
                <span className="font-mono text-gray-600 dark:text-gray-300">
                  {table}
                </span>
                <span className="font-bold text-gray-900 dark:text-white">
                  {count.toLocaleString()}
                </span>
              </div>
            ))}
        </div>
      </Section>
    </div>
  );
}

export default OverviewTab;
