/** Geolocation settings panel — enable/disable geo, view stats, scrub data. */
import { useState, useEffect } from "react";
import { api } from "../../api/client";
import { getErrorMessage } from "../../utils/formatters";
import type { GeoStatus } from "../../api/geo";

interface GeolocationSectionProps {
  error: string;
  setError: (e: string) => void;
  success: string;
  setSuccess: (s: string) => void;
}

export default function GeolocationSection({
  setError,
  setSuccess,
}: GeolocationSectionProps) {
  const [status, setStatus] = useState<GeoStatus | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [toggling, setToggling] = useState(false);
  const [togglingScrub, setTogglingScrub] = useState(false);
  const [scrubbing, setScrubbing] = useState(false);

  useEffect(() => {
    loadStatus();
  }, []);

  async function loadStatus() {
    try {
      const res = await api.geo.getSettings();
      setStatus(res);
      setLoaded(true);
    } catch {
      // Geo endpoints may not be available
    }
  }

  async function handleToggle() {
    if (!status) return;
    setToggling(true);
    setError("");
    try {
      await api.geo.updateSettings({ enabled: !status.enabled });
      setStatus({ ...status, enabled: !status.enabled });
      setSuccess(
        status.enabled
          ? "Geolocation disabled."
          : "Geolocation enabled. Photos will be geo-tagged in the background."
      );
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setToggling(false);
    }
  }

  async function handleScrubToggle() {
    if (!status) return;
    setTogglingScrub(true);
    setError("");
    try {
      await api.geo.updateSettings({ scrub_on_upload: !status.scrub_on_upload });
      setStatus({ ...status, scrub_on_upload: !status.scrub_on_upload });
      setSuccess(
        status.scrub_on_upload
          ? "GPS scrubbing disabled. Future uploads will retain coordinates."
          : "GPS scrubbing enabled. Future uploads will have coordinates removed."
      );
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setTogglingScrub(false);
    }
  }

  async function handleScrubAll() {
    if (!confirm("This will permanently remove all geolocation data from your photos. This cannot be undone. Continue?")) return;
    setScrubbing(true);
    setError("");
    try {
      const res = await api.geo.scrubAll();
      setSuccess(`Geolocation data scrubbed from ${res.scrubbed_photos} photos.`);
      await loadStatus();
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setScrubbing(false);
    }
  }

  if (!loaded) return null;

  return (
    <section className="bg-white dark:bg-gray-800 rounded-lg shadow p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">Geolocation</h2>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Automatically resolve GPS coordinates into city, state, and country
        names. Browse photos by location or timeline.
      </p>

      {/* Enable toggle */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Enable Geolocation
          </h3>
          <p className="text-xs text-gray-500 dark:text-gray-400">
            {status?.enabled
              ? "Location resolution is active."
              : "Geolocation processing is disabled."}
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

      {/* Scrub on upload toggle */}
      <div className="flex items-center justify-between mb-2">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Remove GPS from New Uploads
          </h3>
          <p className="text-xs text-gray-500 dark:text-gray-400">
            {status?.scrub_on_upload
              ? "New uploads will have GPS coordinates removed before saving."
              : "New uploads will keep their original GPS coordinates."}
          </p>
        </div>
        <button
          onClick={handleScrubToggle}
          disabled={togglingScrub}
          className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
            status?.scrub_on_upload
              ? "bg-blue-600"
              : "bg-gray-300 dark:bg-gray-600"
          }`}
          role="switch"
          aria-checked={status?.scrub_on_upload ?? false}
        >
          <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
              status?.scrub_on_upload ? "translate-x-6" : "translate-x-1"
            }`}
          />
        </button>
      </div>
      <p className="text-xs text-gray-400 dark:text-gray-500 mb-4 ml-1">
        This only affects future uploads — photos already in your library are not changed.
        Use &quot;Scrub All&quot; below to remove GPS from existing photos.
      </p>

      {/* Status info */}
      {status && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-blue-600 dark:text-blue-400">
              {status.photos_with_location}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">With Location</p>
          </div>
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-amber-600 dark:text-amber-400">
              {status.photos_without_location}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">No Location</p>
          </div>
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-green-600 dark:text-green-400">
              {status.unique_countries}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">Countries</p>
          </div>
          <div className="bg-gray-50 dark:bg-gray-700 rounded-md p-3 text-center">
            <p className="text-xl font-bold text-purple-600 dark:text-purple-400">
              {status.unique_cities}
            </p>
            <p className="text-xs text-gray-500 dark:text-gray-400">Cities</p>
          </div>
        </div>
      )}

      {/* Scrub all button */}
      <button
        onClick={handleScrubAll}
        disabled={scrubbing}
        className="px-4 py-2 text-sm font-medium text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20 rounded-lg hover:bg-red-100 dark:hover:bg-red-900/40 disabled:opacity-50"
      >
        {scrubbing ? "Scrubbing..." : "Scrub All Location Data"}
      </button>
      <p className="text-xs text-gray-500 dark:text-gray-400 mt-2">
        Permanently remove all GPS coordinates and resolved location data from your photos.
      </p>
    </section>
  );
}
