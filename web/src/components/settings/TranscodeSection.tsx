import { useState, useEffect } from "react";
import { transcodeApi, TranscodeStatusResponse } from "../../api/transcode";

const ACCEL_LABELS: Record<string, string> = {
  nvenc: "NVIDIA NVENC",
  qsv: "Intel Quick Sync",
  vaapi: "VA-API",
  amf: "AMD AMF",
  cpu: "CPU (libx264)",
};

export default function TranscodeSection() {
  const [status, setStatus] = useState<TranscodeStatusResponse | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    transcodeApi
      .getStatus()
      .then(setStatus)
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">Video Transcoding</h2>

      {loading && (
        <p className="text-sm text-gray-500 dark:text-gray-400">Loading...</p>
      )}

      {status && (
        <div className="space-y-2 text-sm">
          <div className="flex justify-between">
            <span className="text-gray-600 dark:text-gray-400">Acceleration</span>
            <span className="font-medium">
              {status.gpu_available ? (
                <span className="text-green-600 dark:text-green-400">
                  {ACCEL_LABELS[status.accel_type] ?? status.accel_type}
                </span>
              ) : (
                <span className="text-gray-600 dark:text-gray-400">CPU Only</span>
              )}
            </span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-600 dark:text-gray-400">Encoder</span>
            <span className="font-mono text-xs">{status.video_encoder}</span>
          </div>
          {status.device && (
            <div className="flex justify-between">
              <span className="text-gray-600 dark:text-gray-400">Device</span>
              <span className="font-mono text-xs">{status.device}</span>
            </div>
          )}
          <div className="flex justify-between">
            <span className="text-gray-600 dark:text-gray-400">GPU Enabled</span>
            <span>{status.gpu_enabled ? "Yes" : "No"}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-600 dark:text-gray-400">CPU Fallback</span>
            <span>{status.fallback_to_cpu ? "Yes" : "No"}</span>
          </div>
        </div>
      )}

      {!loading && !status && (
        <p className="text-sm text-gray-500 dark:text-gray-400">
          Unable to fetch transcode status.
        </p>
      )}
    </section>
  );
}
