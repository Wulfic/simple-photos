/** AI Recognition settings panel — enable/disable AI, view status, trigger reprocessing. */
import { useState, useEffect } from "react";
import { api } from "../../api/client";
import { getErrorMessage } from "../../utils/formatters";
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
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">AI Recognition</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Automatically detect faces and objects in your photos. Detected people
        are grouped into clusters and objects are tagged for easy searching.
      </p>

      {/* Enable toggle */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Enable AI Processing
          </h3>
          <p className="text-xs text-gray-500 dark:text-gray-400">
            {status?.enabled
              ? "AI is analysing your photos in the background."
              : "AI processing is disabled."}
          </p>
        </div>
        <button
          onClick={handleToggle}
          disabled={toggling}
          className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
            status?.enabled
              ? "bg-blue-600"
              : "bg-gray-300 dark:bg-gray-600"
          }`}
          role="switch"
          aria-checked={status?.enabled ?? false}
        >
          <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
              status?.enabled ? "translate-x-6" : "translate-x-1"
            }`}
          />
        </button>
      </div>

      {/* Status info */}
      {status && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-blue-600 dark:text-blue-400">
              {status.photos_processed}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">Processed</p>
          </div>
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-amber-600 dark:text-amber-400">
              {status.photos_pending}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">Pending</p>
          </div>
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-green-600 dark:text-green-400">
              {status.face_clusters}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">People</p>
          </div>
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-purple-600 dark:text-purple-400">
              {status.object_detections}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">Objects</p>
          </div>
        </div>
      )}

      {/* GPU indicator */}
      {status && (
        <p className="text-xs text-gray-500 dark:text-gray-400 mb-4">
          Execution: {status.gpu_available ? "GPU (CUDA)" : "CPU"} &middot;{" "}
          {status.face_detections} face detections total
        </p>
      )}

      {/* Reprocess button */}
      <button
        onClick={handleReprocess}
        disabled={reprocessing || !status?.enabled}
        className="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 disabled:opacity-50 text-sm"
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
