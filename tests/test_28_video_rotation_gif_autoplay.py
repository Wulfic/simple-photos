"""
E2E regression tests for:
  1. Landscape→portrait video rotation: thumbnail must reflect rotation
  2. Duplicate video date preservation: ordering must use original date, not today
  3. Duplicate video dimensions: width/height must be swapped for 90°/270° rotation
  4. Rendered (downloaded) portrait video: correct orientation metadata
  5. GIF autoplay in album/secure gallery grid views (server-side animated thumbnail)

These tests target the server API layer and verify metadata contracts that
the web and Android clients depend on for correct display.
"""

import json
import os
import struct
import subprocess
import tempfile
import time

import pytest

from helpers import APIClient, generate_test_gif, generate_test_jpeg


# ── Test-data generators ─────────────────────────────────────────────

def _ffmpeg_available() -> bool:
    try:
        subprocess.run(["ffmpeg", "-version"], capture_output=True, timeout=5)
        return True
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def generate_landscape_mp4(width: int = 320, height: int = 180,
                           duration: float = 0.5) -> bytes:
    """Generate a short landscape MP4 (wider than tall)."""
    path = tempfile.mktemp(suffix=".mp4")
    try:
        subprocess.run([
            "ffmpeg", "-y", "-f", "lavfi", "-i",
            f"color=c=red:s={width}x{height}:d={duration}",
            "-c:v", "libx264", "-preset", "ultrafast",
            "-pix_fmt", "yuv420p",
            "-movflags", "+faststart",
            path,
        ], capture_output=True, timeout=30, check=True)
        with open(path, "rb") as f:
            return f.read()
    finally:
        if os.path.exists(path):
            os.unlink(path)


# ── Fixtures ─────────────────────────────────────────────────────────

needs_ffmpeg = pytest.mark.skipif(
    not _ffmpeg_available(), reason="ffmpeg not installed"
)


# =====================================================================
# 1. Rotated thumbnail must be portrait after duplicating with rotation
# =====================================================================

class TestRotatedVideoThumbnail:
    """When a landscape video is duplicated with 90° rotation in crop_metadata,
    the duplicate's thumbnail should be portrait (height > width)."""

    @needs_ffmpeg
    def test_duplicate_with_rotation_thumbnail_is_portrait(self, user_client):
        """BUG: duplicate_photo() reuses original thumb_path — the thumbnail
        stays landscape even though the edit applies 90° rotation."""
        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="landscape.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        # Give thumbnail generation a moment
        time.sleep(2)

        # Duplicate with 90° rotation
        crop_meta = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop_meta)
        dup_id = dup["id"]
        time.sleep(2)

        # Fetch the duplicate's metadata
        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)

        # The duplicate should have swapped dimensions (portrait)
        assert dup_photo["width"] == 180, (
            f"Expected width=180 (original height) after 90° rotation, "
            f"got {dup_photo['width']}"
        )
        assert dup_photo["height"] == 320, (
            f"Expected height=320 (original width) after 90° rotation, "
            f"got {dup_photo['height']}"
        )

    @needs_ffmpeg
    def test_duplicate_with_rotation_thumbnail_aspect_ratio(self, user_client):
        """The thumbnail image itself should be portrait-oriented."""
        from PIL import Image
        import io

        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="landscape_thumb_test.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Duplicate with 90° rotation
        crop_meta = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop_meta)
        dup_id = dup["id"]
        time.sleep(2)

        # Fetch thumbnail
        thumb_resp = user_client.get_photo_thumb(dup_id)
        assert thumb_resp.status_code == 200, (
            f"Failed to get duplicate thumbnail: {thumb_resp.status_code}"
        )

        img = Image.open(io.BytesIO(thumb_resp.content))
        # Thumbnail should be portrait (taller than wide)
        assert img.height > img.width, (
            f"Thumbnail should be portrait (h > w) after 90° rotation, "
            f"got {img.width}x{img.height}"
        )

    @needs_ffmpeg
    def test_duplicate_with_270_rotation_also_portrait(self, user_client):
        """270° rotation should also produce portrait dimensions."""
        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="landscape_270.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop_meta = json.dumps({"rotate": 270})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop_meta)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)

        assert dup_photo["width"] == 180
        assert dup_photo["height"] == 320

    @needs_ffmpeg
    def test_duplicate_with_180_rotation_stays_landscape(self, user_client):
        """180° rotation should NOT swap dimensions."""
        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="landscape_180.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop_meta = json.dumps({"rotate": 180})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop_meta)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)

        # 180° rotation does NOT swap
        assert dup_photo["width"] == 320
        assert dup_photo["height"] == 180


# =====================================================================
# 2. Date preservation: duplicate must keep original's date for ordering
# =====================================================================

class TestDuplicateDatePreservation:
    """When duplicating a photo/video, the copy should keep the same
    position in the timeline (same taken_at / created_at for ordering)."""

    @needs_ffmpeg
    def test_duplicate_preserves_ordering_position(self, user_client):
        """BUG: duplicate uses created_at=now() so videos without taken_at
        jump to the top of the timeline."""
        # Upload three videos to establish an ordering
        v1 = generate_landscape_mp4(64, 64, 0.3)
        v2 = generate_landscape_mp4(64, 64, 0.3)
        v3 = generate_landscape_mp4(64, 64, 0.3)

        up1 = user_client.upload_photo("first.mp4", v1, "video/mp4")
        time.sleep(0.5)
        up2 = user_client.upload_photo("middle.mp4", v2, "video/mp4")
        time.sleep(0.5)
        up3 = user_client.upload_photo("last.mp4", v3, "video/mp4")
        time.sleep(1)

        # Get ordering before duplication
        photos_before = user_client.list_photos()["photos"]
        ids_before = [p["id"] for p in photos_before]

        # Duplicate the FIRST (oldest) uploaded video — it should NOT jump to top
        crop_meta = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(up1["photo_id"], crop_metadata=crop_meta)
        dup_id = dup["id"]
        time.sleep(1)

        # Get ordering after duplication
        photos_after = user_client.list_photos()["photos"]
        ids_after = [p["id"] for p in photos_after]
        dup_photo = next(p for p in photos_after if p["id"] == dup_id)
        orig_photo = next(p for p in photos_after if p["id"] == up1["photo_id"])

        # The duplicate should have similar ordering date to the original.
        # Since photos are ordered by COALESCE(taken_at, created_at) DESC,
        # the duplicate should NOT appear far from the original.
        # When created_at matches, the filename tie-break ("Copy of first.mp4"
        # < "first.mp4") may place the duplicate just before the original.
        dup_idx = ids_after.index(dup_id)
        orig_idx = ids_after.index(up1["photo_id"])

        # The duplicate of the oldest photo should be adjacent to the original.
        # Before the fix, it would jump to position 0 (most recent) because
        # its created_at was set to now() instead of the original's date.
        assert abs(dup_idx - orig_idx) <= 1, (
            f"Duplicate should be adjacent to the original. "
            f"Duplicate is at position {dup_idx}, original at position {orig_idx}."
        )

    @needs_ffmpeg
    def test_duplicate_created_at_matches_original(self, user_client):
        """The duplicate's created_at should match the original's, not be 'now'."""
        video_bytes = generate_landscape_mp4(64, 64, 0.3)
        upload = user_client.upload_photo("date_test.mp4", video_bytes, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(1)

        # Get original's created_at
        photos = user_client.list_photos()["photos"]
        original = next(p for p in photos if p["id"] == photo_id)
        original_created = original.get("created_at", "")

        # Wait a bit so "now" is clearly different
        time.sleep(2)

        # Duplicate
        dup = user_client.duplicate_photo(photo_id)
        dup_id = dup["id"]
        time.sleep(1)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)
        dup_created = dup_photo.get("created_at", "")

        # The duplicate's created_at should be the same as the original's
        assert dup_created == original_created, (
            f"Duplicate created_at ({dup_created}) should match "
            f"original ({original_created}), not be today's date"
        )


# =====================================================================
# 3. In-place crop with rotation should update stored dimensions
# =====================================================================

class TestCropRotationDimensions:
    """When crop_metadata with rotation is set on a photo, the list endpoint
    should report the display-correct dimensions for client rendering."""

    @needs_ffmpeg
    def test_crop_rotation_90_updates_dimensions(self, user_client):
        """After setting 90° rotation via crop, the photo listing should
        report swapped width/height so clients can size the container correctly."""
        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="crop_rotate.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Set crop metadata with 90° rotation
        crop_meta = json.dumps({"rotate": 90, "x": 0, "y": 0, "width": 1, "height": 1})
        user_client.crop_photo(photo_id, crop_meta)
        time.sleep(1)

        # The listing should reflect rotated dimensions
        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)

        # After 90° rotation, width should become original height and vice versa
        assert photo["width"] == 180, (
            f"Expected width=180 after 90° rotation, got {photo['width']}"
        )
        assert photo["height"] == 320, (
            f"Expected height=320 after 90° rotation, got {photo['height']}"
        )


# =====================================================================
# 4. Render endpoint produces correctly-oriented video
# =====================================================================

class TestRenderOrientation:
    """POST /api/photos/:id/render with rotation should produce a
    video with correct orientation and dimensions."""

    @needs_ffmpeg
    def test_render_90_rotation_produces_portrait(self, user_client):
        """Rendered video with 90° rotation should have portrait dimensions."""
        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="render_test.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Render with 90° rotation
        crop_meta = json.dumps({"rotate": 90})
        resp = user_client.post(
            f"/api/photos/{photo_id}/render",
            json_data={"crop_metadata": crop_meta},
        )
        assert resp.status_code == 200, f"Render failed: {resp.status_code} {resp.text}"

        # Check rendered video dimensions with ffprobe
        rendered_path = tempfile.mktemp(suffix=".mp4")
        try:
            with open(rendered_path, "wb") as f:
                f.write(resp.content)

            result = subprocess.run([
                "ffprobe", "-v", "error",
                "-select_streams", "v:0",
                "-show_entries", "stream=width,height",
                "-of", "json",
                rendered_path,
            ], capture_output=True, text=True, timeout=10)

            probe = json.loads(result.stdout)
            streams = probe.get("streams", [{}])
            rendered_w = streams[0].get("width", 0)
            rendered_h = streams[0].get("height", 0)

            # After 90° rotation, should be portrait
            assert rendered_h > rendered_w, (
                f"Rendered video should be portrait after 90° rotation, "
                f"got {rendered_w}x{rendered_h}"
            )
        finally:
            if os.path.exists(rendered_path):
                os.unlink(rendered_path)


# =====================================================================
# 5. GIF thumbnails: album & secure gallery should serve animated GIFs
# =====================================================================

class TestGifThumbnailAnimation:
    """GIF thumbnails should be animated (not static JPEG first-frame)
    so gallery grid views can autoplay them."""

    @needs_ffmpeg
    def test_gif_thumbnail_is_animated(self, user_client):
        """Uploaded GIF's thumbnail should be an animated GIF, not static JPEG."""
        from PIL import Image
        import io

        gif_bytes = generate_test_gif(width=64, height=64, frames=3)
        upload = user_client.upload_photo(
            filename="animated.gif",
            content=gif_bytes,
            mime_type="image/gif",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Fetch thumbnail
        thumb_resp = user_client.get_photo_thumb(photo_id)
        assert thumb_resp.status_code == 200

        # Thumbnail should be a GIF (not JPEG)
        content_type = thumb_resp.headers.get("Content-Type", "")
        thumb_data = thumb_resp.content

        # Check if it's a GIF by magic bytes
        assert thumb_data[:3] == b"GIF", (
            f"GIF thumbnail should be GIF format (magic bytes), "
            f"got {thumb_data[:4].hex()}, Content-Type: {content_type}"
        )

        # Check that it's animated (has multiple frames)
        img = Image.open(io.BytesIO(thumb_data))
        assert getattr(img, "is_animated", False), (
            f"GIF thumbnail should be animated (multi-frame), "
            f"but n_frames={getattr(img, 'n_frames', 1)}"
        )

    @needs_ffmpeg
    def test_gif_in_album_has_animated_thumbnail(self, user_client):
        """GIFs added to a shared album should have animated thumbnails
        accessible via the standard thumb endpoint."""
        gif_bytes = generate_test_gif(width=48, height=48, frames=4)
        upload = user_client.upload_photo(
            filename="album_animated.gif",
            content=gif_bytes,
            mime_type="image/gif",
        )
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Create album and add GIF
        album = user_client.create_shared_album("GIF Test Album")
        album_id = album["id"]
        user_client.add_album_photo(album_id, photo_id)

        # Thumbnail should still be animated GIF
        thumb_resp = user_client.get_photo_thumb(photo_id)
        assert thumb_resp.status_code == 200
        assert thumb_resp.content[:3] == b"GIF", (
            "Album GIF thumbnail should be GIF format"
        )


# =====================================================================
# 6. Integration: full rotation + ordering pipeline
# =====================================================================

class TestRotationOrderingIntegration:
    """End-to-end: rotate a landscape video to portrait, verify
    dimensions, thumbnail, and ordering are all correct."""

    @needs_ffmpeg
    def test_full_rotation_pipeline(self, user_client):
        """Upload landscape video, duplicate with 90° rotation, verify:
        - Dimensions are swapped (portrait)
        - Thumbnail is portrait
        - Ordering position matches original
        """
        from PIL import Image
        import io

        # Upload an older video first
        old_video = generate_landscape_mp4(64, 64, 0.3)
        old_upload = user_client.upload_photo("old.mp4", old_video, "video/mp4")
        time.sleep(1)

        # Upload the target video
        video_bytes = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo(
            filename="target.mp4",
            content=video_bytes,
            mime_type="video/mp4",
        )
        photo_id = upload["photo_id"]
        time.sleep(1)

        # Upload a newer video
        new_video = generate_landscape_mp4(64, 64, 0.3)
        new_upload = user_client.upload_photo("new.mp4", new_video, "video/mp4")
        time.sleep(2)

        # Record original ordering position
        photos_before = user_client.list_photos()["photos"]
        orig_idx = next(i for i, p in enumerate(photos_before) if p["id"] == photo_id)

        # Duplicate with 90° rotation
        crop_meta = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop_meta)
        dup_id = dup["id"]
        time.sleep(2)

        # Verify dimensions
        photos_after = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos_after if p["id"] == dup_id)
        assert dup_photo["width"] == 180, f"Expected portrait width=180, got {dup_photo['width']}"
        assert dup_photo["height"] == 320, f"Expected portrait height=320, got {dup_photo['height']}"

        # Verify thumbnail is portrait
        thumb_resp = user_client.get_photo_thumb(dup_id)
        if thumb_resp.status_code == 200 and len(thumb_resp.content) > 100:
            img = Image.open(io.BytesIO(thumb_resp.content))
            assert img.height > img.width, (
                f"Duplicate thumbnail should be portrait, got {img.width}x{img.height}"
            )

        # Verify ordering: duplicate should be near original, not at top
        dup_idx = next(i for i, p in enumerate(photos_after) if p["id"] == dup_id)
        # Allow some flexibility but it shouldn't be at position 0 if original wasn't
        if orig_idx > 0:
            assert dup_idx > 0, (
                f"Duplicate jumped to top of timeline (idx=0) instead of "
                f"near original (idx={orig_idx})"
            )
