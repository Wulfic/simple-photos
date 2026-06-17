/**
 * Export Downloads page — lists generated export zip files with download links.
 *
 * Navigated to from the Settings > Library Export section.
 */
import { useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { useAuthStore } from "../store/auth";
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

  const [downloading, setDownloading] = useState<string | null>(null);
  const accessToken = useAuthStore((s) => s.accessToken);

  /**
   * Trigger a streaming download via a direct anchor link.
   *
   * The previous implementation buffered the entire export zip into memory
   * via `arrayBuffer()` before constructing a Blob and clicking a link.
   * For multi-GB exports this wasted memory and made the UI appear to
   * "hang" while the whole file accumulated in RAM.
   *
   * Instead we point a hidden `<a download>` at the export endpoint with
   * `?token=<jwt>` so the server's auth middleware (which already accepts
   * the query-param token for media URLs) authenticates the request, and
   * the browser streams the response straight to disk like a normal
   * download — progress shows in the browser's download manager and
   * memory usage stays flat regardless of file size.
   */
  function handleDownload(file: ExportFile) {
    if (downloading) return;
    if (!accessToken) {
      setError("Not authenticated.");
      return;
    }
    setDownloading(file.id);
    try {
      const sep = file.download_url.includes("?") ? "&" : "?";
      // download_url is server-relative (e.g. "/api/export/files/{id}/download").
      const url = `${file.download_url}${sep}token=${encodeURIComponent(accessToken)}`;
      const a = document.createElement("a");
      a.href = url;
      a.download = file.filename;
      a.rel = "noopener";
      document.body.appendChild(a);
      a.click();
      a.remove();
    } catch (err: unknown) {
      setError(getErrorMessage(err, "Failed to start download."));
    } finally {
      // The browser owns the actual transfer once the click fires; clear
      // our local "downloading" state on the next tick so the button
      // returns to the idle state without waiting for the full transfer.
      setTimeout(() => setDownloading(null), 500);
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
            className="text-sm text-accent-600 dark:text-accent-400 hover:underline"
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
            <div className="w-8 h-8 border-4 border-accent-600 border-t-transparent rounded-full animate-spin" />
          </div>
        ) : files.length === 0 ? (
          <div className="card p-8 text-center">
            <p className="text-gray-700 dark:text-gray-400 mb-2">
              No export files available.
            </p>
            <p className="text-sm text-gray-600 dark:text-gray-500">
              Start an export from the Settings page to generate downloadable zip files.
            </p>
          </div>
        ) : (
          <>
            {/* Job status summary */}
            {job && (
              <div className="card p-4 mb-4">
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm text-gray-700 dark:text-gray-400">
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
                  className="card p-4 flex items-center justify-between"
                >
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
                      {file.filename}
                    </p>
                    <p className="text-xs text-gray-700 dark:text-gray-400">
                      {formatBytes(file.size_bytes)} — {timeRemaining(file.expires_at)}
                    </p>
                  </div>
                  <button
                    onClick={() => handleDownload(file)}
                    disabled={downloading === file.id}
                    className="btn btn-primary btn-md inline-flex items-center flex-shrink-0"
                  >
                    {downloading === file.id ? "Downloading…" : "Download"}
                  </button>
                </div>
              ))}
            </div>
          </>
        )}
      </main>
    </div>
  );
}
