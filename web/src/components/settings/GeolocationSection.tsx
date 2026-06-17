/** Geolocation settings panel — enable/disable geo, view stats, scrub data. */
import { useState, useEffect } from "react";
import { api } from "../../api/client";
import { getErrorMessage } from "../../utils/formatters";
import { Button, Toggle, StatTile } from "../ui";
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
  const [togglingPrecise, setTogglingPrecise] = useState(false);
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

  async function handlePreciseToggle() {
    if (!status) return;
    // Confirm before the first opt-in, since this changes the privacy posture.
    if (
      !status.precise_enabled &&
      !confirm(
        "Enable precise (street-level) addresses?\n\n" +
          "To resolve house-number/street addresses, your photos' GPS " +
          "coordinates will be sent to a free external geocoder " +
          "(OpenStreetMap/Photon). City-level resolution stays fully offline. " +
          "This is off by default. Continue?"
      )
    ) {
      return;
    }
    setTogglingPrecise(true);
    setError("");
    try {
      await api.geo.updateSettings({ precise_enabled: !status.precise_enabled });
      setStatus({ ...status, precise_enabled: !status.precise_enabled });
      setSuccess(
        status.precise_enabled
          ? "Precise addresses disabled. Coordinates stay on your server."
          : "Precise addresses enabled. Street addresses will resolve in the background."
      );
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setTogglingPrecise(false);
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
    <section className="card p-6 mb-4">
      <h2 className="text-lg font-semibold mb-3">Geolocation</h2>
      <p className="text-sm text-gray-700 dark:text-gray-400 mb-4">
        Automatically resolve GPS coordinates into city, state, and country
        names. Browse photos by location or timeline.
      </p>

      {/* Enable toggle */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Enable Geolocation
          </h3>
          <p className="text-xs text-gray-700 dark:text-gray-400">
            {status?.enabled
              ? "Location resolution is active."
              : "Geolocation processing is disabled."}
          </p>
        </div>
        <Toggle
          label="Enable Geolocation"
          checked={status?.enabled ?? false}
          onClick={handleToggle}
          disabled={toggling}
        />
      </div>

      {/* Scrub on upload toggle */}
      <div className="flex items-center justify-between mb-2">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Remove GPS from New Uploads
          </h3>
          <p className="text-xs text-gray-700 dark:text-gray-400">
            {status?.scrub_on_upload
              ? "New uploads will have GPS coordinates removed before saving."
              : "New uploads will keep their original GPS coordinates."}
          </p>
        </div>
        <Toggle
          label="Remove GPS from New Uploads"
          checked={status?.scrub_on_upload ?? false}
          onClick={handleScrubToggle}
          disabled={togglingScrub}
        />
      </div>
      <p className="text-xs text-gray-600 dark:text-gray-500 mb-4 ml-1">
        This only affects future uploads — photos already in your library are not changed.
        Use &quot;Scrub All&quot; below to remove GPS from existing photos.
      </p>

      {/* Precise (street-level) addresses — opt-in, contacts a third party */}
      <div className="flex items-center justify-between mb-2">
        <div>
          <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Precise Street Addresses
          </h3>
          <p className="text-xs text-gray-700 dark:text-gray-400">
            {status?.precise_enabled
              ? "Resolving house-number/street addresses (e.g. memories like “86 Nelson Blvd”)."
              : "City-level only. Turn on to resolve full street addresses."}
          </p>
        </div>
        <Toggle
          label="Precise Street Addresses"
          checked={status?.precise_enabled ?? false}
          onClick={handlePreciseToggle}
          disabled={togglingPrecise || !status?.enabled}
          title={!status?.enabled ? "Enable Geolocation first" : undefined}
        />
      </div>
      <p className="text-xs text-amber-600 dark:text-amber-500 mb-4 ml-1">
        ⚠ Privacy: when on, your photos&apos; GPS coordinates are sent to a free
        external geocoder (OpenStreetMap/Photon) to look up street addresses.
        City-level resolution always stays fully offline. Off by default.
      </p>

      {/* Status info */}
      {status && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
          <StatTile tone="accent" value={status.photos_with_location} label="With Location" />
          <StatTile tone="amber" value={status.photos_without_location} label="No Location" />
          <StatTile tone="green" value={status.unique_countries} label="Countries" />
          <StatTile tone="purple" value={status.unique_cities} label="Cities" />
        </div>
      )}

      {/* Scrub all button */}
      <Button variant="danger" onClick={handleScrubAll} disabled={scrubbing}>
        {scrubbing ? "Scrubbing..." : "Scrub All Location Data"}
      </Button>
      <p className="text-xs text-gray-700 dark:text-gray-400 mt-2">
        Permanently remove all GPS coordinates and resolved location data from your photos.
      </p>
    </section>
  );
}
