/** Server storage usage breakdown — displayed on Settings and Diagnostics pages. */
import { formatBytes } from "../utils/formatters";

export interface StorageStats {
  photo_bytes: number;
  photo_count: number;
  video_bytes: number;
  video_count: number;
  other_blob_bytes: number;
  other_blob_count: number;
  user_total_bytes: number;
  fs_total_bytes: number;
  fs_free_bytes: number;
}

export default function StorageStatsSection({
  stats,
  loading,
}: {
  stats: StorageStats | null;
  loading: boolean;
}) {
  if (loading) {
    return (
      <section className="card p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Storage</h2>
        <p className="text-sm text-fg-muted animate-pulse">Loading storage stats…</p>
      </section>
    );
  }

  if (!stats || stats.fs_total_bytes === 0) return null;

  const fsUsed = stats.fs_total_bytes - stats.fs_free_bytes;
  const otherUsage = Math.max(0, fsUsed - stats.user_total_bytes);

  // Percentages for the stacked bar
  const pctPhotos = stats.photo_bytes / stats.fs_total_bytes * 100;
  const pctVideos = stats.video_bytes / stats.fs_total_bytes * 100;
  const pctYou = stats.other_blob_bytes / stats.fs_total_bytes * 100;
  const pctOther = otherUsage / stats.fs_total_bytes * 100;

  return (
    <section className="card p-6 mb-4">
      <h2 className="text-lg font-semibold mb-4">Storage</h2>

      {/* Stacked usage bar */}
      <div className="w-full h-5 rounded-full overflow-hidden flex bg-edge mb-3">
        {pctPhotos > 0 && (
          <div className="bg-blue-500 h-full transition-all" style={{ width: `${pctPhotos}%` }}
            title={`Photos: ${formatBytes(stats.photo_bytes)}`} />
        )}
        {pctVideos > 0 && (
          <div className="bg-purple-500 h-full transition-all" style={{ width: `${pctVideos}%` }}
            title={`Videos: ${formatBytes(stats.video_bytes)}`} />
        )}
        {pctYou > 0 && (
          <div className="bg-cyan-500 h-full transition-all" style={{ width: `${pctYou}%` }}
            title={`Other app data: ${formatBytes(stats.other_blob_bytes)}`} />
        )}
        {pctOther > 0 && (
          <div className="bg-gray-400 dark:bg-gray-500 h-full transition-all" style={{ width: `${pctOther}%` }}
            title={`System / other: ${formatBytes(otherUsage)}`} />
        )}
        {/* Free space is the remaining background (gray-200/700) */}
      </div>

      {/* Legend */}
      <div className="grid grid-cols-2 gap-x-6 gap-y-2 text-sm mb-4">
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-blue-500 inline-block flex-shrink-0" />
          <span className="text-fg-muted">Photos</span>
          <span className="ml-auto font-medium text-fg">
            {formatBytes(stats.photo_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-purple-500 inline-block flex-shrink-0" />
          <span className="text-fg-muted">Videos</span>
          <span className="ml-auto font-medium text-fg">
            {formatBytes(stats.video_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-cyan-500 inline-block flex-shrink-0" />
          <span className="text-fg-muted">App Data</span>
          <span className="ml-auto font-medium text-fg">
            {formatBytes(stats.other_blob_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-gray-400 dark:bg-gray-500 inline-block flex-shrink-0" />
          <span className="text-fg-muted">System / Other</span>
          <span className="ml-auto font-medium text-fg">
            {formatBytes(otherUsage)}
          </span>
        </div>
      </div>

      {/* Detail rows */}
      <div className="border-t border-edge pt-3 space-y-1.5 text-sm">
        <div className="flex justify-between">
          <span className="text-fg-muted">Your usage</span>
          <span className="font-medium text-fg">{formatBytes(stats.user_total_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-fg-muted pl-3">Photos &amp; GIFs ({stats.photo_count})</span>
          <span className="text-fg-muted">{formatBytes(stats.photo_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-fg-muted pl-3">Videos ({stats.video_count})</span>
          <span className="text-fg-muted">{formatBytes(stats.video_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-fg-muted pl-3">Thumbnails &amp; manifests ({stats.other_blob_count})</span>
          <span className="text-fg-muted">{formatBytes(stats.other_blob_bytes)}</span>
        </div>
        <div className="flex justify-between pt-1.5 border-t border-edge">
          <span className="text-fg-muted">Free space</span>
          <span className="font-medium text-green-600 dark:text-green-400">{formatBytes(stats.fs_free_bytes)}</span>
        </div>
        <div className="flex justify-between">
          <span className="text-fg-muted">Total capacity</span>
          <span className="font-medium text-fg">{formatBytes(stats.fs_total_bytes)}</span>
        </div>
      </div>
    </section>
  );
}
