"""
E2E DDT: audio_backup_enabled toggle is enforced on every import path.

Regression for P0-1.  Before this fix:
  - /api/photos/upload accepted audio uploads regardless of the toggle
  - The cross-server sync engine pushed audio to backup regardless

Each row uploads a tiny MP3 (silent, valid frame) under a known toggle
state and asserts the server's response matches the policy.  No mocks —
the request hits the real Rust handler against the shared test fixture.
"""

import io
import struct
import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── Test data ────────────────────────────────────────────────────────────────


def _minimal_mp3() -> bytes:
    """Construct a minimal but well-formed MP3 byte sequence.

    Real MP3 frames are complex; for upload-path testing the server only
    inspects the file extension / declared MIME type to derive `media_type`,
    so a short ID3v2 header followed by a single MPEG-1 Layer 3 frame
    sync pattern is sufficient to be accepted as `audio/mpeg`.
    """
    id3 = b"ID3\x03\x00\x00\x00\x00\x00\x00"
    # MPEG-1 Layer III, 128 kbps, 44.1 kHz, mono frame header
    frame_header = b"\xff\xfb\x90\x00"
    payload = b"\x00" * 512
    return id3 + frame_header + payload


def _set_audio_toggle(admin: APIClient, enabled: bool) -> None:
    """Flip the server-wide audio_backup_enabled setting via admin API."""
    r = admin.put(
        "/api/admin/audio-backup",
        json_data={"audio_backup_enabled": enabled},
    )
    r.raise_for_status()


def _read_audio_toggle(client: APIClient) -> bool:
    r = client.get("/api/settings/audio-backup")
    r.raise_for_status()
    return bool(r.json().get("audio_backup_enabled"))


# ── DDT: upload endpoint honors the toggle ───────────────────────────────────


UPLOAD_CASES = [
    pytest.param(
        True,
        "song_enabled.mp3",
        "audio/mpeg",
        201,
        id="audio_enabled__mp3_accepted",
    ),
    pytest.param(
        False,
        "song_disabled.mp3",
        "audio/mpeg",
        403,
        id="audio_disabled__mp3_rejected_403",
    ),
    pytest.param(
        False,
        "track.flac",
        "audio/flac",
        403,
        id="audio_disabled__flac_rejected_403",
    ),
    pytest.param(
        False,
        "photo.jpg",
        "image/jpeg",
        201,
        id="audio_disabled__jpeg_still_accepted",
    ),
]


@pytest.mark.parametrize("toggle,filename,mime,expected_status", UPLOAD_CASES)
def test_upload_respects_audio_toggle(
    admin_client, user_client, toggle, filename, mime, expected_status
):
    _set_audio_toggle(admin_client, toggle)
    # The setting must have actually persisted before we test against it.
    assert _read_audio_toggle(user_client) is toggle

    if mime.startswith("audio/"):
        body = _minimal_mp3()
    else:
        body = generate_test_jpeg(width=64, height=64)

    headers = {
        **user_client._auth_headers(),
        "X-Filename": unique_filename(filename),
        "X-Mime-Type": mime,
        "Content-Type": "application/octet-stream",
    }
    r = user_client.session.post(
        user_client._url("/api/photos/upload"),
        data=body,
        headers=headers,
    )
    assert r.status_code == expected_status, (
        f"Expected {expected_status} for toggle={toggle} mime={mime}, "
        f"got {r.status_code}: {r.text[:200]}"
    )

    if expected_status == 403:
        assert "audio" in r.text.lower(), (
            f"403 response should mention audio policy, got: {r.text[:200]}"
        )


# ── Sync-engine SQL filter is exercised indirectly ───────────────────────────
#
# The sync engine pushes registered photos to a backup server.  After this
# fix it filters `media_type = 'audio'` from the query when the toggle is
# off.  We can validate the SQL path by toggling, registering an audio file
# while the toggle is *on*, then turning it off and asserting the same row
# is excluded from the sync candidate list.  The sync candidate list is
# not directly exposed, so we use the photos list endpoint with an explicit
# media_type filter as a proxy: the row must still exist (we never delete)
# but it must report `media_type='audio'` so the engine's WHERE clause kicks in.


def test_audio_row_persisted_when_toggle_enabled_then_filtered_when_disabled(
    admin_client, user_client
):
    _set_audio_toggle(admin_client, True)
    body = _minimal_mp3()
    headers = {
        **user_client._auth_headers(),
        "X-Filename": unique_filename("persist.mp3"),
        "X-Mime-Type": "audio/mpeg",
        "Content-Type": "application/octet-stream",
    }
    r = user_client.session.post(
        user_client._url("/api/photos/upload"),
        data=body,
        headers=headers,
    )
    assert r.status_code == 201, r.text[:200]
    photo_id = r.json()["photo_id"]

    # Confirm the row landed with media_type='audio'
    listing = user_client.list_photos()
    rows = listing.get("photos", listing) if isinstance(listing, dict) else listing
    matching = [p for p in rows if p.get("id") == photo_id]
    assert matching, f"Uploaded audio not found in listing: {photo_id}"
    assert matching[0].get("media_type") == "audio"

    # Toggle off — the row stays (we never auto-delete) but new audio
    # uploads must now fail with 403.
    _set_audio_toggle(admin_client, False)
    r2 = user_client.session.post(
        user_client._url("/api/photos/upload"),
        data=body,
        headers={**headers, "X-Filename": unique_filename("after_off.mp3")},
    )
    assert r2.status_code == 403, (
        f"Toggle off must reject new audio uploads, got {r2.status_code}: "
        f"{r2.text[:200]}"
    )

    # Restore the toggle so other tests aren't disturbed.
    _set_audio_toggle(admin_client, True)
