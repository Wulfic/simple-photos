/** Floating banner shown while a "Save Copy" render is in progress.
 *
 *  Mirrors ConversionBanner styling — appears at the bottom of the screen
 *  with a spinner and label.  Visible whenever the processing store's
 *  "saveCopy" task is active (ffmpeg render on the server can take 30+ sec
 *  for large videos). */
import { useProcessingStore } from "../store/processing";

export default function SavingBanner() {
  const isActive = useProcessingStore((s) => s.tasks.has("saveCopy"));

  if (!isActive) return null;

  return (
    <div className="fixed bottom-20 left-4 right-4 z-50 pointer-events-none">
      <div className="pointer-events-auto max-w-md mx-auto flex items-center gap-3 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg px-4 py-3 shadow-lg">
        <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-500 border-t-blue-500 dark:border-t-blue-400 rounded-full animate-spin flex-shrink-0" />
        <p className="text-sm font-medium text-gray-700 dark:text-gray-200">
          Rendering edited copy…
        </p>
      </div>
    </div>
  );
}
