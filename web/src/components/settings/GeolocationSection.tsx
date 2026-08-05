/** Geolocation settings panel — enable/disable geo, view stats, scrub data. */
import { useState, useEffect } from "react";
import { api } from "../../api/client";
import { getErrorMessage } from "../../utils/formatters";
import { Button, Toggle, StatTile, Select } from "../ui";
import type { GeoStatus, HomeResponse, LocationEntry } from "../../api/geo";

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

  // ── Home location ──────────────────────────────────────────────────
  const [home, setHome] = useState<HomeResponse | null>(null);
  const [cities, setCities] = useState<LocationEntry[]>([]);
  // Selected dropdown value, encoded "country_code|city" (matches a LocationEntry).
  const [homeChoice, setHomeChoice] = useState<string>("");
  const [savingHome, setSavingHome] = useState(false);

  useEffect(() => {
    loadStatus();
    loadHome();
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

  function homeKey(c: { country_code: string; city: string }): string {
    return `${c.country_code}|${c.city}`;
  }

  async function loadHome() {
    try {
      const [h, locs] = await Promise.all([
        api.geo.getHome(),
        api.geo.listLocations(),
      ]);
      setHome(h);
      setCities(locs);
      if (h.home) setHomeChoice(homeKey(h.home));
    } catch {
      // Geo endpoints may not be available
    }
  }

  async function handleSaveHome() {
    const match = cities.find((c) => homeKey(c) === homeChoice);
    if (!match) return;
    setSavingHome(true);
    setError("");
    try {
      const res = await api.geo.setHome({
        city: match.city,
        state: match.state,
        country_code: match.country_code,
      });
      setHome(res);
      setSuccess(`Home set to ${match.city}. It's now excluded from your trips.`);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setSavingHome(false);
    }
  }

  async function handleClearHome() {
    setSavingHome(true);
    setError("");
    try {
      const res = await api.geo.clearHome();
      setHome(res);
      setHomeChoice(res.home ? homeKey(res.home) : "");
      setSuccess("Home override cleared — using the inferred home city.");
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setSavingHome(false);
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
      <p className="text-sm text-fg-muted mb-4">
        Automatically resolve GPS coordinates into city, state, and country
        names. Browse photos by location or timeline.
      </p>

      {/* Enable toggle */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-medium text-fg-muted">
            Enable Geolocation
          </h3>
          <p className="text-xs text-fg-muted">
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
          <h3 className="text-sm font-medium text-fg-muted">
            Remove GPS from New Uploads
          </h3>
          <p className="text-xs text-fg-muted">
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
      <p className="text-xs text-fg-muted mb-4 ml-1">
        This only affects future uploads — photos already in your library are not changed.
        Use &quot;Scrub All&quot; below to remove GPS from existing photos.
      </p>

      {/* Precise (street-level) addresses — opt-in, contacts a third party */}
      <div className="flex items-center justify-between mb-2">
        <div>
          <h3 className="text-sm font-medium text-fg-muted">
            Precise Street Addresses
          </h3>
          <p className="text-xs text-fg-muted">
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

      {/* Home location — excluded from trip detection */}
      <div className="border-t border-edge pt-4 mb-4">
        <h3 className="text-sm font-medium text-fg-muted">Home Location</h3>
        <p className="text-xs text-fg-muted mb-3">
          {home?.source === "manual" && home.home
            ? `Home is set to ${home.home.city}. Photos taken here are excluded from your Trips.`
            : home?.source === "inferred" && home.home
              ? `We inferred your home is ${home.home.city} (your most-photographed city). Photos there are excluded from Trips. Override it below if that's wrong.`
              : "Set your home city so everyday local photos don't show up as Trips."}
        </p>
        {cities.length > 0 ? (
          <div className="flex flex-col sm:flex-row gap-2 sm:items-center">
            <Select
              fullWidth
              value={homeChoice}
              onChange={(e) => setHomeChoice(e.target.value)}
            >
              <option value="">Select your home city…</option>
              {cities.map((c) => (
                <option key={homeKey(c)} value={homeKey(c)}>
                  {c.city}
                  {c.state ? `, ${c.state}` : ""} ({c.country_code})
                </option>
              ))}
            </Select>
            <div className="flex gap-2">
              <Button
                onClick={handleSaveHome}
                disabled={
                  savingHome ||
                  !homeChoice ||
                  (home?.source === "manual" &&
                    !!home.home &&
                    homeKey(home.home) === homeChoice)
                }
              >
                {savingHome ? "Saving…" : "Set Home"}
              </Button>
              {home?.source === "manual" && (
                <Button variant="secondary" onClick={handleClearHome} disabled={savingHome}>
                  Use Inferred
                </Button>
              )}
            </div>
          </div>
        ) : (
          <p className="text-xs text-fg-muted">
            No located photos yet — once your photos have GPS data, you can pick a home city here.
          </p>
        )}
      </div>

      {/* Scrub all button */}
      <Button variant="danger" onClick={handleScrubAll} disabled={scrubbing}>
        {scrubbing ? "Scrubbing..." : "Scrub All Location Data"}
      </Button>
      <p className="text-xs text-fg-muted mt-2">
        Permanently remove all GPS coordinates and resolved location data from your photos.
      </p>
    </section>
  );
}
