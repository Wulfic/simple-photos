/**
 * Server metrics overview tab for the admin Diagnostics page.
 *
 * Displays uptime, storage usage, user/photo counts, recent audit events,
 * system health data, backup server details, collection timings, and
 * performance metrics fetched from `/api/admin/diagnostics`.
 */
import { useState } from "react";
import { DiagnosticsResponse } from "../../api/client";
import { Section, StatCard } from "./shared";
import { formatBytes, formatUptime, formatDate, relativeTime } from "../../utils/formatters";

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

const LEVEL_COLORS: Record<string, string> = {
  debug: "bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
  info: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300",
  warn: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300",
  error: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300",
};

const SYNC_STATUS_COLORS: Record<string, string> = {
  success: "text-green-600 dark:text-green-400",
  running: "text-blue-600 dark:text-blue-400",
  error: "text-red-600 dark:text-red-400",
  partial: "text-yellow-600 dark:text-yellow-400",
  "": "text-gray-400",
  never: "text-gray-400",
};

function TimingBar({ label, ms, maxMs }: { label: string; ms: number; maxMs: number }) {
  const pct = maxMs > 0 ? Math.min((ms / maxMs) * 100, 100) : 0;
  const color =
    ms > 500 ? "bg-red-500" : ms > 100 ? "bg-yellow-500" : "bg-green-500";
  return (
    <div className="flex items-center gap-3 text-xs">
      <span className="w-24 text-right text-gray-500 dark:text-gray-400 shrink-0 font-medium">
        {label}
      </span>
      <div className="flex-1 bg-gray-200 dark:bg-gray-700 rounded-full h-2">
        <div
          className={`h-2 rounded-full transition-all ${color}`}
          style={{ width: `${Math.max(pct, 1)}%` }}
        />
      </div>
      <span className="w-20 text-right font-mono text-gray-700 dark:text-gray-300 shrink-0">
        {ms < 1 ? `${(ms * 1000).toFixed(0)} \u00B5s` : ms < 1000 ? `${ms.toFixed(1)} ms` : `${(ms / 1000).toFixed(2)} s`}
      </span>
    </div>
  );
}

function OverviewTab({ metrics }: { metrics: DiagnosticsResponse }) {
  const { server, database, storage, users, photos, audit, client_logs, backup, performance, timings } =
    metrics;

  const [expandedBackupServer, setExpandedBackupServer] = useState<string | null>(null);

  return (
    <div className="space-y-4">
      {/* ── Collection Timings ── */}
      <Section title="Collection Timings">
        <div className="mb-3 flex items-center gap-2">
          <span className="text-xs text-gray-500 dark:text-gray-400">
            Total collection time:
          </span>
          <span className={`text-sm font-bold ${
            timings.total_ms > 2000 ? "text-red-600 dark:text-red-400" :
            timings.total_ms > 500 ? "text-yellow-600 dark:text-yellow-400" :
            "text-green-600 dark:text-green-400"
          }`}>
            {timings.total_ms < 1000 ? `${timings.total_ms.toFixed(1)} ms` : `${(timings.total_ms / 1000).toFixed(2)} s`}
          </span>
        </div>
        <div className="space-y-1.5">
          <TimingBar label="Server" ms={timings.server_ms} maxMs={timings.total_ms} />
          <TimingBar label="Database" ms={timings.database_ms} maxMs={timings.total_ms} />
          <TimingBar label="Storage" ms={timings.storage_ms} maxMs={timings.total_ms} />
          <TimingBar label="Users" ms={timings.users_ms} maxMs={timings.total_ms} />
          <TimingBar label="Photos" ms={timings.photos_ms} maxMs={timings.total_ms} />
          <TimingBar label="Audit" ms={timings.audit_ms} maxMs={timings.total_ms} />
          <TimingBar label="Client Logs" ms={timings.client_logs_ms} maxMs={timings.total_ms} />
          <TimingBar label="Backup" ms={timings.backup_ms} maxMs={timings.total_ms} />
          <TimingBar label="Performance" ms={timings.performance_ms} maxMs={timings.total_ms} />
        </div>
      </Section>

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
          <StatCard label="Threads" value={server.thread_count.toString()} />
          <StatCard label="Open FDs" value={server.open_fds.toString()} />
          <StatCard label="TLS" value={server.tls_enabled ? "Enabled" : "Disabled"} />
          <StatCard
            label="Max Blob"
            value={`${server.max_blob_size_mb} MiB`}
          />
        </div>
        {/* Load averages */}
        {(server.load_average[0] > 0 || server.load_average[1] > 0) && (
          <div className="mt-3 p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
              System Load Average
            </p>
            <div className="flex gap-6">
              {["1 min", "5 min", "15 min"].map((label, i) => (
                <div key={label} className="text-center">
                  <p className={`text-lg font-bold ${
                    server.load_average[i] > 4 ? "text-red-600 dark:text-red-400" :
                    server.load_average[i] > 2 ? "text-yellow-600 dark:text-yellow-400" :
                    "text-green-600 dark:text-green-400"
                  }`}>
                    {server.load_average[i].toFixed(2)}
                  </p>
                  <p className="text-xs text-gray-400">{label}</p>
                </div>
              ))}
            </div>
          </div>
        )}
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
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
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
          <StatCard label="Cache Size" value={`${performance.cache_size_kib.toLocaleString()} KiB`} />
          <StatCard label="Pages" value={database.page_count.toLocaleString()} />
          <StatCard label="Free Pages" value={database.freelist_count.toLocaleString()} />
          <StatCard label="Journal Mode" value={database.journal_mode.toUpperCase()} />
          <StatCard
            label="Read Pool"
            value={`${performance.read_pool_idle} idle / ${performance.read_pool_size} total`}
          />
          <StatCard
            label="Write Pool"
            value={`${performance.write_pool_idle} idle / ${performance.write_pool_size} total`}
          />
        </div>
        {/* WAL Checkpoint info */}
        {performance.wal_checkpoint && (
          <div className="mt-3 p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
              WAL Checkpoint Status
            </p>
            <div className="flex gap-6 text-xs">
              <div>
                <span className="text-gray-500 dark:text-gray-400">Log pages: </span>
                <span className="font-bold text-gray-900 dark:text-white">
                  {performance.wal_checkpoint.log_pages.toLocaleString()}
                </span>
              </div>
              <div>
                <span className="text-gray-500 dark:text-gray-400">Checkpointed: </span>
                <span className="font-bold text-gray-900 dark:text-white">
                  {performance.wal_checkpoint.checkpointed_pages.toLocaleString()}
                </span>
              </div>
              <div>
                <span className="text-gray-500 dark:text-gray-400">Blocked: </span>
                <span className={`font-bold ${
                  performance.wal_checkpoint.busy > 0 ? "text-yellow-600 dark:text-yellow-400" : "text-green-600 dark:text-green-400"
                }`}>
                  {performance.wal_checkpoint.busy > 0 ? "Yes" : "No"}
                </span>
              </div>
            </div>
          </div>
        )}
        {/* SQLite compile options */}
        {performance.compile_options.length > 0 && (
          <details className="mt-3">
            <summary className="text-xs font-medium text-gray-500 dark:text-gray-400 cursor-pointer hover:text-gray-700 dark:hover:text-gray-200">
              SQLite Compile Options ({performance.compile_options.length})
            </summary>
            <div className="mt-2 flex flex-wrap gap-1.5">
              {performance.compile_options.map((opt) => (
                <span
                  key={opt}
                  className="inline-block px-2 py-0.5 bg-gray-100 dark:bg-gray-700 rounded text-[10px] font-mono text-gray-600 dark:text-gray-300"
                >
                  {opt}
                </span>
              ))}
            </div>
          </details>
        )}
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

      {/* ── Backup Servers ── */}
      <Section title="Backup">
        <div className="grid grid-cols-3 gap-4 mb-4">
          <StatCard label="Servers" value={backup.server_count.toString()} />
          <StatCard label="Sync Logs" value={backup.total_sync_logs.toLocaleString()} />
          <StatCard
            label="Last Sync"
            value={backup.last_sync_at ? relativeTime(backup.last_sync_at) : "Never"}
          />
        </div>

        {/* Per-server detail cards */}
        {backup.servers.length > 0 && (
          <div className="space-y-3">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400">
              Backup Servers ({backup.servers.length})
            </p>
            {backup.servers.map((srv) => {
              const isExpanded = expandedBackupServer === srv.id;
              const statusColor = SYNC_STATUS_COLORS[srv.last_sync_status] || "text-gray-500";
              const hasRecentDiag = srv.last_diagnostics !== null;

              return (
                <div
                  key={srv.id}
                  className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden"
                >
                  {/* Server header row */}
                  <button
                    onClick={() => setExpandedBackupServer(isExpanded ? null : srv.id)}
                    className="w-full flex items-center justify-between p-3 text-left hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors"
                  >
                    <div className="flex items-center gap-3 min-w-0">
                      <div className={`w-2 h-2 rounded-full shrink-0 ${
                        srv.enabled
                          ? srv.last_sync_status === "success" ? "bg-green-500" :
                            srv.last_sync_status === "error" ? "bg-red-500" :
                            srv.last_sync_status === "running" ? "bg-blue-500 animate-pulse" :
                            "bg-yellow-500"
                          : "bg-gray-400"
                      }`} />
                      <div className="min-w-0">
                        <p className="text-sm font-semibold text-gray-900 dark:text-white truncate">
                          {srv.name}
                        </p>
                        <p className="text-xs text-gray-500 dark:text-gray-400 font-mono truncate">
                          {srv.address}
                        </p>
                      </div>
                    </div>
                    <div className="flex items-center gap-4 shrink-0">
                      <div className="text-right hidden sm:block">
                        <p className={`text-xs font-medium ${statusColor}`}>
                          {srv.last_sync_status || "never synced"}
                        </p>
                        <p className="text-[10px] text-gray-400">
                          {srv.last_sync_at ? relativeTime(srv.last_sync_at) : "no sync yet"}
                        </p>
                      </div>
                      {!srv.enabled && (
                        <span className="px-1.5 py-0.5 text-[10px] font-bold bg-gray-200 dark:bg-gray-600 text-gray-600 dark:text-gray-300 rounded">
                          DISABLED
                        </span>
                      )}
                      {hasRecentDiag && (
                        <span className="px-1.5 py-0.5 text-[10px] font-bold bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400 rounded">
                          REPORTING
                        </span>
                      )}
                      <svg
                        className={`w-4 h-4 text-gray-400 transition-transform ${isExpanded ? "rotate-180" : ""}`}
                        fill="none" stroke="currentColor" viewBox="0 0 24 24"
                      >
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                      </svg>
                    </div>
                  </button>

                  {/* Expanded details */}
                  {isExpanded && (
                    <div className="border-t border-gray-200 dark:border-gray-700 p-3 bg-gray-50 dark:bg-gray-800/50 space-y-3">
                      {/* Server config */}
                      <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-xs">
                        <div>
                          <p className="text-gray-500 dark:text-gray-400">Sync Frequency</p>
                          <p className="font-medium text-gray-900 dark:text-white">
                            Every {srv.sync_frequency_hours}h
                          </p>
                        </div>
                        <div>
                          <p className="text-gray-500 dark:text-gray-400">Status</p>
                          <p className={`font-medium ${statusColor}`}>
                            {srv.last_sync_status || "Never synced"}
                          </p>
                        </div>
                        <div>
                          <p className="text-gray-500 dark:text-gray-400">Last Sync</p>
                          <p className="font-medium text-gray-900 dark:text-white">
                            {srv.last_sync_at ? relativeTime(srv.last_sync_at) : "Never"}
                          </p>
                        </div>
                        <div>
                          <p className="text-gray-500 dark:text-gray-400">Last Report</p>
                          <p className="font-medium text-gray-900 dark:text-white">
                            {srv.last_diagnostics_at ? relativeTime(srv.last_diagnostics_at) : "None"}
                          </p>
                        </div>
                      </div>

                      {/* Sync error */}
                      {srv.last_sync_error && (
                        <div className="p-2 bg-red-50 dark:bg-red-900/20 rounded border border-red-200 dark:border-red-800">
                          <p className="text-xs text-red-700 dark:text-red-400">
                            <span className="font-medium">Last sync error: </span>
                            {srv.last_sync_error}
                          </p>
                        </div>
                      )}

                      {/* Backup server diagnostics (pushed by the backup) */}
                      {srv.last_diagnostics && (
                        <div>
                          <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
                            Server Health (reported {srv.last_diagnostics_at ? relativeTime(srv.last_diagnostics_at) : "unknown"})
                          </p>
                          <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                            {srv.last_diagnostics.version && (
                              <StatCard label="Version" value={srv.last_diagnostics.version} />
                            )}
                            {srv.last_diagnostics.uptime_seconds != null && (
                              <StatCard label="Uptime" value={formatUptime(srv.last_diagnostics.uptime_seconds)} />
                            )}
                            {srv.last_diagnostics.memory_rss_bytes != null && (
                              <StatCard label="Memory" value={formatBytes(srv.last_diagnostics.memory_rss_bytes)} />
                            )}
                            {srv.last_diagnostics.total_photos != null && (
                              <StatCard label="Photos" value={srv.last_diagnostics.total_photos.toLocaleString()} />
                            )}
                            {srv.last_diagnostics.disk_used_percent != null && (
                              <StatCard
                                label="Disk Used"
                                value={`${srv.last_diagnostics.disk_used_percent.toFixed(1)}%`}
                                color={
                                  srv.last_diagnostics.disk_used_percent > 90 ? "red" :
                                  srv.last_diagnostics.disk_used_percent > 70 ? "yellow" : "green"
                                }
                              />
                            )}
                            {srv.last_diagnostics.db_size_bytes != null && (
                              <StatCard label="DB Size" value={formatBytes(srv.last_diagnostics.db_size_bytes)} />
                            )}
                            {srv.last_diagnostics.cpu_seconds != null && (
                              <StatCard label="CPU Time" value={`${srv.last_diagnostics.cpu_seconds.toFixed(1)}s`} />
                            )}
                          </div>
                        </div>
                      )}

                      {/* Recent sync logs */}
                      {srv.recent_sync_logs.length > 0 && (
                        <div>
                          <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
                            Recent Sync History ({srv.recent_sync_logs.length})
                          </p>
                          <div className="max-h-48 overflow-y-auto rounded border border-gray-200 dark:border-gray-700">
                            <table className="w-full text-xs">
                              <thead className="bg-gray-100 dark:bg-gray-800 sticky top-0">
                                <tr>
                                  <th className="text-left px-2 py-1 font-medium">Status</th>
                                  <th className="text-left px-2 py-1 font-medium">Started</th>
                                  <th className="text-left px-2 py-1 font-medium hidden sm:table-cell">Duration</th>
                                  <th className="text-right px-2 py-1 font-medium">Photos</th>
                                  <th className="text-right px-2 py-1 font-medium hidden sm:table-cell">Size</th>
                                  <th className="text-left px-2 py-1 font-medium">Error</th>
                                </tr>
                              </thead>
                              <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
                                {srv.recent_sync_logs.map((log) => {
                                  const logStatusColor = SYNC_STATUS_COLORS[log.status] || "text-gray-500";
                                  let duration = "";
                                  if (log.completed_at && log.started_at) {
                                    const ms = new Date(log.completed_at).getTime() - new Date(log.started_at).getTime();
                                    duration = ms < 1000 ? `${ms}ms` : ms < 60000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms / 60000)}m`;
                                  }
                                  return (
                                    <tr key={log.id} className="hover:bg-gray-50 dark:hover:bg-gray-800/50">
                                      <td className={`px-2 py-1 font-medium ${logStatusColor}`}>
                                        {log.status}
                                      </td>
                                      <td className="px-2 py-1 text-gray-500" title={formatDate(log.started_at)}>
                                        {relativeTime(log.started_at)}
                                      </td>
                                      <td className="px-2 py-1 font-mono text-gray-500 hidden sm:table-cell">
                                        {duration || (log.status === "running" ? "in progress" : "-")}
                                      </td>
                                      <td className="px-2 py-1 text-right font-mono text-gray-900 dark:text-white">
                                        {log.photos_synced.toLocaleString()}
                                      </td>
                                      <td className="px-2 py-1 text-right font-mono text-gray-500 hidden sm:table-cell">
                                        {log.bytes_synced > 0 ? formatBytes(log.bytes_synced) : "-"}
                                      </td>
                                      <td className="px-2 py-1 text-red-600 dark:text-red-400 max-w-[200px] truncate" title={log.error || ""}>
                                        {log.error || ""}
                                      </td>
                                    </tr>
                                  );
                                })}
                              </tbody>
                            </table>
                          </div>
                        </div>
                      )}

                      {/* No data state */}
                      {!srv.last_diagnostics && srv.recent_sync_logs.length === 0 && (
                        <p className="text-xs text-gray-400 dark:text-gray-500 italic">
                          No diagnostics reports or sync logs received from this server yet.
                        </p>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}

        {backup.servers.length === 0 && (
          <p className="text-xs text-gray-400 dark:text-gray-500 italic">
            No backup servers configured.
          </p>
        )}
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
