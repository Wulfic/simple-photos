import { formatBytes } from "../utils/formatters";

export interface StorageStats {
  photo_bytes: number;
  photo_count: number;
  video_bytes: number;
  video_count: number;
  other_blob_bytes: number;
  other_blob_count: number;
  plain_bytes: number;
  plain_count: number;
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
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
        <h2 className="text-lg font-semibold mb-3">Storage</h2>
        <p className="text-sm text-gray-400 animate-pulse">Loading storage stats…</p>
      </section>
    );
  }

  if (!stats || stats.fs_total_bytes === 0) return null;

  const fsUsed = stats.fs_total_bytes - stats.fs_free_bytes;
  const otherUsage = Math.max(0, fsUsed - stats.user_total_bytes);

  // Percentages for the stacked bar
  const pctPhotos = (stats.photo_bytes + stats.plain_bytes) / stats.fs_total_bytes * 100;
  const pctVideos = stats.video_bytes / stats.fs_total_bytes * 100;
  const pctYou = stats.other_blob_bytes / stats.fs_total_bytes * 100;
  const pctOther = otherUsage / stats.fs_total_bytes * 100;

  const totalPhotoCount = stats.photo_count + stats.plain_count;
  const totalPhotoBytes = stats.photo_bytes + stats.plain_bytes;

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-4">Storage</h2>

      {/* Stacked usage bar */}
      <div className="w-full h-5 rounded-full overflow-hidden flex bg-gray-200 dark:bg-gray-700 mb-3">
        {pctPhotos > 0 && (
          <div className="bg-blue-500 h-full transition-all" style={{ width: `${pctPhotos}%` }}
            title={`Photos: ${formatBytes(totalPhotoBytes)}`} />
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
          <span className="text-gray-600 dark:text-gray-400">Photos</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(totalPhotoBytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-purple-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">Videos</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(stats.video_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-cyan-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">App Data</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(stats.other_blob_bytes)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-3 h-3 rounded-full bg-gray-400 dark:bg-gray-500 inline-block flex-shrink-0" />
          <span className="text-gray-600 dark:text-gray-400">System / Other</span>
          <span className="ml-auto font-medium text-gray-900 dark:text-gray-100">
            {formatBytes(otherUsage)}
          </span>
        </div>
      </div>

      {/* Detail rows */}
      <div className="border-t border-gray-100 dark:border-gray-700 pt-3 space-y-1.5 text-sm">
        <div className="flex justify-between">
          <span className="text-gray-500 dark:text-gray-400">Your usage</span>
          <span className="font-medium text-gray-900 dark:text-gray-100">{formatBytes(stats.user_total_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-gray-400 dark:text-gray-500 pl-3">Photos &amp; GIFs ({totalPhotoCount})</span>
          <span className="text-gray-500 dark:text-gray-400">{formatBytes(totalPhotoBytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-gray-400 dark:text-gray-500 pl-3">Videos ({stats.video_count})</span>
          <span className="text-gray-500 dark:text-gray-400">{formatBytes(stats.video_bytes)}</span>
        </div>
        <div className="flex justify-between text-xs">
          <span className="text-gray-400 dark:text-gray-500 pl-3">Thumbnails &amp; manifests ({stats.other_blob_count})</span>
          <span className="text-gray-500 dark:text-gray-400">{formatBytes(stats.other_blob_bytes)}</span>
        </div>
        <div className="flex justify-between pt-1.5 border-t border-gray-100 dark:border-gray-700">
          <span className="text-gray-500 dark:text-gray-400">Free space</span>
          <span className="font-medium text-green-600 dark:text-green-400">{formatBytes(stats.fs_free_bytes)}</span>
        </div>
        <div className="flex justify-between">
          <span className="text-gray-500 dark:text-gray-400">Total capacity</span>
          <span className="font-medium text-gray-900 dark:text-gray-100">{formatBytes(stats.fs_total_bytes)}</span>
        </div>
      </div>
    </section>
  );
}
