"""
Test 85: Deferred-conversion import path.

The bulk Import page sets `X-Defer-Conversion` so a folder of convertible
files (HEIC, TIFF, MKV, …) doesn't stall the sequential upload loop on a slow
per-file FFmpeg run — the Windows-vs-Ubuntu divergence we traced, where the
automated autoscan path is two-phase (import all → convert in background) but
the manual Import path used to convert inline and block.

When honored, the server drops the raw original into the storage tree and lets
the SAME background pass the autoscan uses convert + register + encrypt it,
returning 202 immediately.

Covers:
  - Convertible admin upload + defer header → 202 "queued", converts in bg
  - Native upload + defer header → still inline (returns a photo record)
  - Non-admin upload + defer header → inline (defer is admin-only)
  - Convertible upload + metadata override → inline (override can't be replayed)
"""

import io
import random
import time

import pytest
from helpers import (
    APIClient,
    generate_test_tiff,
    generate_test_jpeg,
    unique_filename,
    _ffmpeg_available,
)


def _unique_tiff() -> bytes:
    """A real TIFF with unique pixels (random size + colour). Conversion strips
    metadata, so the *converted* JPEG must differ in pixels — otherwise the
    pass's content-hash dedup would drop it as a duplicate of the deterministic
    TIFFs other test classes upload, and it would never register."""
    from PIL import Image
    w, h = random.randint(24, 240), random.randint(24, 240)
    color = (random.randint(0, 255), random.randint(0, 255), random.randint(0, 255))
    buf = io.BytesIO()
    Image.new("RGB", (w, h), color).save(buf, format="TIFF")
    return buf.getvalue()


pytestmark = pytest.mark.skipif(
    not _ffmpeg_available(),
    reason="ffmpeg not installed — conversion tests require ffmpeg",
)

DEFER = {"X-Defer-Conversion": "1"}


class TestDeferredConversion:
    def test_convertible_admin_defer_queues_then_converts(self, admin_client: APIClient):
        """Convertible + admin + defer → 202 queued, no inline photo_id,
        then the background pass converts it to JPEG."""
        tiff_name = unique_filename("tiff")
        data = admin_client.upload_photo(
            tiff_name, _unique_tiff(), mime_type="image/tiff",
            extra_headers=DEFER,
        )

        # Deferred response: queued, NOT an inline photo record.
        assert data.get("deferred") is True, f"expected deferred response, got {data}"
        assert data.get("status") == "queued"
        assert "photo_id" not in data

        # The background pass should convert + register it as a JPEG. Drive a
        # scan each iteration so a transient conversion miss under concurrent
        # load is retried promptly (in production the periodic autoscan does
        # this); poll for the converted photo by its unique filename stem.
        stem = tiff_name.rsplit(".", 1)[0]
        deadline = time.time() + 120
        converted: list = []
        photos: list = []
        while time.time() < deadline:
            admin_client.admin_trigger_scan()
            admin_client.wait_for_conversion(timeout=30)
            photos = admin_client.list_photos()["photos"]
            converted = [
                p for p in photos
                if p.get("mime_type") == "image/jpeg"
                and p.get("filename", "").startswith(stem)
            ]
            if converted:
                break
            time.sleep(2)
        assert len(converted) >= 1, (
            f"deferred TIFF never converted. stem={stem}, "
            f"photos={[p.get('filename') for p in photos]}"
        )

    def test_native_defer_header_still_inline(self, admin_client: APIClient):
        """Native files are never deferred — they upload fast and inline even
        with the defer header set."""
        data = admin_client.upload_photo(
            unique_filename("jpg"), generate_test_jpeg(), mime_type="image/jpeg",
            extra_headers=DEFER,
        )
        assert "photo_id" in data, f"native upload should be inline, got {data}"
        assert data.get("deferred") is not True

    def test_defer_is_admin_only(self, user_client: APIClient):
        """A non-admin's convertible upload ignores the defer header and
        converts inline (the background pass attributes photos to the admin
        user, so deferring a non-admin upload would misattribute it)."""
        data = user_client.upload_photo(
            unique_filename("tiff"), generate_test_tiff(), mime_type="image/tiff",
            extra_headers=DEFER,
        )
        assert "photo_id" in data, f"non-admin defer should be inline, got {data}"
        assert data.get("deferred") is not True
        assert data["filename"].endswith(".jpg")

    def test_metadata_override_forces_inline(self, admin_client: APIClient):
        """A defer request carrying a sidecar metadata override (X-Taken-At)
        stays on the inline path so the override isn't lost — the background
        pass reads metadata from the file and can't replay sidecar values."""
        headers = {**DEFER, "X-Taken-At": "2019-03-04T12:00:00Z"}
        data = admin_client.upload_photo(
            unique_filename("tiff"), generate_test_tiff(), mime_type="image/tiff",
            extra_headers=headers,
        )
        assert "photo_id" in data, f"override upload should be inline, got {data}"
        assert data.get("deferred") is not True
        assert data["filename"].endswith(".jpg")
