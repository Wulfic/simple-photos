"""E2E regression tests: render & duplicate endpoints must not hang.

These tests verify that the server-side ffmpeg/ffprobe calls have proper
timeouts and stdin/stdout handling so they never block indefinitely.

Each request uses an HTTP-level timeout to fail fast if the server hangs
(the old bug), while allowing enough time for legitimate rendering.
"""

import json
import os
import subprocess
import tempfile
import time

import pytest
import requests


# ── Helper: generate test media ──────────────────────────────────────

def _ffmpeg_available() -> bool:
    try:
        subprocess.run(["ffmpeg", "-version"], capture_output=True, timeout=5)
        return True
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


needs_ffmpeg = pytest.mark.skipif(
    not _ffmpeg_available(), reason="ffmpeg not installed"
)


def generate_mp4(width: int = 160, height: int = 90,
                 duration: float = 0.5) -> bytes:
    """Generate a minimal MP4 for testing."""
    path = tempfile.mktemp(suffix=".mp4")
    try:
        subprocess.run([
            "ffmpeg", "-y",
            "-f", "lavfi", "-i",
            f"color=c=red:s={width}x{height}:d={duration}",
            "-f", "lavfi", "-i",
            f"sine=frequency=440:duration={duration}",
            "-c:v", "libx264", "-preset", "ultrafast",
            "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "64k",
            "-movflags", "+faststart",
            path,
        ], capture_output=True, timeout=30, check=True)
        with open(path, "rb") as f:
            return f.read()
    finally:
        if os.path.exists(path):
            os.unlink(path)


# Max seconds we allow the server to spend before we declare it hung.
# Normal renders finish in <10s for tiny test clips; 60s is generous.
RENDER_HTTP_TIMEOUT = 60
SERVE_HTTP_TIMEOUT = 30


# =====================================================================
# 1. Render endpoint must respond within timeout
# =====================================================================

class TestRenderEndpointTimeout:
    """POST /api/photos/:id/render should never hang."""

    @needs_ffmpeg
    def test_render_video_responds_in_time(self, user_client):
        """Render a rotated video — must complete within RENDER_HTTP_TIMEOUT."""
        video = generate_mp4(160, 90, 0.5)
        upload = user_client.upload_photo("timeout_render.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)  # let ingest finish

        crop = json.dumps({"rotate": 90})
        try:
            resp = user_client.post(
                f"/api/photos/{photo_id}/render",
                json_data={"crop_metadata": crop},
                timeout=RENDER_HTTP_TIMEOUT,
            )
        except requests.exceptions.Timeout:
            pytest.fail(
                f"Render endpoint hung for >{RENDER_HTTP_TIMEOUT}s — "
                "ffmpeg process likely blocked (missing stdin null / no timeout)"
            )

        assert resp.status_code == 200, (
            f"Render returned {resp.status_code}: {resp.text[:200]}"
        )
        assert len(resp.content) > 0, "Render returned empty body"

    @needs_ffmpeg
    def test_render_no_edit_responds_in_time(self, user_client):
        """Render with no edits (identity) must also finish promptly."""
        video = generate_mp4(160, 90, 0.5)
        upload = user_client.upload_photo("timeout_noedit.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({})
        try:
            resp = user_client.post(
                f"/api/photos/{photo_id}/render",
                json_data={"crop_metadata": crop},
                timeout=RENDER_HTTP_TIMEOUT,
            )
        except requests.exceptions.Timeout:
            pytest.fail(
                f"Render (no-edit) hung for >{RENDER_HTTP_TIMEOUT}s"
            )

        assert resp.status_code == 200


# =====================================================================
# 2. Duplicate endpoint must respond within timeout
# =====================================================================

class TestDuplicateEndpointTimeout:
    """POST /api/photos/:id/duplicate should never hang."""

    @needs_ffmpeg
    def test_duplicate_video_responds_in_time(self, user_client):
        """Duplicate (save copy) with rotation must finish promptly."""
        video = generate_mp4(160, 90, 0.5)
        upload = user_client.upload_photo("timeout_dup.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 180})
        try:
            dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        except requests.exceptions.Timeout:
            pytest.fail(
                f"Duplicate endpoint hung — ffmpeg process likely blocked"
            )

        assert "id" in dup, f"Duplicate response missing id: {dup}"

    @needs_ffmpeg
    def test_duplicate_copy_file_serves_in_time(self, user_client):
        """After duplication, the copy's file should serve without hanging."""
        video = generate_mp4(160, 90, 0.5)
        upload = user_client.upload_photo("timeout_serve.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)  # wait for background render

        try:
            file_resp = user_client.get_photo_file(dup_id)
        except requests.exceptions.Timeout:
            pytest.fail(
                f"Serving duplicate file hung for >{SERVE_HTTP_TIMEOUT}s"
            )

        assert file_resp.status_code == 200, (
            f"File serve returned {file_resp.status_code}"
        )
        assert len(file_resp.content) > 100, "Served file suspiciously small"


# =====================================================================
# 3. Thumbnail generation must not hang
# =====================================================================

class TestThumbnailTimeout:
    """Thumbnail generation (ffmpeg frame-grab) must not hang."""

    @needs_ffmpeg
    def test_video_thumbnail_serves_in_time(self, user_client):
        """After upload, requesting the thumbnail should not hang."""
        video = generate_mp4(160, 90, 0.5)
        upload = user_client.upload_photo("timeout_thumb.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(3)  # wait for thumbnail generation

        try:
            resp = user_client.get_photo_thumb(photo_id)
        except requests.exceptions.Timeout:
            pytest.fail(
                "Thumbnail serve hung — ffmpeg thumbnail generation may have blocked"
            )

        assert resp.status_code == 200
        assert len(resp.content) > 100, "Thumbnail suspiciously small"

    @needs_ffmpeg
    def test_duplicate_thumbnail_serves_in_time(self, user_client):
        """Thumbnail of a rendered copy should also serve without hanging."""
        video = generate_mp4(160, 90, 0.5)
        upload = user_client.upload_photo("timeout_dupthumb.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(4)  # wait for render + thumbnail

        try:
            resp = user_client.get_photo_thumb(dup_id)
        except requests.exceptions.Timeout:
            pytest.fail("Duplicate thumbnail serve hung")

        assert resp.status_code == 200


# =====================================================================
# 4. Image render must not hang
# =====================================================================

class TestImageRenderTimeout:
    """Image (non-video) render and duplicate should also finish promptly."""

    def test_render_image_rejects_fast(self, user_client):
        """Render endpoint rejects images with 400 (images are client-side),
        and critically, does NOT hang."""
        from helpers import generate_test_jpeg
        jpeg = generate_test_jpeg(200, 150)
        upload = user_client.upload_photo("timeout_img.jpg", jpeg, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(1)

        crop = json.dumps({"rotate": 90})
        try:
            resp = user_client.post(
                f"/api/photos/{photo_id}/render",
                json_data={"crop_metadata": crop},
                timeout=RENDER_HTTP_TIMEOUT,
            )
        except requests.exceptions.Timeout:
            pytest.fail("Image render hung (should reject fast with 400)")

        assert resp.status_code == 400, (
            f"Expected 400 for image render, got {resp.status_code}"
        )

    def test_duplicate_image_responds_in_time(self, user_client):
        """Duplicate a JPEG with crop — must not hang."""
        from helpers import generate_test_jpeg
        jpeg = generate_test_jpeg(200, 150)
        upload = user_client.upload_photo("timeout_imgdup.jpg", jpeg, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(1)

        crop = json.dumps({"rotate": 180})
        try:
            dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        except requests.exceptions.Timeout:
            pytest.fail("Image duplicate hung")

        assert "id" in dup
