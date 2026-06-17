/** AI Recognition settings panel — enable/disable AI, view status, trigger reprocessing. */
import { useState, useEffect } from "react";
import { api } from "../../api/client";
import { getErrorMessage } from "../../utils/formatters";
import { Toggle, StatTile } from "../ui";
import type { AiStatus } from "../../api/ai";

interface AiRecognitionSectionProps {
  error: string;
  setError: (e: string) => void;
  success: string;
  setSuccess: (s: string) => void;
}

export default function AiRecognitionSection({
  setError,
  setSuccess,
}: AiRecognitionSectionProps) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [toggling, setToggling] = useState(false);
  const [reprocessing, setReprocessing] = useState(false);

  useEffect(() => {
    loadStatus();
  }, []);

  async function loadStatus() {
    try {
      const res = await api.ai.getStatus();
      setStatus(res);
      setLoaded(true);
    } catch {
      // AI endpoints may not be available
    }
  }

  async function handleToggle() {
    if (!status) return;
    setToggling(true);
    setError("");
    try {
      await api.ai.toggle(!status.enabled);
      setStatus({ ...status, enabled: !status.enabled });
      setSuccess(
        status.enabled
          ? "AI processing disabled."
          : "AI processing enabled. Photos will be analysed in the background."
      );
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setToggling(false);
    }
  }

  async function handleReprocess() {
    setReprocessing(true);
    setError("");
    try {
      const res = await api.ai.reprocess();
      setSuccess(`AI reprocessing started. ${res.cleared} photos queued.`);
      await loadStatus();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setReprocessing(false);
    }
  }

  if (!loaded) return null;

  return (
    <section className="card p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">AI Recognition</h2>
      <p className="text-sm text-gray-700 dark:text-gray-400 mb-4">
        Automatically detect faces and objects in your photos. Detected people
        are grouped into clusters and objects are tagged for easy searching.
      </p>

      {/* Enable toggle */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Enable AI Processing
          </h3>
          <p className="text-xs text-gray-700 dark:text-gray-400">
            {status?.enabled
              ? "AI is analysing your photos in the background."
              : "AI processing is disabled."}
          </p>
        </div>
        <Toggle
          label="Enable AI Processing"
          checked={status?.enabled ?? false}
          onClick={handleToggle}
          disabled={toggling}
        />
      </div>

      {/* Status info */}
      {status && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
          <StatTile tone="accent" value={status.photos_processed} label="Processed" />
          <StatTile tone="amber" value={status.photos_pending} label="Pending" />
          <StatTile tone="green" value={status.face_clusters} label="People" />
          <StatTile tone="orange" value={status.pet_clusters ?? 0} label="Pets" />
          <StatTile tone="purple" value={status.object_detections} label="Objects" />
        </div>
      )}

      {/* GPU indicator */}
      {status && (
        <p className="text-xs text-gray-700 dark:text-gray-400 mb-4">
          Execution: {status.gpu_available ? "GPU (CUDA)" : "CPU"} &middot;{" "}
          {status.face_detections} face detections &middot;{" "}
          {status.pet_detections ?? 0} pet detections
        </p>
      )}

      {/* Reprocess button */}
      <button
        onClick={handleReprocess}
        disabled={reprocessing || !status?.enabled}
        className="btn btn-primary btn-md"
      >
        {reprocessing ? (
          <span className="flex items-center gap-2">
            <span className="w-3 h-3 border-2 border-white border-t-transparent rounded-full animate-spin" />
            Reprocessing…
          </span>
        ) : (
          "Reprocess All Photos"
        )}
      </button>
    </section>
  );
}
