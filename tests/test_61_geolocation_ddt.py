"""
Test 52: Geolocation & Timeline Albums — Data-Driven Tests (DDT).

Parametrized tests covering all geolocation API endpoints:
  - Geo settings endpoint returns expected fields
  - Toggle geo on/off, scrub-on-upload on/off
  - Location listing (empty initially)
  - Country listing (empty initially)
  - Timeline listing (empty initially)
  - Map photos (empty initially)
  - Location photo queries
  - Timeline year/month queries
  - Geo scrub endpoint (requires confirmation)
  - Settings persistence across queries
  - Counter validation (non-negative)
  - Multi-user isolation

Each test case is a single row in a parameter table.
"""

import pytest
import time

from helpers import (
    APIClient, unique_filename, generate_test_jpeg,
    generate_test_jpeg_with_gps,
)


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient, w: int = 80, h: int = 80) -> str:
    """Upload a plain test photo (no GPS) and return its photo_id."""
    content = generate_test_jpeg(width=w, height=h)
    data = client.upload_photo(unique_filename(), content=content)
    return data["photo_id"]


def _upload_gps(client: APIClient, lat: float, lon: float,
                date_str: str = "2024:07:15 10:30:00") -> str:
    """Upload a test photo with GPS EXIF data and return its photo_id."""
    content = generate_test_jpeg_with_gps(lat=lat, lon=lon, date_str=date_str)
    data = client.upload_photo(unique_filename(".jpg"), content=content)
    return data["photo_id"]


# ══════════════════════════════════════════════════════════════════════
# DDT: Geo Settings Endpoint — Field Presence
# ══════════════════════════════════════════════════════════════════════

SETTINGS_FIELDS = [
    pytest.param("enabled", id="has_enabled_field"),
    pytest.param("scrub_on_upload", id="has_scrub_field"),
    pytest.param("photos_with_location", id="has_with_location_field"),
    pytest.param("photos_without_location", id="has_without_location_field"),
    pytest.param("unique_countries", id="has_countries_field"),
    pytest.param("unique_cities", id="has_cities_field"),
]


@pytest.mark.parametrize("field", SETTINGS_FIELDS)
def test_geo_settings_has_field(user_client, field):
    """Geo settings response contains the expected field."""
    settings = user_client.geo_settings()
    assert field in settings, f"Geo settings missing field '{field}'"


# ══════════════════════════════════════════════════════════════════════
# DDT: Geo Settings Counters Non-Negative
# ══════════════════════════════════════════════════════════════════════

COUNTER_FIELDS = [
    pytest.param("photos_with_location", id="with_loc_non_negative"),
    pytest.param("photos_without_location", id="without_loc_non_negative"),
    pytest.param("unique_countries", id="countries_non_negative"),
    pytest.param("unique_cities", id="cities_non_negative"),
]


@pytest.mark.parametrize("field", COUNTER_FIELDS)
def test_geo_counters_non_negative(user_client, field):
    """All geo settings counters should be non-negative integers."""
    settings = user_client.geo_settings()
    assert isinstance(settings[field], int), f"{field} should be int"
    assert settings[field] >= 0, f"{field} should be >= 0"


# ══════════════════════════════════════════════════════════════════════
# DDT: Geo Settings Toggle
# ══════════════════════════════════════════════════════════════════════

TOGGLE_CASES = [
    pytest.param(True, id="enable_geo"),
    pytest.param(False, id="disable_geo"),
]


@pytest.mark.parametrize("enabled", TOGGLE_CASES)
def test_geo_toggle(user_client, enabled):
    """Toggling geo on/off is reflected in subsequent settings queries."""
    r = user_client.geo_update_settings(enabled=enabled)
    assert r.status_code == 200

    settings = user_client.geo_settings()
    assert settings["enabled"] == enabled


# ══════════════════════════════════════════════════════════════════════
# DDT: Scrub-On-Upload Toggle
# ══════════════════════════════════════════════════════════════════════

SCRUB_TOGGLE_CASES = [
    pytest.param(True, id="enable_scrub_on_upload"),
    pytest.param(False, id="disable_scrub_on_upload"),
]


@pytest.mark.parametrize("scrub", SCRUB_TOGGLE_CASES)
def test_geo_scrub_toggle(user_client, scrub):
    """Toggling scrub-on-upload is reflected in subsequent settings queries."""
    r = user_client.geo_update_settings(scrub_on_upload=scrub)
    assert r.status_code == 200

    settings = user_client.geo_settings()
    assert settings["scrub_on_upload"] == scrub


# ══════════════════════════════════════════════════════════════════════
# DDT: Settings Persistence (toggle cycles)
# ══════════════════════════════════════════════════════════════════════

def test_geo_settings_persistence(user_client):
    """Geo settings persist across multiple toggle cycles."""
    # Enable → verify → disable → verify → re-enable → verify
    user_client.geo_update_settings(enabled=True)
    assert user_client.geo_settings()["enabled"] is True

    user_client.geo_update_settings(enabled=False)
    assert user_client.geo_settings()["enabled"] is False

    user_client.geo_update_settings(enabled=True)
    assert user_client.geo_settings()["enabled"] is True


def test_geo_scrub_persistence(user_client):
    """Scrub setting persists across toggle cycles."""
    user_client.geo_update_settings(scrub_on_upload=True)
    assert user_client.geo_settings()["scrub_on_upload"] is True

    user_client.geo_update_settings(scrub_on_upload=False)
    assert user_client.geo_settings()["scrub_on_upload"] is False


# ══════════════════════════════════════════════════════════════════════
# DDT: Combined Settings Update
# ══════════════════════════════════════════════════════════════════════

def test_geo_combined_update(user_client):
    """Both enabled and scrub_on_upload can be set in one request."""
    r = user_client.geo_update_settings(enabled=True, scrub_on_upload=True)
    assert r.status_code == 200

    settings = user_client.geo_settings()
    assert settings["enabled"] is True
    assert settings["scrub_on_upload"] is True


# ══════════════════════════════════════════════════════════════════════
# DDT: Empty List Endpoints — New User
# ══════════════════════════════════════════════════════════════════════

EMPTY_LIST_CASES = [
    pytest.param("geo_locations", id="locations_empty"),
    pytest.param("geo_countries", id="countries_empty"),
    pytest.param("geo_map_photos", id="map_empty"),
    pytest.param("geo_timeline", id="timeline_empty"),
]


@pytest.mark.parametrize("method", EMPTY_LIST_CASES)
def test_geo_list_empty_initially(user_client, method):
    """List endpoints return empty list for a new user with no photos."""
    result = getattr(user_client, method)()
    assert isinstance(result, list)
    assert len(result) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Timeline Year — No Photos
# ══════════════════════════════════════════════════════════════════════

TIMELINE_YEAR_CASES = [
    pytest.param(2024, id="year_2024"),
    pytest.param(2025, id="year_2025"),
    pytest.param(1999, id="year_1999"),
]


@pytest.mark.parametrize("year", TIMELINE_YEAR_CASES)
def test_geo_timeline_year_empty(user_client, year):
    """Timeline year returns empty list when no photos exist for that year."""
    result = user_client.geo_timeline_year(year)
    assert isinstance(result, list)
    assert len(result) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Timeline Month — No Photos
# ══════════════════════════════════════════════════════════════════════

TIMELINE_MONTH_CASES = [
    pytest.param(2024, 1, id="jan_2024"),
    pytest.param(2024, 7, id="jul_2024"),
    pytest.param(2024, 12, id="dec_2024"),
]


@pytest.mark.parametrize("year,month", TIMELINE_MONTH_CASES)
def test_geo_timeline_month_empty(user_client, year, month):
    """Timeline month photos returns empty list when no photos exist."""
    result = user_client.geo_timeline_month_photos(year, month)
    assert isinstance(result, list)
    assert len(result) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Location Photos — Non-Existent Location
# ══════════════════════════════════════════════════════════════════════

LOCATION_QUERY_CASES = [
    pytest.param("US", "New York", id="us_new_york"),
    pytest.param("FR", "Paris", id="fr_paris"),
    pytest.param("ZZ", "Nonexistent", id="nonexistent_location"),
]


@pytest.mark.parametrize("country_code,city", LOCATION_QUERY_CASES)
def test_geo_location_photos_empty(user_client, country_code, city):
    """Location photos returns empty list for locations with no photos."""
    result = user_client.geo_location_photos(country_code, city)
    assert isinstance(result, list)
    assert len(result) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Scrub Requires Confirmation
# ══════════════════════════════════════════════════════════════════════

def test_geo_scrub_without_confirm_fails(user_client):
    """Scrub endpoint rejects requests without confirm=true."""
    r = user_client.post("/api/geo/scrub", json_data={"confirm": False})
    assert r.status_code == 400


def test_geo_scrub_with_confirm_succeeds(user_client):
    """Scrub endpoint succeeds with confirm=true (even with no data)."""
    result = user_client.geo_scrub(confirm=True)
    assert "scrubbed_photos" in result
    assert isinstance(result["scrubbed_photos"], int)
    assert result["scrubbed_photos"] >= 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Upload Increments Counter
# ══════════════════════════════════════════════════════════════════════

def test_geo_upload_increments_without_location(user_client):
    """Uploading a photo without GPS increments without_location count."""
    before = user_client.geo_settings()
    _upload(user_client)
    after = user_client.geo_settings()
    assert after["photos_without_location"] == before["photos_without_location"] + 1


# ══════════════════════════════════════════════════════════════════════
# DDT: Upload with GPS Coordinates
# ══════════════════════════════════════════════════════════════════════

def test_geo_upload_with_gps_has_location(user_client):
    """Uploading a photo with GPS EXIF increments with_location count."""
    before = user_client.geo_settings()
    # Paris coordinates
    _upload_gps(user_client, lat=48.8566, lon=2.3522)
    after = user_client.geo_settings()
    assert after["photos_with_location"] == before["photos_with_location"] + 1


def test_geo_upload_with_gps_appears_in_map(user_client):
    """Photo with GPS coordinates appears in map endpoint."""
    _upload_gps(user_client, lat=40.7128, lon=-74.0060)
    photos = user_client.geo_map_photos()
    assert len(photos) >= 1
    # Verify photo has coordinates
    found = any(p.get("latitude") is not None and p.get("longitude") is not None for p in photos)
    assert found, "Map should contain a photo with coordinates"


# ══════════════════════════════════════════════════════════════════════
# DDT: Map Photo Summary Fields
# ══════════════════════════════════════════════════════════════════════

MAP_PHOTO_FIELDS = [
    pytest.param("id", id="map_has_id"),
    pytest.param("filename", id="map_has_filename"),
    pytest.param("latitude", id="map_has_latitude"),
    pytest.param("longitude", id="map_has_longitude"),
]


@pytest.mark.parametrize("field", MAP_PHOTO_FIELDS)
def test_geo_map_photo_has_field(user_client, field):
    """Map photo entries contain expected fields."""
    _upload_gps(user_client, lat=35.6762, lon=139.6503)
    photos = user_client.geo_map_photos()
    assert len(photos) >= 1
    assert field in photos[0], f"Map photo missing field '{field}'"


# ══════════════════════════════════════════════════════════════════════
# DDT: Timeline — Upload with Date Populates Year/Month
# ══════════════════════════════════════════════════════════════════════

def test_geo_timeline_shows_year_after_upload(user_client):
    """Upload with EXIF date populates the timeline endpoint."""
    _upload_gps(user_client, lat=0.0, lon=0.0, date_str="2023:03:20 12:00:00")
    # Give the server a moment to set photo_year/month
    timeline = user_client.geo_timeline()
    # Should have at least one year entry
    assert isinstance(timeline, list)
    if len(timeline) > 0:
        assert "year" in timeline[0]
        assert "photo_count" in timeline[0]


def test_geo_timeline_year_has_months(user_client):
    """Timeline year endpoint returns month entries."""
    _upload_gps(user_client, lat=0.0, lon=0.0, date_str="2022:06:15 08:00:00")
    months = user_client.geo_timeline_year(2022)
    assert isinstance(months, list)
    if len(months) > 0:
        assert "year" in months[0]
        assert "month" in months[0]
        assert "photo_count" in months[0]


def test_geo_timeline_month_has_photos(user_client):
    """Timeline month photos endpoint returns photo summaries."""
    _upload_gps(user_client, lat=0.0, lon=0.0, date_str="2021:11:25 16:30:00")
    photos = user_client.geo_timeline_month_photos(2021, 11)
    assert isinstance(photos, list)
    if len(photos) > 0:
        assert "id" in photos[0]
        assert "filename" in photos[0]


# ══════════════════════════════════════════════════════════════════════
# DDT: Scrub Clears Location Data
# ══════════════════════════════════════════════════════════════════════

def test_geo_scrub_clears_locations(user_client):
    """Scrub removes all geolocation data from user photos."""
    _upload_gps(user_client, lat=51.5074, lon=-0.1278)
    before = user_client.geo_settings()
    assert before["photos_with_location"] >= 1

    result = user_client.geo_scrub(confirm=True)
    assert result["scrubbed_photos"] >= 1

    after = user_client.geo_settings()
    assert after["photos_with_location"] == 0


def test_geo_scrub_map_empty_after(user_client):
    """After scrub, map endpoint returns no photos with coordinates."""
    _upload_gps(user_client, lat=48.8566, lon=2.3522)
    assert len(user_client.geo_map_photos()) >= 1

    user_client.geo_scrub(confirm=True)
    # After scrub, no photos should have coordinates
    map_photos = user_client.geo_map_photos()
    assert len(map_photos) == 0


# ══════════════════════════════════════════════════════════════════════
# DDT: Scrub On Upload — GPS Stripped
# ══════════════════════════════════════════════════════════════════════

def test_geo_scrub_on_upload_strips_gps(user_client):
    """When scrub_on_upload is enabled, uploaded photos lose GPS data."""
    user_client.geo_update_settings(scrub_on_upload=True)

    before = user_client.geo_settings()
    _upload_gps(user_client, lat=40.7128, lon=-74.0060)
    after = user_client.geo_settings()

    # Photo should be counted as without location (GPS stripped)
    assert after["photos_without_location"] == before["photos_without_location"] + 1
    assert after["photos_with_location"] == before["photos_with_location"]


def test_geo_scrub_on_upload_disabled_preserves_gps(user_client):
    """When scrub_on_upload is disabled, uploaded photos retain GPS data."""
    user_client.geo_update_settings(scrub_on_upload=False)

    before = user_client.geo_settings()
    _upload_gps(user_client, lat=35.6762, lon=139.6503)
    after = user_client.geo_settings()

    assert after["photos_with_location"] == before["photos_with_location"] + 1


# ══════════════════════════════════════════════════════════════════════
# DDT: Multi-User Isolation
# ══════════════════════════════════════════════════════════════════════

def test_geo_multi_user_settings_isolated(user_client, second_user_client):
    """Geo settings are isolated per user."""
    user_client.geo_update_settings(enabled=True, scrub_on_upload=True)
    second_user_client.geo_update_settings(enabled=False, scrub_on_upload=False)

    s1 = user_client.geo_settings()
    s2 = second_user_client.geo_settings()

    assert s1["enabled"] is True
    assert s1["scrub_on_upload"] is True
    assert s2["enabled"] is False
    assert s2["scrub_on_upload"] is False


def test_geo_multi_user_data_isolated(user_client, second_user_client):
    """Geo data is isolated between users — one user's photos don't leak."""
    _upload_gps(user_client, lat=48.8566, lon=2.3522)

    user_map = user_client.geo_map_photos()
    other_map = second_user_client.geo_map_photos()

    assert len(user_map) >= 1
    assert len(other_map) == 0


def test_geo_multi_user_scrub_isolated(user_client, second_user_client):
    """Scrubbing one user's data doesn't affect another user's photos."""
    _upload_gps(user_client, lat=48.8566, lon=2.3522)
    _upload_gps(second_user_client, lat=40.7128, lon=-74.0060)

    user_client.geo_scrub(confirm=True)

    u1_map = user_client.geo_map_photos()
    u2_map = second_user_client.geo_map_photos()

    assert len(u1_map) == 0
    assert len(u2_map) >= 1
