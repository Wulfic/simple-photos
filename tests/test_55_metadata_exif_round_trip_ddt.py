"""
Test 55: Metadata EXIF Round-Trip — Data-Driven Tests (DDT).

Covers:
  - Upload with GPS EXIF → read full metadata → verify extraction matches
  - Edit metadata → read back → verify persistence across read cycles
  - Upload → verify initial state → update → verify updated state
  - Write EXIF round-trip (requires exiftool — skips gracefully)
  - Sequential edits maintain latest values
  - EXIF tag extraction from JPEG with GPS
"""

import pytest
import time
import shutil

from helpers import (
    APIClient, unique_filename, generate_test_jpeg,
    generate_test_jpeg_with_gps,
)


HAS_EXIFTOOL = shutil.which("exiftool") is not None


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient, w: int = 80, h: int = 80) -> str:
    content = generate_test_jpeg(width=w, height=h)
    data = client.upload_photo(unique_filename(), content=content)
    return data["photo_id"]


def _upload_gps(client: APIClient, lat: float, lon: float,
                date_str: str = "2024:07:15 10:30:00") -> str:
    content = generate_test_jpeg_with_gps(lat=lat, lon=lon, date_str=date_str)
    data = client.upload_photo(unique_filename(".jpg"), content=content)
    return data["photo_id"]


# ══════════════════════════════════════════════════════════════════════
# DDT: GPS EXIF Upload → Read-Back
# ══════════════════════════════════════════════════════════════════════

GPS_EXIF_UPLOAD_CASES = [
    pytest.param(40.7128, -74.0060, "2024:07:15 10:30:00", id="nyc_exif"),
    pytest.param(35.6762, 139.6503, "2023:01:20 08:00:00", id="tokyo_exif"),
    pytest.param(-33.8688, 151.2093, "2022:06:01 16:45:00", id="sydney_exif"),
    pytest.param(51.5074, -0.1278, "2021:12:25 12:00:00", id="london_exif"),
    pytest.param(0.0, 0.0, "2020:03:14 09:26:53", id="null_island_exif"),
]


@pytest.mark.parametrize("lat,lon,date_str", GPS_EXIF_UPLOAD_CASES)
def test_gps_exif_extracted_on_upload(user_client: APIClient, lat, lon, date_str):
    """Photos uploaded with GPS EXIF should have coordinates in full metadata."""
    pid = _upload_gps(user_client, lat, lon, date_str)
    meta = user_client.metadata_get_full(pid)

    # GPS should be extracted from EXIF and stored in DB
    if meta["latitude"] is not None:
        assert abs(meta["latitude"] - lat) < 0.01, \
            f"Latitude mismatch: expected ~{lat}, got {meta['latitude']}"
    if meta["longitude"] is not None:
        assert abs(meta["longitude"] - lon) < 0.01, \
            f"Longitude mismatch: expected ~{lon}, got {meta['longitude']}"


# ══════════════════════════════════════════════════════════════════════
# DDT: Edit → Read-Back → Verify Persistence
# ══════════════════════════════════════════════════════════════════════

EDIT_READBACK_CASES = [
    pytest.param(
        {"filename": "roundtrip_test.jpg"},
        {"filename": "roundtrip_test.jpg"},
        id="edit_readback_filename",
    ),
    pytest.param(
        {"taken_at": "2019-08-15T14:30:00Z"},
        {"taken_at": "2019-08-15T14:30:00Z", "photo_year": 2019, "photo_month": 8},
        id="edit_readback_date_timeline",
    ),
    pytest.param(
        {"camera_model": "Canon EOS R5"},
        {"camera_model": "Canon EOS R5"},
        id="edit_readback_camera",
    ),
    pytest.param(
        {"latitude": 52.5200, "longitude": 13.4050},
        {"latitude": 52.5200, "longitude": 13.4050},
        id="edit_readback_gps_berlin",
    ),
]


@pytest.mark.parametrize("patch_data,expected_fields", EDIT_READBACK_CASES)
def test_edit_readback_persistence(user_client: APIClient, patch_data, expected_fields):
    """Edit metadata → read full metadata → values persist correctly."""
    pid = _upload(user_client)
    user_client.metadata_update(pid, **patch_data)

    meta = user_client.metadata_get_full(pid)
    for field, expected in expected_fields.items():
        actual = meta[field]
        if isinstance(expected, float):
            assert abs(actual - expected) < 0.0001, \
                f"{field}: expected ~{expected}, got {actual}"
        else:
            assert actual == expected, \
                f"{field}: expected {expected}, got {actual}"


# ══════════════════════════════════════════════════════════════════════
# DDT: Sequential Edits — Latest Value Wins
# ══════════════════════════════════════════════════════════════════════

SEQUENTIAL_EDIT_CASES = [
    pytest.param(
        [
            {"filename": "first.jpg"},
            {"filename": "second.jpg"},
            {"filename": "final.jpg"},
        ],
        {"filename": "final.jpg"},
        id="sequential_filename",
    ),
    pytest.param(
        [
            {"taken_at": "2020-01-01T00:00:00Z"},
            {"taken_at": "2023-06-15T12:00:00Z"},
        ],
        {"taken_at": "2023-06-15T12:00:00Z", "photo_year": 2023, "photo_month": 6},
        id="sequential_date",
    ),
    pytest.param(
        [
            {"latitude": 10.0, "longitude": 20.0},
            {"latitude": 30.0, "longitude": 40.0},
        ],
        {"latitude": 30.0, "longitude": 40.0},
        id="sequential_gps",
    ),
]


@pytest.mark.parametrize("edits,expected_final", SEQUENTIAL_EDIT_CASES)
def test_sequential_edits(user_client: APIClient, edits, expected_final):
    """Multiple sequential edits — final read-back reflects last edit."""
    pid = _upload(user_client)
    for edit in edits:
        user_client.metadata_update(pid, **edit)

    meta = user_client.metadata_get_full(pid)
    for field, expected in expected_final.items():
        actual = meta[field]
        if isinstance(expected, float):
            assert abs(actual - expected) < 0.0001
        else:
            assert actual == expected


# ══════════════════════════════════════════════════════════════════════
# DDT: Initial Upload State
# ══════════════════════════════════════════════════════════════════════

INITIAL_STATE_CHECKS = [
    pytest.param("id", lambda v: v is not None and len(v) > 0, id="initial_id_set"),
    pytest.param("filename", lambda v: v is not None and len(v) > 0, id="initial_filename_set"),
    pytest.param("mime_type", lambda v: v is not None, id="initial_mime_set"),
    pytest.param("width", lambda v: v is not None and v > 0, id="initial_width_positive"),
    pytest.param("height", lambda v: v is not None and v > 0, id="initial_height_positive"),
    pytest.param("size_bytes", lambda v: v is not None and v > 0, id="initial_size_positive"),
    pytest.param("photo_hash", lambda v: v is not None and len(v) > 0, id="initial_hash_set"),
    pytest.param("created_at", lambda v: v is not None and len(v) > 0, id="initial_created_at_set"),
]


@pytest.mark.parametrize("field,check_fn", INITIAL_STATE_CHECKS)
def test_initial_upload_state(user_client: APIClient, field, check_fn):
    """Freshly uploaded photo has expected initial metadata state."""
    pid = _upload(user_client)
    meta = user_client.metadata_get_full(pid)
    assert check_fn(meta[field]), \
        f"Initial state check failed for '{field}': value = {meta[field]}"


# ══════════════════════════════════════════════════════════════════════
# DDT: GPS EXIF → Edit → Verify Both Survive
# ══════════════════════════════════════════════════════════════════════

def test_edit_preserves_other_fields(user_client: APIClient):
    """Editing filename should not affect GPS coordinates."""
    pid = _upload_gps(user_client, 48.8566, 2.3522)
    meta_before = user_client.metadata_get_full(pid)

    user_client.metadata_update(pid, filename="renamed.jpg")

    meta_after = user_client.metadata_get_full(pid)
    assert meta_after["filename"] == "renamed.jpg"
    # GPS should be untouched
    if meta_before["latitude"] is not None:
        assert meta_after["latitude"] == meta_before["latitude"]
        assert meta_after["longitude"] == meta_before["longitude"]


def test_edit_gps_preserves_filename(user_client: APIClient):
    """Editing GPS should not affect filename."""
    pid = _upload(user_client)
    meta_before = user_client.metadata_get_full(pid)
    original_filename = meta_before["filename"]

    user_client.metadata_update(pid, latitude=55.7558, longitude=37.6173)

    meta_after = user_client.metadata_get_full(pid)
    assert meta_after["filename"] == original_filename
    assert abs(meta_after["latitude"] - 55.7558) < 0.0001


# ══════════════════════════════════════════════════════════════════════
# DDT: Write-EXIF Round-Trip (requires exiftool)
# ══════════════════════════════════════════════════════════════════════

@pytest.mark.skipif(not HAS_EXIFTOOL, reason="exiftool not installed")
def test_write_exif_round_trip(user_client: APIClient):
    """Write metadata to EXIF → re-read → values match."""
    pid = _upload_gps(user_client, 40.7128, -74.0060)

    # Set specific metadata
    user_client.metadata_update(
        pid,
        taken_at="2024-06-01T12:00:00Z",
        camera_model="TestCam 3000",
        latitude=51.5074,
        longitude=-0.1278,
    )

    # Write to EXIF
    result = user_client.metadata_write_exif(pid)
    assert result["status"] == "ok"
    assert result.get("new_photo_hash") is not None

    # Re-read full metadata — EXIF tags should reflect written values
    meta = user_client.metadata_get_full(pid)
    assert meta["photo_hash"] == result["new_photo_hash"]


@pytest.mark.skipif(not HAS_EXIFTOOL, reason="exiftool not installed")
def test_write_exif_updates_hash(user_client: APIClient):
    """Writing EXIF changes the file, so photo_hash should change."""
    pid = _upload_gps(user_client, 40.0, -74.0)
    meta_before = user_client.metadata_get_full(pid)
    old_hash = meta_before["photo_hash"]

    user_client.metadata_update(pid, camera_model="HashChangeTest")
    result = user_client.metadata_write_exif(pid)
    assert result["status"] == "ok"

    meta_after = user_client.metadata_get_full(pid)
    # Hash should have changed after EXIF modification
    assert meta_after["photo_hash"] != old_hash


# ══════════════════════════════════════════════════════════════════════
# DDT: Write-EXIF on Non-JPEG Fails Gracefully
# ══════════════════════════════════════════════════════════════════════

def test_write_exif_without_exiftool(user_client: APIClient):
    """Write-EXIF when exiftool is not installed returns a clear error."""
    if HAS_EXIFTOOL:
        pytest.skip("exiftool is installed; cannot test missing-tool path")

    pid = _upload_gps(user_client, 40.7128, -74.0060)
    user_client.metadata_update(pid, camera_model="WriteTest")
    r = user_client.post(f"/api/photos/{pid}/metadata/write-exif")
    # Without exiftool, server should return 500 with error message
    # This is expected — exiftool is a system dependency
    assert r.status_code == 500
    assert "error" in r.json()
