import { request, BASE } from "./core";
import { useAuthStore } from "../store/auth";

// ── Encryption Settings API ──────────────────────────────────────────────────

export const encryptionApi = {
  getSettings: () =>
    request<{
      encryption_mode: string;
      migration_status: string;
      migration_total: number;
      migration_completed: number;
      migration_error: string | null;
    }>("/settings/encryption"),

  setMode: (mode: "plain" | "encrypted") =>
    request<{ message: string; mode: string; migration_items: number }>(
      "/admin/encryption",
      {
        method: "PUT",
        body: JSON.stringify({ mode }),
      }
    ),

  reportProgress: (data: {
    completed_count: number;
    error?: string;
    done?: boolean;
  }) =>
    request<{ ok: boolean }>("/admin/encryption/progress", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  /** Start server-side parallel encryption migration */
  startServerMigration: (keyHex: string) =>
    request<{ message: string; total: number }>("/admin/encryption/migrate", {
      method: "POST",
      body: JSON.stringify({ key_hex: keyHex }),
    }),

  /** Fetch-based SSE migration progress stream (supports auth headers).
   *  Calls `onProgress` for each event, and `onDone` when complete. */
  streamMigrationProgress: async (
    onProgress: (data: {
      completed: number;
      total: number;
      succeeded: number;
      failed: number;
      current_file: string;
      done: boolean;
      last_error: string;
    }) => void,
    onDone: () => void,
    onError: (err: string) => void,
  ): Promise<AbortController> => {
    const controller = new AbortController();
    const { accessToken } = useAuthStore.getState();
    const headers: Record<string, string> = {
      "X-Requested-With": "SimplePhotos",
    };
    if (accessToken) headers["Authorization"] = `Bearer ${accessToken}`;

    try {
      const res = await fetch(`${BASE}/admin/encryption/migrate/stream`, {
        headers,
        signal: controller.signal,
      });
      if (!res.ok) {
        onError(`SSE stream failed: ${res.status}`);
        return controller;
      }
      const reader = res.body?.getReader();
      if (!reader) {
        onError("No response body");
        return controller;
      }
      const decoder = new TextDecoder();
      let buffer = "";

      (async () => {
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            buffer += decoder.decode(value, { stream: true });

            // Parse SSE frames
            const lines = buffer.split("\n\n");
            buffer = lines.pop() || "";
            for (const frame of lines) {
              const dataLine = frame.trim();
              if (dataLine.startsWith("data: ")) {
                try {
                  const json = JSON.parse(dataLine.slice(6));
                  onProgress(json);
                  if (json.done) {
                    onDone();
                    return;
                  }
                } catch { /* ignore malformed frame */ }
              }
            }
          }
          onDone();
        } catch (err: unknown) {
          if (controller.signal.aborted) return;
          onError(err instanceof Error ? err.message : "Stream error");
        }
      })();
    } catch (err: unknown) {
      if (controller.signal.aborted) return controller;
      onError(err instanceof Error ? err.message : "Fetch error");
    }
    return controller;
  },
};
