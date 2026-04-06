/**
 * Export Downloads page — lists generated export zip files with download links.
 *
 * Navigated to from the Settings > Library Export section.
 */
import { useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import type { ExportFile, ExportJob } from "../api/export";
import AppHeader from "../components/AppHeader";
import { getErrorMessage } from "../utils/formatters";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function timeRemaining(expiresAt: string): string {
  const expires = new Date(expiresAt).getTime();
  const now = Date.now();
  const diff = expires - now;
  if (diff <= 0) return "Expired";
  const hours = Math.floor(diff / 3600000);
  const mins = Math.floor((diff % 3600000) / 60000);
  if (hours > 0) return `${hours}h ${mins}m remaining`;
  return `${mins}m remaining`;
}

export default function ExportDownloads() {
  const navigate = useNavigate();
  const [files, setFiles] = useState<ExportFile[]>([]);
  const [job, setJob] = useState<ExportJob | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    loadData();
  }, []);

  async function loadData() {
    setLoading(true);
    try {
      const res = await api.export.status();
      setJob(res.job);
      setFiles(res.files);
    } catch {
      // Try just files in case no job exists
      try {
        const res = await api.export.listFiles();
        setFiles(res.files);
      } catch {
        setFiles([]);
      }
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete() {
    if (!job) return;
    if (!confirm("Delete this export and all its files?")) return;
    try {
      await api.export.delete(job.id);
      setJob(null);
      setFiles([]);
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to delete export."));
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <AppHeader />

      <main className="max-w-2xl mx-auto p-4">
        <div className="flex items-center justify-between mb-4">
          <h1 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
            Export Downloads
          </h1>
          <button
            onClick={() => navigate("/settings")}
            className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
          >
            &larr; Back to Settings
          </button>
        </div>

        {error && (
          <p className="text-red-600 dark:text-red-400 text-sm mb-4 p-3 bg-red-50 dark:bg-red-900/30 rounded">
            {error}
          </p>
        )}

        {loading ? (
          <div className="flex items-center justify-center py-12">
            <div className="w-8 h-8 border-4 border-blue-600 border-t-transparent rounded-full animate-spin" />
          </div>
        ) : files.length === 0 ? (
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-8 text-center">
            <p className="text-gray-500 dark:text-gray-400 mb-2">
              No export files available.
            </p>
            <p className="text-sm text-gray-400 dark:text-gray-500">
              Start an export from the Settings page to generate downloadable zip files.
            </p>
          </div>
        ) : (
          <>
            {/* Job status summary */}
            {job && (
              <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 mb-4">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm text-gray-500 dark:text-gray-400">
                      Export from {new Date(job.created_at).toLocaleString()}
                    </p>
                    <p className="text-sm font-medium text-gray-700 dark:text-gray-300">
                      {files.length} file{files.length !== 1 ? "s" : ""} —{" "}
                      {formatBytes(files.reduce((sum, f) => sum + f.size_bytes, 0))} total
                    </p>
                  </div>
                  <button
                    onClick={handleDelete}
                    className="text-sm text-red-600 dark:text-red-400 hover:underline"
                  >
                    Delete Export
                  </button>
                </div>
              </div>
            )}

            {/* File list */}
            <div className="space-y-3">
              {files.map((file) => (
                <div
                  key={file.id}
                  className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 flex items-center justify-between"
                >
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
                      {file.filename}
                    </p>
                    <p className="text-xs text-gray-500 dark:text-gray-400">
                      {formatBytes(file.size_bytes)} — {timeRemaining(file.expires_at)}
                    </p>
                  </div>
                  <a
                    href={`/api${file.download_url}`}
                    download={file.filename}
                    className="inline-flex items-center gap-1.5 bg-blue-600 text-white px-3 py-1.5 rounded-md hover:bg-blue-700 text-sm flex-shrink-0"
                  >
                    Download
                  </a>
                </div>
              ))}
            </div>
          </>
        )}
      </main>
    </div>
  );
}
