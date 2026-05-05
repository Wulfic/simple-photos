"""
E2E DDT: Smart location-based trip albums (`/api/geo/trips`).

A trip clusters photos taken in the same city where consecutive dates
differ by ≤ 3 days.  Trips with < 5 photos are filtered out.

We bypass the reverse-geocoder by writing `geo_city`/`geo_country*`
directly to the primary's SQLite DB, the same approach used in
`test_67_backup_extended_metadata.py`.
"""

from __future__ import annotations

import sqlite3
from datetime import datetime, timedelta, timezone

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg,
    random_username,
    unique_filename,
)
from conftest import USER_PASSWORD


# ── Helpers ────────────────────────────────────────────────────────────────


def _upload_photos(client: APIClient, count: int) -> list[str]:
    """Upload `count` photos, each with unique pixel content so the server's
    content-hash dedup does not collapse them into a single row."""
    import io
    from PIL import Image
    import secrets

    ids = []
    for _ in range(count):
        img = Image.new("RGB", (20, 20), color=(
            secrets.randbelow(256),
            secrets.randbelow(256),
            secrets.randbelow(256),
        ))
        # Plant a few random pixels so quality compression doesn't collapse
        # everything to the same JPEG bytes.
        for _ in range(8):
            img.putpixel(
                (secrets.randbelow(20), secrets.randbelow(20)),
                (secrets.randbelow(256), secrets.randbelow(256), secrets.randbelow(256)),
            )
        buf = io.BytesIO()
        img.save(buf, format="JPEG", quality=92)
        result = client.upload_photo(
            filename=unique_filename(),
            content=buf.getvalue(),
        )
        ids.append(result["photo_id"])
    return ids


def _set_geo(db_path: str, photo_id: str, *, city: str, state: str | None,
             country: str, country_code: str, taken_at: str):
    db = sqlite3.connect(db_path)
    try:
        db.execute(
            "UPDATE photos SET geo_city = ?, geo_state = ?, geo_country = ?, "
            "geo_country_code = ?, taken_at = ? WHERE id = ?",
            (city, state, country, country_code, taken_at, photo_id),
        )
        db.commit()
    finally:
        db.close()


def _iso(d: datetime) -> str:
    return d.replace(tzinfo=timezone.utc).isoformat().replace("+00:00", "Z")


# ── Fixture: build a synthetic geo dataset on the primary DB ───────────────


@pytest.fixture(scope="module")
def trips_user(primary_admin, primary_server):
    """Create a user, upload N photos, write synthetic geo data."""
    username = random_username("trips_")
    primary_admin.admin_create_user(username, USER_PASSWORD)

    client = APIClient(primary_server.base_url)
    client.login(username, USER_PASSWORD)

    # Plan:
    # • Yellowstone trip — 7 photos across 5 days (multi-day trip)
    # • Paris weekend — 6 photos across 2 days
    # • Single Paris photo 6 months later (gap > 3 days → new trip
    #   but only 1 photo, so filtered out)
    # • Tokyo same-day burst — 8 photos same date (single-day, count >= 5)
    # • Rome — 3 photos, 2 days (filtered: photos < 5)

    # Yellowstone: Jul 1, 2, 2, 3, 4, 5, 5
    yellow_dates = [
        datetime(2025, 7, 1, 10),
        datetime(2025, 7, 2, 9),
        datetime(2025, 7, 2, 14),
        datetime(2025, 7, 3, 11),
        datetime(2025, 7, 4, 13),
        datetime(2025, 7, 5, 8),
        datetime(2025, 7, 5, 17),
    ]
    yellow_ids = _upload_photos(client, len(yellow_dates))

    # Paris weekend: 2 days, 6 photos
    paris_dates = [
        datetime(2025, 9, 13, 9),
        datetime(2025, 9, 13, 11),
        datetime(2025, 9, 13, 18),
        datetime(2025, 9, 14, 10),
        datetime(2025, 9, 14, 14),
        datetime(2025, 9, 14, 20),
    ]
    paris_ids = _upload_photos(client, len(paris_dates))

    # Lone Paris (Mar 1, 2026) — 1 photo only
    lone_paris_ids = _upload_photos(client, 1)

    # Tokyo same-day: 8 photos, same date
    tokyo_dates = [datetime(2025, 11, 4, h) for h in range(8, 16)]
    tokyo_ids = _upload_photos(client, len(tokyo_dates))

    # Rome: 3 photos, 2 days — filtered (count < 5)
    rome_dates = [
        datetime(2025, 8, 1, 10),
        datetime(2025, 8, 1, 14),
        datetime(2025, 8, 2, 9),
    ]
    rome_ids = _upload_photos(client, len(rome_dates))

    db = primary_server.db_path

    for pid, d in zip(yellow_ids, yellow_dates):
        _set_geo(db, pid, city="West Yellowstone", state="Montana",
                 country="United States", country_code="US", taken_at=_iso(d))

    for pid, d in zip(paris_ids, paris_dates):
        _set_geo(db, pid, city="Paris", state="Île-de-France",
                 country="France", country_code="FR", taken_at=_iso(d))

    for pid in lone_paris_ids:
        _set_geo(db, pid, city="Paris", state="Île-de-France",
                 country="France", country_code="FR",
                 taken_at=_iso(datetime(2026, 3, 1, 12)))

    for pid, d in zip(tokyo_ids, tokyo_dates):
        _set_geo(db, pid, city="Tokyo", state=None,
                 country="Japan", country_code="JP", taken_at=_iso(d))

    for pid, d in zip(rome_ids, rome_dates):
        _set_geo(db, pid, city="Rome", state="Lazio",
                 country="Italy", country_code="IT", taken_at=_iso(d))

    return {
        "client": client,
        "yellow_ids": yellow_ids,
        "paris_ids": paris_ids,
        "lone_paris_ids": lone_paris_ids,
        "tokyo_ids": tokyo_ids,
        "rome_ids": rome_ids,
    }


# ── DDT cases — each row asserts one trip (or absence of one) ──────────────


# Each row: (city, country_code, expected_photo_count, expected_day_count)
# A `expected_photo_count` of 0 means the trip should NOT appear.

TRIP_CASES = [
    pytest.param(
        "West Yellowstone", "US", 7, 5,
        id="yellowstone_week_long",
    ),
    pytest.param(
        "Paris", "FR", 6, 2,
        id="paris_weekend",
    ),
    pytest.param(
        "Tokyo", "JP", 8, 1,
        id="tokyo_single_day_burst",
    ),
    pytest.param(
        "Rome", "IT", 0, 0,
        id="rome_filtered_too_few_photos",
    ),
]


@pytest.mark.parametrize("city,cc,expected_count,expected_days", TRIP_CASES)
def test_trip_clustering(trips_user, city, cc, expected_count, expected_days):
    client: APIClient = trips_user["client"]
    r = client.get("/api/geo/trips")
    assert r.status_code == 200, r.text
    trips = r.json()
    matching = [
        t for t in trips
        if t["city"] == city and t["country_code"] == cc
    ]
    if expected_count == 0:
        assert not matching, f"Expected no {city}/{cc} trip, found: {matching}"
        return
    assert len(matching) == 1, (
        f"Expected exactly one {city}/{cc} trip, found {len(matching)}: {matching}"
    )
    trip = matching[0]
    assert trip["photo_count"] == expected_count
    assert trip["day_count"] == expected_days


def test_trip_isolated_lone_photo_filtered(trips_user):
    """A single photo in Paris later in the year must NOT spawn a trip."""
    client: APIClient = trips_user["client"]
    trips = client.get("/api/geo/trips").json()
    paris_trips = [t for t in trips if t["city"] == "Paris"]
    # Only the weekend trip should exist; the lone Mar 2026 photo is < 5.
    assert len(paris_trips) == 1
    assert paris_trips[0]["start_date"].startswith("2025-09")


def test_trip_response_shape(trips_user):
    client: APIClient = trips_user["client"]
    trips = client.get("/api/geo/trips").json()
    yellow = next(t for t in trips if t["city"] == "West Yellowstone")
    for key in (
        "id", "name", "city", "country", "country_code",
        "start_date", "end_date", "date_label",
        "photo_count", "day_count", "first_photo_id",
    ):
        assert key in yellow, f"Missing key {key!r} in trip response"
    assert yellow["start_date"] == "2025-07-01"
    assert yellow["end_date"] == "2025-07-05"
    # Date label includes the en-dash range.
    assert "–" in yellow["date_label"] or "-" in yellow["date_label"]


def test_trip_photos_endpoint(trips_user):
    client: APIClient = trips_user["client"]
    trips = client.get("/api/geo/trips").json()
    yellow = next(t for t in trips if t["city"] == "West Yellowstone")
    r = client.get(f"/api/geo/trips/{yellow['id']}/photos")
    assert r.status_code == 200, r.text
    photos = r.json()
    assert len(photos) == 7
    returned_ids = {p["id"] for p in photos}
    assert returned_ids == set(trips_user["yellow_ids"])


def test_trip_photos_endpoint_unknown_id_404(trips_user):
    client: APIClient = trips_user["client"]
    r = client.get("/api/geo/trips/zz_unknown-city_2099-01-01_2099-01-02/photos")
    assert r.status_code == 404


def test_trips_require_auth(primary_server):
    """Unauthenticated request must be rejected."""
    anon = APIClient(primary_server.base_url)
    r = anon.get("/api/geo/trips")
    assert r.status_code in (401, 403)
