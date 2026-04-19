/**
 * Geolocation & timeline API client.
 *
 * Covers geo settings, location albums, timeline views, and geo scrubbing.
 */

import { request } from "./core";

export interface GeoStatus {
  enabled: boolean;
  scrub_on_upload: boolean;
  photos_with_location: number;
  photos_without_location: number;
  unique_countries: number;
  unique_cities: number;
}

export interface LocationEntry {
  city: string;
  state: string | null;
  country: string;
  country_code: string;
  photo_count: number;
}

export interface CountryEntry {
  country: string;
  country_code: string;
  photo_count: number;
}

export interface TimelineYearEntry {
  year: number;
  photo_count: number;
}

export interface TimelineMonthEntry {
  year: number;
  month: number;
  photo_count: number;
}

export interface PhotoSummary {
  id: string;
  filename: string;
  thumb_path: string | null;
  taken_at: string | null;
  latitude: number | null;
  longitude: number | null;
}

export interface Memory {
  id: string;
  name: string;
  city: string;
  country: string;
  date_label: string;
  photo_count: number;
  first_photo_id: string | null;
  first_thumb_path: string | null;
}

export const geoApi = {
  /** Get geo settings for current user */
  getSettings: () => request<GeoStatus>("/settings/geo"),

  /** Update geo settings */
  updateSettings: (settings: { enabled?: boolean; scrub_on_upload?: boolean }) =>
    request<void>("/settings/geo", {
      method: "POST",
      body: JSON.stringify(settings),
    }),

  /** List all locations with photo counts */
  listLocations: () => request<LocationEntry[]>("/geo/locations"),

  /** List photos from a specific location */
  listLocationPhotos: (country: string, city: string) =>
    request<PhotoSummary[]>(`/geo/locations/${encodeURIComponent(country)}/${encodeURIComponent(city)}`),

  /** List countries with photo counts */
  listCountries: () => request<CountryEntry[]>("/geo/countries"),

  /** List photos with coordinates (for map view) */
  listMapPhotos: () => request<PhotoSummary[]>("/geo/map"),

  /** List timeline by year */
  listTimeline: () => request<TimelineYearEntry[]>("/geo/timeline"),

  /** List months within a year */
  listTimelineYear: (year: number) =>
    request<TimelineMonthEntry[]>(`/geo/timeline/${year}`),

  /** List photos from a specific month */
  listTimelineMonthPhotos: (year: number, month: number) =>
    request<PhotoSummary[]>(`/geo/timeline/${year}/${month}`),

  /** Scrub all geolocation data (irreversible) */
  scrubAll: () =>
    request<{ scrubbed_photos: number }>("/geo/scrub", {
      method: "POST",
      body: JSON.stringify({ confirm: true }),
    }),

  /** List auto-generated memories (photo clusters by location + date) */
  listMemories: () => request<Memory[]>("/geo/memories"),

  /** List photos in a specific memory */
  listMemoryPhotos: (memoryId: string) =>
    request<PhotoSummary[]>(`/geo/memories/${encodeURIComponent(memoryId)}/photos`),
};
