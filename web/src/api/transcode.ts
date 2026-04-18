import { request } from "./core";

export interface TranscodeStatusResponse {
  gpu_available: boolean;
  accel_type: string;
  video_encoder: string;
  device: string | null;
  gpu_enabled: boolean;
  fallback_to_cpu: boolean;
}

export const transcodeApi = {
  async getStatus(): Promise<TranscodeStatusResponse> {
    return request<TranscodeStatusResponse>("/api/transcode/status");
  },
};
