/** Floating banner shown while a "Save Copy" render is in progress.
 *
 *  Mirrors ConversionBanner styling — appears at the bottom of the screen
 *  with a spinner and label.  Visible whenever the processing store's
 *  "saveCopy" task is active (ffmpeg render on the server can take 30+ sec
 *  for large videos). */
import { useProcessingStore } from "../store/processing";
import { ProgressBanner } from "./ProgressBanner";

export default function SavingBanner() {
  const isActive = useProcessingStore((s) => s.tasks.has("saveCopy"));

  if (!isActive) return null;

  // Spinner-only (no pct, not dismissible) — the render is a single opaque job.
  return (
    <ProgressBanner
      position="bottom-20"
      tone="accent"
      label="Rendering edited copy…"
    />
  );
}
