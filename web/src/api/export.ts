/**
 * Library export API client — trigger exports, check status, list/download files.
 *
 * Maps to server routes: `/api/export/*`
 */
import { request } from "./core";

export interface ExportJob {
  id: string;
  status: "pending" | "running" | "completed" | "failed";
  size_limit: number;
  created_at: string;
  completed_at: string | null;
  error: string | null;
}

export interface ExportFile {
  id: string;
  job_id: string;
  filename: string;
  size_bytes: number;
  created_at: string;
  expires_at: string;
  download_url: string;
}

export const exportApi = {
  /** Start a new library export job */
  start: (sizeLimitBytes: number) =>
    request<ExportJob>("/export", {
      method: "POST",
      body: JSON.stringify({ size_limit: sizeLimitBytes }),
    }),

  /** Get latest export job status + files */
  status: () =>
    request<{ job: ExportJob; files: ExportFile[] }>("/export/status"),

  /** List all non-expired export files */
  listFiles: () =>
    request<{ files: ExportFile[] }>("/export/files"),

  /** Delete an export job and its files */
  delete: (jobId: string) =>
    request<void>(`/export/${jobId}`, { method: "DELETE" }),
};
