"""
E2E DDT: manual `/api/photos/upload` honors sidecar metadata override headers.

Regression for the long-standing parity gap between the "import on setup"
flow (which extracted EXIF/GPS via the autoscan pipeline) and the post-setup
manual upload paths (which bypassed it entirely by encrypting blobs
client-side). After the fix, the web client routes every manual upload
through `/api/photos/upload`, so the resulting photo row must populate the
same metadata columns regardless of whether values came from EXIF or from
sidecar-supplied override headers (X-Taken-At / X-Latitude / X-Longitude).

Each row uploads a fresh JPEG (deliberately stripped of EXIF) under a
different combination of override headers and asserts the persisted photo
record reflects the supplied values. EXIF, when present, must still win
over the override headers (lower-priority fallbacks only).

Runs against the real server fixture — no mocks.
"""

from __future__ import annotations

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── Test data ────────────────────────────────────────────────────────────────


# Each row exercises one (taken_at, lat, lon) override combination.
# `expect_*` is None when the value should NOT be set on the resulting row
# (e.g. when only one of lat/lon is supplied, or coordinates are zero).
OVERRIDE_CASES = [
    pytest.param(
        "2018-06-15T12:30:00+00:00",
        47.6062,
        -122.3321,
        "2018-06-15T12:30:00+00:00",
        47.6062,
        -122.3321,
        id="all_three_overrides_applied",
    ),
    pytest.param(
        "2010-01-02T03:04:05+00:00",
        None,
        None,
        "2010-01-02T03:04:05+00:00",
        None,
        None,
        id="taken_at_only",
    ),
    pytest.param(
        None,
        51.5074,
        -0.1278,
        None,
        51.5074,
        -0.1278,
        id="gps_only",
    ),
    pytest.param(
        None,
        0.0,
        0.0,
        None,
        None,
        None,
        id="zero_coords_dropped",
    ),
    pytest.param(
        None,
        91.0,  # out of range
        45.0,
        None,
        None,
        None,
        id="out_of_range_lat_dropped",
    ),
    pytest.param(
        None,
        45.0,
        181.0,  # out of range
        None,
        None,
        None,
        id="out_of_range_lon_dropped",
    ),
    pytest.param(
        None,
        None,
        None,
        None,
        None,
        None,
        id="no_overrides_no_metadata",
    ),
]


def _upload_with_overrides(
    client: APIClient,
    *,
    filename: str,
    taken_at: str | None,
    latitude: float | None,
    longitude: float | None,
) -> dict:
    """POST /api/photos/upload with the optional override headers."""
    headers = {
        **client._auth_headers(),
        "X-Filename": filename,
        "X-Mime-Type": "image/jpeg",
        "Content-Type": "application/octet-stream",
    }
    if taken_at is not None:
        headers["X-Taken-At"] = taken_at
    if latitude is not None:
        headers["X-Latitude"] = str(latitude)
    if longitude is not None:
        headers["X-Longitude"] = str(longitude)

    body = generate_test_jpeg(width=64, height=64)
    r = client.session.post(
        client._url("/api/photos/upload"),
        data=body,
        headers=headers,
    )
    r.raise_for_status()
    return r.json()


def _find_photo(client: APIClient, photo_id: str) -> dict:
    """Locate a photo in the user's list. Pages through results when needed."""
    cursor = None
    for _ in range(50):
        params = {"limit": 200}
        if cursor:
            params["after"] = cursor
        page = client.list_photos(**params)
        for p in page.get("photos", []):
            if p["id"] == photo_id:
                return p
        cursor = page.get("next_cursor")
        if not cursor:
            break
    raise AssertionError(f"Photo {photo_id} not found in /api/photos listing")


# ── DDT: override headers ────────────────────────────────────────────────────


@pytest.mark.parametrize(
    "taken_at,lat,lon,expect_taken_at,expect_lat,expect_lon",
    OVERRIDE_CASES,
)
def test_upload_override_headers_populate_photo(
    user_client: APIClient,
    taken_at: str | None,
    lat: float | None,
    lon: float | None,
    expect_taken_at: str | None,
    expect_lat: float | None,
    expect_lon: float | None,
):
    """
    `/api/photos/upload` populates `taken_at` / `latitude` / `longitude`
    from the override headers when the file's EXIF doesn't supply them.
    Invalid (zero / out-of-range) coordinates are silently dropped so the
    DB never contains a poisoned location.
    """
    fname = unique_filename(".jpg")
    res = _upload_with_overrides(
        user_client,
        filename=fname,
        taken_at=taken_at,
        latitude=lat,
        longitude=lon,
    )
    photo_id = res["photo_id"]

    photo = _find_photo(user_client, photo_id)

    if expect_taken_at is None:
        # Server must still populate *something* (falls back to upload time).
        # All we assert is that, with no override and no EXIF, the value
        # isn't the supplied (None) override.
        assert photo.get("taken_at"), "taken_at should default to upload time"
    else:
        # Normalize to compare just the date+time prefix; the server may
        # round-trip through a different timezone formatter.
        actual = (photo.get("taken_at") or "").replace(" ", "T")
        assert actual.startswith(expect_taken_at[:19]), (
            f"taken_at mismatch: expected {expect_taken_at!r}, got {actual!r}"
        )

    assert photo.get("latitude") == expect_lat, (
        f"latitude: expected {expect_lat}, got {photo.get('latitude')}"
    )
    assert photo.get("longitude") == expect_lon, (
        f"longitude: expected {expect_lon}, got {photo.get('longitude')}"
    )


# ── Smoke: parity with autoscan-style processing ─────────────────────────────


def test_manual_upload_creates_photos_row_with_metadata(user_client: APIClient):
    """
    The web manual-upload path now lands in the same `photos` table as the
    setup-time autoscan. Verify a single upload appears in the list with the
    expected media_type and a non-empty file_path so downstream pipelines
    (ingest encryption, geo backfill, conversion) have the row they need.
    """
    fname = unique_filename(".jpg")
    res = _upload_with_overrides(
        user_client,
        filename=fname,
        taken_at="2020-05-05T05:05:05+00:00",
        latitude=None,
        longitude=None,
    )

    photo = _find_photo(user_client, res["photo_id"])
    assert photo["media_type"] == "photo"
    assert photo["file_path"], "manual upload must register a file_path for ingest"
    assert photo["filename"] == fname


# ── Pipeline unification: upload auto-triggers encryption ───────────────────


def test_manual_upload_auto_triggers_encryption(
    user_client: APIClient,
    primary_admin: APIClient,
):
    """
    Regression for "stuck at 0/250" — manual uploads must kick off the same
    post-scan pipeline that autoscan runs (encrypt → convert), so a freshly
    uploaded photo gets encrypted without needing a settings toggle, an
    autoscan tick, or any other side-channel trigger.

    With an encryption key already configured, a manual upload should
    transition from `encrypted_blob_id == None` to a populated blob id
    on its own within a short window.
    """
    import time

    # Ensure a key is stored so encryption is allowed to run.
    primary_admin.admin_store_encryption_key("b" * 64)

    fname = unique_filename(".jpg")
    res = _upload_with_overrides(
        user_client,
        filename=fname,
        taken_at=None,
        latitude=None,
        longitude=None,
    )
    photo_id = res["photo_id"]

    # Poll encrypted_sync until our row reports a blob id (or we time out).
    deadline = time.time() + 30
    encrypted_blob_id = None
    while time.time() < deadline:
        page = user_client.encrypted_sync(limit=500)
        for p in page.get("photos", []):
            if p.get("id") == photo_id:
                encrypted_blob_id = p.get("encrypted_blob_id")
                break
        if encrypted_blob_id:
            break
        time.sleep(0.5)

    assert encrypted_blob_id, (
        f"Upload {photo_id} was not auto-encrypted within 30s — the upload "
        "endpoint is not triggering the encryption pipeline (autoscan parity "
        "regression)."
    )
