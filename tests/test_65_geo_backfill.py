"""E2E regression for todo P0-4: geo-backfill must populate geo_city /
geo_country for uploaded photos with GPS EXIF, and the server must clearly
report when it's running degraded because the cities500 dataset is missing.

Two layers:
  1. **Always-run**: a photo with GPS EXIF must round-trip its `latitude` /
     `longitude` fields.  This locks in that uploads parse GPS at all.
  2. **Conditional**: when `cities500.txt` is present at one of the standard
     paths, upload + a short wait must populate `geo_city`.  If absent the
     test is skipped with a pointer to `scripts/fetch_geo_data.sh`.
"""

from __future__ import annotations

import io
import os
import struct
import time
from pathlib import Path

import pytest
from PIL import Image

from helpers import APIClient, generate_test_jpeg_with_gps, unique_filename


REPO_ROOT = Path(__file__).resolve().parent.parent
GEO_DATASET_CANDIDATES = [
    REPO_ROOT / "server" / "data" / "cities500.txt",
    REPO_ROOT / "server" / "cities500.txt",
    Path("/var/lib/simple-photos/cities500.txt"),
]


def _has_geo_dataset() -> Path | None:
    for p in GEO_DATASET_CANDIDATES:
        if p.exists() and p.stat().st_size > 1024:
            return p
    env = os.environ.get("E2E_GEO_DATASET_PATH")
    if env and Path(env).exists():
        return Path(env)
    return None


def _gps_jpeg(lat: float, lon: float, width: int = 160, height: int = 120) -> bytes:
    """Build a minimal JPEG with GPS EXIF tags (delegates to helpers)."""
    return generate_test_jpeg_with_gps(lat, lon, width=width, height=height)


# ── Always-run: GPS round-trip ──────────────────────────────────────


def test_gps_exif_roundtrip(user_client: APIClient):
    """Uploading a JPEG with GPS EXIF must surface latitude/longitude in
    the photo metadata regardless of geo-dataset availability."""
    # Eiffel Tower
    content = _gps_jpeg(48.8584, 2.2945)
    data = user_client.upload_photo(unique_filename(), content=content)
    pid = data["photo_id"]

    # Wait briefly for metadata extraction.
    deadline = time.time() + 15.0
    photo = None
    while time.time() < deadline:
        listing = user_client.list_photos()
        photo = next((p for p in listing["photos"] if p["id"] == pid), None)
        if photo and photo.get("latitude") is not None:
            break
        time.sleep(0.5)

    assert photo is not None, "uploaded photo never appeared in listing"
    assert photo.get("latitude") is not None, (
        f"GPS EXIF was not extracted; photo={photo}"
    )
    assert abs(photo["latitude"] - 48.8584) < 0.01, photo["latitude"]
    assert abs(photo["longitude"] - 2.2945) < 0.01, photo["longitude"]


# ── Conditional: full reverse-geocoding ─────────────────────────────


@pytest.mark.skipif(
    _has_geo_dataset() is None,
    reason=(
        "GeoNames cities500.txt not installed.  Run "
        "scripts/fetch_geo_data.sh to download it (~10 MB), then re-run."
    ),
)
def test_geo_backfill_populates_city(user_client: APIClient):
    """When the dataset IS present, upload + wait must populate geo_city."""
    # Enable geo for this user (in case config default is off).
    try:
        user_client.put("/api/settings/geo", json_data={"enabled": True})
    except Exception:
        pass

    content = _gps_jpeg(48.8584, 2.2945)  # Paris
    data = user_client.upload_photo(unique_filename(), content=content)
    pid = data["photo_id"]

    deadline = time.time() + 60.0
    while time.time() < deadline:
        listing = user_client.list_photos()
        photo = next((p for p in listing["photos"] if p["id"] == pid), None)
        if photo and photo.get("geo_city"):
            assert photo["geo_country_code"] == "FR", photo
            return
        time.sleep(2.0)

    pytest.fail(
        f"geo_city was not populated within 60 s of upload; "
        f"check that the geo processor is enabled and cities500.txt is loaded."
    )
