/**
 * Metadata editing API client.
 *
 * Covers metadata CRUD, full EXIF retrieval, and EXIF write-back.
 */

import { request } from "./core";

export interface MetadataUpdateRequest {
  filename?: string;
  taken_at?: string;
  latitude?: number;
  longitude?: number;
  camera_model?: string;
  clear_gps?: boolean;
  camera_make?: string;
  lens_model?: string;
  iso_speed?: number;
  f_number?: number;
  exposure_time?: string;
  focal_length?: number;
  flash?: string;
  white_balance?: string;
  exposure_program?: string;
  metering_mode?: string;
  orientation?: number;
  software?: string;
  artist?: string;
  copyright?: string;
  description?: string;
  user_comment?: string;
  color_space?: string;
  exposure_bias?: number;
  scene_type?: string;
  digital_zoom?: number;
  exif_overrides?: Record<string, string>;
}

export interface MetadataUpdateResponse {
  status: string;
  updated_fields: string[];
}

export interface FullMetadataResponse {
  id: string;
  filename: string;
  mime_type: string;
  media_type: string;
  width: number;
  height: number;
  size_bytes: number;
  taken_at: string | null;
  latitude: number | null;
  longitude: number | null;
  camera_model: string | null;
  photo_hash: string | null;
  photo_subtype: string | null;
  geo_city: string | null;
  geo_state: string | null;
  geo_country: string | null;
  geo_country_code: string | null;
  photo_year: number | null;
  photo_month: number | null;
  created_at: string;
  camera_make: string | null;
  lens_model: string | null;
  iso_speed: number | null;
  f_number: number | null;
  exposure_time: string | null;
  focal_length: number | null;
  flash: string | null;
  white_balance: string | null;
  exposure_program: string | null;
  metering_mode: string | null;
  orientation: number | null;
  software: string | null;
  artist: string | null;
  copyright: string | null;
  description: string | null;
  user_comment: string | null;
  color_space: string | null;
  exposure_bias: number | null;
  scene_type: string | null;
  digital_zoom: number | null;
  exif_overrides: Record<string, string> | null;
  exif_tags: Record<string, string> | null;
}

export interface WriteExifResponse {
  status: string;
  new_photo_hash: string | null;
}

export const metadataApi = {
  /** Update metadata fields for a photo */
  update: (photoId: string, data: MetadataUpdateRequest) =>
    request<MetadataUpdateResponse>(`/photos/${photoId}/metadata`, {
      method: "PATCH",
      body: JSON.stringify(data),
    }),

  /** Get full metadata including raw EXIF tags */
  getFull: (photoId: string) =>
    request<FullMetadataResponse>(`/photos/${photoId}/metadata/full`),

  /** Write current DB metadata back to file EXIF */
  writeExif: (photoId: string) =>
    request<WriteExifResponse>(`/photos/${photoId}/metadata/write-exif`, {
      method: "POST",
    }),
};
