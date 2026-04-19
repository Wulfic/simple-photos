"""
Test 53: Geolocation Pipeline — End-to-End.

Verifies that:
  1. Photos uploaded with GPS EXIF retain their coordinates (when scrub is off)
  2. GPS coordinates are stripped when scrub-on-upload is enabled
  3. The geo status endpoint reflects correct photo counts
  4. The geo toggle actually updates the per-user setting
  5. Timeline (year/month) backfill works inline during upload
  6. Map endpoint returns photos with coordinates
  7. Scrub-all removes location data from existing photos
  8. Multi-user isolation: one user's scrub doesn't affect another

NOTE: The background geo-processor (reverse geocoding lat/lon → city name)
runs on a 5-minute cycle and requires the GeoNames dataset.  These tests
focus on the upload path, settings API, and coordinate storage/scrubbing
which are all synchronous.
"""

import pytest
import time

from helpers import (
    APIClient,
    unique_filename,
    generate_test_jpeg_with_gps,
    generate_test_jpeg,
    random_username,
)


# ── Coordinates for test photos ──────────────────────────────────────

# New York City (Central Park)
NYC_LAT, NYC_LON = 40.7829, -73.9654
# Tokyo (Shibuya)
TOKYO_LAT, TOKYO_LON = 35.6595, 139.7006
# London (Big Ben)
LONDON_LAT, LONDON_LON = 51.5007, -0.1246


# ══════════════════════════════════════════════════════════════════════
# Tests
# ══════════════════════════════════════════════════════════════════════

class TestGeoPipeline:
    """End-to-end tests for the geolocation pipeline."""

    def test_gps_preserved_on_upload(self, user_client: APIClient):
        """Photos with GPS EXIF should retain coordinates after upload."""
        content = generate_test_jpeg_with_gps(NYC_LAT, NYC_LON)
        data = user_client.upload_photo(unique_filename(".jpg"), content=content)
        photo_id = data["photo_id"]

        # Check geo status — should show 1 photo with location
        status = user_client.geo_settings()
        assert status["photos_with_location"] >= 1, \
            f"Expected at least 1 photo with location, got {status['photos_with_location']}"

        # Check map endpoint returns this photo
        map_photos = user_client.geo_map_photos()
        ids = [p["id"] for p in map_photos]
        assert photo_id in ids, "Uploaded GPS photo should appear in map endpoint"

        # Verify coordinates are approximately correct
        photo = next(p for p in map_photos if p["id"] == photo_id)
        assert abs(photo["latitude"] - NYC_LAT) < 0.01, \
            f"Latitude mismatch: {photo['latitude']} vs {NYC_LAT}"
        assert abs(photo["longitude"] - NYC_LON) < 0.01, \
            f"Longitude mismatch: {photo['longitude']} vs {NYC_LON}"

    def test_gps_stripped_when_scrub_enabled(self, user_client: APIClient):
        """When scrub-on-upload is enabled, GPS should be removed."""
        # Enable scrub
        r = user_client.geo_update_settings(scrub_on_upload=True)
        assert r.status_code == 200

        # Upload photo with GPS
        content = generate_test_jpeg_with_gps(TOKYO_LAT, TOKYO_LON)
        data = user_client.upload_photo(unique_filename(".jpg"), content=content)
        photo_id = data["photo_id"]

        # The photo should NOT appear in map (no coordinates)
        map_photos = user_client.geo_map_photos()
        ids = [p["id"] for p in map_photos]
        assert photo_id not in ids, \
            "Scrubbed photo should not appear in map endpoint"

        # Disable scrub for subsequent tests
        user_client.geo_update_settings(scrub_on_upload=False)

    def test_geo_toggle_persists(self, user_client: APIClient):
        """Toggling geo on and off should update the per-user setting."""
        # Default should be off (config.geo.enabled = false)
        status = user_client.geo_settings()
        assert status["enabled"] is False

        # Enable
        r = user_client.geo_update_settings(enabled=True)
        assert r.status_code == 200
        status = user_client.geo_settings()
        assert status["enabled"] is True

        # Disable
        r = user_client.geo_update_settings(enabled=False)
        assert r.status_code == 200
        status = user_client.geo_settings()
        assert status["enabled"] is False

    def test_timeline_backfill_inline(self, user_client: APIClient):
        """Year/month should be populated inline during upload (not waiting for background cycle)."""
        content = generate_test_jpeg_with_gps(LONDON_LAT, LONDON_LON,
                                               date_str="2023:12:25 14:30:00")
        user_client.upload_photo(unique_filename(".jpg"), content=content)

        # Timeline should show year 2023
        timeline = user_client.geo_timeline()
        years = [e["year"] for e in timeline]
        assert 2023 in years, f"Expected year 2023 in timeline, got {years}"

        # Drill into year — should show month 12
        months = user_client.geo_timeline_year(2023)
        month_nums = [e["month"] for e in months]
        assert 12 in month_nums, f"Expected month 12 in timeline, got {month_nums}"

    def test_no_gps_photo_excluded_from_map(self, user_client: APIClient):
        """A photo without GPS EXIF should not appear in map endpoint."""
        content = generate_test_jpeg(200, 200)
        data = user_client.upload_photo(unique_filename(".jpg"), content=content)
        photo_id = data["photo_id"]

        map_photos = user_client.geo_map_photos()
        ids = [p["id"] for p in map_photos]
        assert photo_id not in ids, "Photo without GPS should not appear in map"

    def test_status_counts_accurate(self, user_client: APIClient):
        """Status endpoint should accurately count photos with/without location."""
        # Upload one with GPS and one without
        gps_content = generate_test_jpeg_with_gps(NYC_LAT, NYC_LON)
        no_gps_content = generate_test_jpeg(200, 200)

        user_client.upload_photo(unique_filename(".jpg"), content=gps_content)
        user_client.upload_photo(unique_filename(".jpg"), content=no_gps_content)

        status = user_client.geo_settings()
        assert status["photos_with_location"] >= 1, "Should have at least 1 photo with location"
        assert status["photos_without_location"] >= 1, "Should have at least 1 photo without location"

    def test_scrub_all_removes_coordinates(self, user_client: APIClient):
        """Scrub-all should remove GPS from all existing photos."""
        # Upload a GPS photo
        content = generate_test_jpeg_with_gps(NYC_LAT, NYC_LON)
        user_client.upload_photo(unique_filename(".jpg"), content=content)

        # Verify it has coordinates
        status = user_client.geo_settings()
        assert status["photos_with_location"] >= 1

        # Scrub all
        result = user_client.geo_scrub(confirm=True)
        assert result["scrubbed_photos"] >= 1

        # Now no photos should have location
        status = user_client.geo_settings()
        assert status["photos_with_location"] == 0, \
            f"After scrub-all, expected 0 photos with location, got {status['photos_with_location']}"

    def test_multi_user_isolation(self, admin_client: APIClient, user_client: APIClient):
        """One user's geo scrub should not affect another user's photos."""
        # Both upload GPS photos
        content = generate_test_jpeg_with_gps(NYC_LAT, NYC_LON)
        admin_client.upload_photo(unique_filename(".jpg"), content=content)
        user_client.upload_photo(unique_filename(".jpg"), content=content)

        # User scrubs all
        user_client.geo_scrub(confirm=True)

        # Admin's photos should still have location
        admin_status = admin_client.geo_settings()
        assert admin_status["photos_with_location"] >= 1, \
            "Admin's GPS photos should be unaffected by user's scrub"

    def test_scrub_toggle_then_upload_preserves_after_disable(self, user_client: APIClient):
        """After disabling scrub, subsequent uploads should retain GPS."""
        # Enable scrub, upload (should strip)
        user_client.geo_update_settings(scrub_on_upload=True)
        content_a = generate_test_jpeg_with_gps(NYC_LAT, NYC_LON)
        data_a = user_client.upload_photo(unique_filename(".jpg"), content=content_a)

        # Disable scrub, upload (should preserve)
        user_client.geo_update_settings(scrub_on_upload=False)
        content_b = generate_test_jpeg_with_gps(TOKYO_LAT, TOKYO_LON)
        data_b = user_client.upload_photo(unique_filename(".jpg"), content=content_b)

        map_photos = user_client.geo_map_photos()
        ids = [p["id"] for p in map_photos]

        assert data_a["photo_id"] not in ids, "Photo uploaded with scrub ON should not be on map"
        assert data_b["photo_id"] in ids, "Photo uploaded with scrub OFF should be on map"
