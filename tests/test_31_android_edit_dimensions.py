"""E2E regression tests for Android edit/save dimension & thumbnail issues.

Verifies that:
  1. POST /api/photos/:id/duplicate returns correct width/height/metadata
  2. After 90° rotation, duplicate has swapped dimensions (portrait↔landscape)
  3. Duplicate thumbnail has correct aspect ratio matching rendered dimensions
  4. The duplicate response includes all fields Android needs (width, height,
     duration_secs, mime_type, media_type, size_bytes)
  5. Photo (image) duplicates also get correct rotated dimensions
  6. No-edit duplicates preserve original dimensions exactly
"""

import io
import json
import os
import subprocess
import tempfile
import time

import pytest


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


def generate_landscape_mp4(width: int = 320, height: int = 180,
                           duration: float = 1.0) -> bytes:
    """Generate a short landscape MP4 (wider than tall) with audio."""
    path = tempfile.mktemp(suffix=".mp4")
    try:
        subprocess.run([
            "ffmpeg", "-y",
            "-f", "lavfi", "-i",
            f"color=c=blue:s={width}x{height}:d={duration}",
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


# =====================================================================
# 1. Duplicate response includes width/height/metadata fields
# =====================================================================

class TestDuplicateResponseFields:
    """The duplicate endpoint must return all fields Android needs to
    create a correct local PhotoEntity without a follow-up sync."""

    @needs_ffmpeg
    def test_duplicate_response_has_dimensions(self, user_client):
        """POST /api/photos/:id/duplicate response must include width & height."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("resp_fields.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        assert resp.status_code == 201, f"Duplicate failed: {resp.status_code} {resp.text}"
        data = resp.json()

        # Must include all fields Android needs
        assert "width" in data, f"Response missing 'width': {data}"
        assert "height" in data, f"Response missing 'height': {data}"
        assert "mime_type" in data, f"Response missing 'mime_type': {data}"
        assert "media_type" in data, f"Response missing 'media_type': {data}"
        assert "size_bytes" in data, f"Response missing 'size_bytes': {data}"
        assert "duration_secs" in data, f"Response missing 'duration_secs': {data}"

        # Basic sanity
        assert data["width"] > 0, f"width should be > 0, got {data['width']}"
        assert data["height"] > 0, f"height should be > 0, got {data['height']}"
        assert data["size_bytes"] > 0

    @needs_ffmpeg
    def test_duplicate_response_crop_metadata_null(self, user_client):
        """Duplicate response should always have crop_metadata=null
        (edits are baked into the file)."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("null_crop.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 180, "brightness": 10})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        data = resp.json()
        assert data["crop_metadata"] is None, (
            f"Expected crop_metadata=null, got {data['crop_metadata']}"
        )


# =====================================================================
# 2. Video rotation produces correct dimensions in duplicate response
# =====================================================================

class TestDuplicateRotationDimensions:
    """When duplicating with rotation, the response dimensions must reflect
    the actual rendered file — not the original's dimensions."""

    @needs_ffmpeg
    def test_90_rotation_swaps_dimensions_in_response(self, user_client):
        """320×180 landscape + 90° rotation → response should be ~180×320."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("rot90_resp.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        data = resp.json()
        w, h = data["width"], data["height"]

        # After 90° rotation: width should be < height (portrait)
        assert w < h, (
            f"After 90° rotation of 320×180, expected portrait (w<h), got {w}×{h}"
        )
        # Specific values (ffmpeg may round to even numbers)
        assert w == 180, f"Expected width=180, got {w}"
        assert h == 320, f"Expected height=320, got {h}"

    @needs_ffmpeg
    def test_270_rotation_swaps_dimensions_in_response(self, user_client):
        """320×180 + 270° rotation → same as 90° (portrait output)."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("rot270_resp.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 270})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        data = resp.json()
        w, h = data["width"], data["height"]

        assert w < h, f"Expected portrait after 270°, got {w}×{h}"

    @needs_ffmpeg
    def test_180_rotation_preserves_dimensions(self, user_client):
        """180° rotation should keep the same orientation."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("rot180_resp.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 180})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        data = resp.json()
        w, h = data["width"], data["height"]

        assert w == 320 and h == 180, (
            f"180° should preserve 320×180, got {w}×{h}"
        )

    @needs_ffmpeg
    def test_no_edit_preserves_original_dimensions(self, user_client):
        """Duplicate without edits should keep original dimensions exactly."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("noedit_resp.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={},
        )
        data = resp.json()
        assert data["width"] == 320, f"Expected width=320, got {data['width']}"
        assert data["height"] == 180, f"Expected height=180, got {data['height']}"


# =====================================================================
# 3. Duplicate listing also has correct dimensions
# =====================================================================

class TestDuplicateListingDimensions:
    """After a duplicate with rotation, the photos listing must also
    reflect the correct post-render dimensions (not the original's)."""

    @needs_ffmpeg
    def test_rotated_duplicate_in_listing_has_correct_dims(self, user_client):
        """The GET /api/photos listing should show the copy's rendered dimensions."""
        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("listing_dims.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup_resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        dup_data = dup_resp.json()
        dup_id = dup_data["id"]
        time.sleep(1)

        # Check the listing
        photos = user_client.list_photos()["photos"]
        dup_photo = next((p for p in photos if p["id"] == dup_id), None)
        assert dup_photo is not None, f"Duplicate {dup_id} not found in listing"

        assert dup_photo["width"] == 180, (
            f"Listing width should be 180, got {dup_photo['width']}"
        )
        assert dup_photo["height"] == 320, (
            f"Listing height should be 320, got {dup_photo['height']}"
        )
        assert dup_photo.get("crop_metadata") is None, (
            "Listing crop_metadata should be null for rendered copy"
        )


# =====================================================================
# 4. Thumbnail of rotated duplicate has correct aspect ratio
# =====================================================================

class TestDuplicateThumbnailOrientation:
    """The server-generated thumbnail of a rotated duplicate should have
    the correct aspect ratio (portrait thumbnail for portrait media)."""

    @needs_ffmpeg
    def test_rotated_duplicate_thumbnail_is_portrait(self, user_client):
        """Thumbnail of a 90°-rotated landscape video should be portrait."""
        from PIL import Image

        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("thumb_orient.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup_resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        dup_id = dup_resp.json()["id"]
        time.sleep(3)  # wait for thumbnail generation

        thumb_resp = user_client.get_photo_thumb(dup_id)
        assert thumb_resp.status_code == 200, (
            f"Thumbnail fetch failed: {thumb_resp.status_code}"
        )

        img = Image.open(io.BytesIO(thumb_resp.content))
        assert img.height > img.width, (
            f"Thumbnail should be portrait (h>w), got {img.width}×{img.height}"
        )

    @needs_ffmpeg
    def test_unrotated_duplicate_thumbnail_is_landscape(self, user_client):
        """Thumbnail of an unrotated landscape video should stay landscape."""
        from PIL import Image

        video = generate_landscape_mp4(320, 180)
        upload = user_client.upload_photo("thumb_land.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        # No rotation — just duplicate
        dup_resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={},
        )
        dup_id = dup_resp.json()["id"]
        time.sleep(3)

        thumb_resp = user_client.get_photo_thumb(dup_id)
        assert thumb_resp.status_code == 200

        img = Image.open(io.BytesIO(thumb_resp.content))
        assert img.width > img.height, (
            f"Thumbnail should be landscape (w>h), got {img.width}×{img.height}"
        )


# =====================================================================
# 5. Image (JPEG) duplicate rotation
# =====================================================================

class TestImageDuplicateDimensions:
    """Image duplicates (rendered via image crate) should also have
    correct post-rotation dimensions in the response and listing."""

    def test_image_90_rotation_swaps_dimensions(self, user_client):
        """JPEG 200×150 + 90° rotation → expect 150×200."""
        from helpers import generate_test_jpeg
        jpeg = generate_test_jpeg(200, 150)
        upload = user_client.upload_photo("img_rot.jpg", jpeg, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(1)

        crop = json.dumps({"rotate": 90})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        assert resp.status_code == 201
        data = resp.json()

        assert data["width"] == 150, f"Expected width=150, got {data['width']}"
        assert data["height"] == 200, f"Expected height=200, got {data['height']}"

    def test_image_no_edit_preserves_dimensions(self, user_client):
        """Plain image duplicate preserves original dimensions."""
        from helpers import generate_test_jpeg
        jpeg = generate_test_jpeg(200, 150)
        upload = user_client.upload_photo("img_plain.jpg", jpeg, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(1)

        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={},
        )
        data = resp.json()
        assert data["width"] == 200, f"Expected width=200, got {data['width']}"
        assert data["height"] == 150, f"Expected height=150, got {data['height']}"

    def test_image_rotated_thumbnail_is_portrait(self, user_client):
        """Thumbnail of a 90°-rotated landscape JPEG should be portrait."""
        from PIL import Image
        from helpers import generate_test_jpeg

        jpeg = generate_test_jpeg(200, 150)
        upload = user_client.upload_photo("img_thumb.jpg", jpeg, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(1)

        crop = json.dumps({"rotate": 90})
        dup_resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": crop},
        )
        dup_id = dup_resp.json()["id"]
        time.sleep(2)

        thumb_resp = user_client.get_photo_thumb(dup_id)
        assert thumb_resp.status_code == 200

        img = Image.open(io.BytesIO(thumb_resp.content))
        assert img.height > img.width, (
            f"Image thumbnail should be portrait (h>w) after 90° rotation, "
            f"got {img.width}×{img.height}"
        )
