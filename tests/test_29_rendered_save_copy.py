"""E2E tests for the rendered Save Copy overhaul.

The ``duplicate_photo`` endpoint now uses ffmpeg (video/audio) or the image
crate (photos) to bake edits into a fully independent file.  The copy has
its own ``file_path``, ``thumb_path``, correct ``width``/``height``, and
**no** ``crop_metadata`` (edits are burned in).

Tests verify:
  1. Rotated video copy has correct portrait dimensions
  2. Thumbnail of rotated copy is portrait
  3. Rotated copy's served file is a real re-encoded video (different bytes)
  4. Date/timeline ordering is preserved for copies
  5. Trimmed video copy has shorter duration
  6. Copy has NULL crop_metadata (edits baked in)
  7. Original is untouched after copy
  8. Photo (image) with rotation produces correct dimensions
  9. 180° rotation keeps same aspect ratio
 10. No-edit copy is a plain file copy (identity)
 11. Save (non-copy) syncs crop_metadata to server
"""

import json
import io
import os
import subprocess
import tempfile
import time

import pytest

# ── Helper: generate MP4 test videos ─────────────────────────────────────

def _ffmpeg_available() -> bool:
    try:
        subprocess.run(["ffmpeg", "-version"], capture_output=True, timeout=5)
        return True
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def generate_landscape_mp4(width: int = 320, height: int = 180,
                           duration: float = 1.0) -> bytes:
    """Generate a short landscape MP4 (wider than tall) with audio track."""
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


def generate_portrait_mp4(width: int = 180, height: int = 320,
                          duration: float = 1.0) -> bytes:
    """Generate a short portrait MP4 (taller than wide)."""
    path = tempfile.mktemp(suffix=".mp4")
    try:
        subprocess.run([
            "ffmpeg", "-y",
            "-f", "lavfi", "-i",
            f"color=c=green:s={width}x{height}:d={duration}",
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


needs_ffmpeg = pytest.mark.skipif(
    not _ffmpeg_available(), reason="ffmpeg not installed"
)


# =====================================================================
# 1. Video rotation produces a real rendered copy
# =====================================================================

class TestRenderedVideoCopy:
    """When duplicating a video with rotation, the server should ffmpeg-render
    a new file — not just store crop_metadata on a shared blob."""

    @needs_ffmpeg
    def test_rotated_copy_has_portrait_dimensions(self, user_client):
        """A 320×180 landscape video rotated 90° → copy should be 180×320."""
        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("landscape.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)  # ffmpeg render takes a moment

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)

        assert dup_photo["width"] == 180, (
            f"Expected width=180 after 90° rotation, got {dup_photo['width']}"
        )
        assert dup_photo["height"] == 320, (
            f"Expected height=320 after 90° rotation, got {dup_photo['height']}"
        )

    @needs_ffmpeg
    def test_rotated_copy_thumbnail_is_portrait(self, user_client):
        """The thumbnail of a 90° rotated copy should be portrait-shaped."""
        from PIL import Image

        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("thumb_test.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        thumb_resp = user_client.get_photo_thumb(dup_id)
        assert thumb_resp.status_code == 200
        img = Image.open(io.BytesIO(thumb_resp.content))
        assert img.height > img.width, (
            f"Thumbnail should be portrait (h>w), got {img.width}×{img.height}"
        )

    @needs_ffmpeg
    def test_rotated_copy_file_differs_from_original(self, user_client):
        """The copy's served file should be different bytes than the original
        (proves ffmpeg actually re-encoded, not just shared the blob)."""
        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("differ_test.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        orig_file = user_client.get_photo_file(photo_id)
        dup_file = user_client.get_photo_file(dup_id)

        assert orig_file.status_code == 200
        assert dup_file.status_code == 200
        assert orig_file.content != dup_file.content, (
            "Copy file should differ from original (ffmpeg re-encoded)"
        )

    @needs_ffmpeg
    def test_copy_has_null_crop_metadata(self, user_client):
        """The rendered copy should have crop_metadata=NULL since edits are
        baked into the file."""
        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("nullcrop.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)
        assert dup_photo.get("crop_metadata") is None, (
            f"Rendered copy should have NULL crop_metadata, got {dup_photo.get('crop_metadata')}"
        )

    @needs_ffmpeg
    def test_original_untouched_after_copy(self, user_client):
        """Original photo should be unchanged after creating a rotated copy."""
        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("untouched.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Remember original state
        photos_before = user_client.list_photos()["photos"]
        orig_before = next(p for p in photos_before if p["id"] == photo_id)

        crop = json.dumps({"rotate": 90})
        user_client.duplicate_photo(photo_id, crop_metadata=crop)
        time.sleep(3)

        photos_after = user_client.list_photos()["photos"]
        orig_after = next(p for p in photos_after if p["id"] == photo_id)

        assert orig_after["width"] == orig_before["width"]
        assert orig_after["height"] == orig_before["height"]
        assert orig_after.get("crop_metadata") == orig_before.get("crop_metadata")

    @needs_ffmpeg
    def test_270_rotation_also_portrait(self, user_client):
        """270° rotation should also produce portrait dimensions."""
        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("rot270.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 270})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)
        assert dup_photo["width"] == 180
        assert dup_photo["height"] == 320

    @needs_ffmpeg
    def test_180_rotation_stays_landscape(self, user_client):
        """180° rotation does NOT swap dimensions."""
        video = generate_landscape_mp4(320, 180, 1.0)
        upload = user_client.upload_photo("rot180.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 180})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)
        assert dup_photo["width"] == 320
        assert dup_photo["height"] == 180


# =====================================================================
# 2. Trimmed video copy has shorter duration
# =====================================================================

class TestTrimmedCopy:

    @needs_ffmpeg
    def test_trimmed_copy_has_shorter_duration(self, user_client):
        """A 2-second video trimmed to 0–1s should produce a ~1s copy.
        Note: the upload endpoint doesn't store duration_secs, but the
        duplicate endpoint probes it from the rendered file."""
        video = generate_landscape_mp4(320, 180, 2.0)
        upload = user_client.upload_photo("trim_test.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        # Trim first second only
        crop = json.dumps({"trimStart": 0, "trimEnd": 1.0})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)

        # The rendered copy should have duration_secs probed from the file
        assert dup_photo["duration_secs"] is not None, (
            "Rendered trimmed copy should have duration_secs populated"
        )
        assert dup_photo["duration_secs"] < 1.5, (
            f"Trimmed copy should be ~1s, got {dup_photo['duration_secs']}s"
        )


# =====================================================================
# 3. Date/ordering preservation
# =====================================================================

class TestCopyDatePreservation:

    @needs_ffmpeg
    def test_copy_preserves_timeline_position(self, user_client):
        """Copy of the oldest video should stay adjacent in the timeline,
        not jump to the top."""
        v1 = generate_landscape_mp4(64, 64, 0.3)
        v2 = generate_landscape_mp4(64, 64, 0.3)
        v3 = generate_landscape_mp4(64, 64, 0.3)

        up1 = user_client.upload_photo("first.mp4", v1, "video/mp4")
        time.sleep(0.5)
        up2 = user_client.upload_photo("middle.mp4", v2, "video/mp4")
        time.sleep(0.5)
        up3 = user_client.upload_photo("last.mp4", v3, "video/mp4")
        time.sleep(1)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(up1["photo_id"], crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        ids = [p["id"] for p in photos]
        dup_idx = ids.index(dup_id)
        orig_idx = ids.index(up1["photo_id"])

        assert abs(dup_idx - orig_idx) <= 1, (
            f"Copy should be adjacent to original: copy@{dup_idx}, orig@{orig_idx}"
        )

    @needs_ffmpeg
    def test_copy_created_at_matches_original(self, user_client):
        """Copy's created_at should match the original's."""
        video = generate_landscape_mp4(64, 64, 0.3)
        upload = user_client.upload_photo("date_test.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(1)

        photos_before = user_client.list_photos()["photos"]
        original = next(p for p in photos_before if p["id"] == photo_id)
        original_created = original["created_at"]

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(3)

        photos_after = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos_after if p["id"] == dup_id)
        assert dup_photo["created_at"] == original_created


# =====================================================================
# 4. Image rotation (photo, not video)
# =====================================================================

class TestRenderedImageCopy:

    def test_rotated_image_copy_has_portrait_dimensions(self, user_client):
        """A 200×100 landscape JPEG rotated 90° → copy should be 100×200."""
        from PIL import Image

        # Generate a 200×100 JPEG
        img = Image.new("RGB", (200, 100), color=(255, 0, 0))
        buf = io.BytesIO()
        img.save(buf, "JPEG")
        jpeg_bytes = buf.getvalue()

        upload = user_client.upload_photo("landscape.jpg", jpeg_bytes, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)
        assert dup_photo["width"] == 100, f"Expected 100, got {dup_photo['width']}"
        assert dup_photo["height"] == 200, f"Expected 200, got {dup_photo['height']}"

    def test_rotated_image_copy_file_is_portrait(self, user_client):
        """The actual served file should be portrait after rotation."""
        from PIL import Image

        img = Image.new("RGB", (200, 100), color=(0, 255, 0))
        buf = io.BytesIO()
        img.save(buf, "JPEG")
        jpeg_bytes = buf.getvalue()

        upload = user_client.upload_photo("portrait_check.jpg", jpeg_bytes, "image/jpeg")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=crop)
        dup_id = dup["id"]
        time.sleep(2)

        file_resp = user_client.get_photo_file(dup_id)
        assert file_resp.status_code == 200
        result_img = Image.open(io.BytesIO(file_resp.content))
        assert result_img.height > result_img.width, (
            f"Served file should be portrait, got {result_img.width}×{result_img.height}"
        )


# =====================================================================
# 5. No-edit copy (identity)
# =====================================================================

class TestIdentityCopy:

    @needs_ffmpeg
    def test_no_edit_copy_preserves_dimensions(self, user_client):
        """Copy with no crop_metadata should have same dimensions."""
        video = generate_landscape_mp4(320, 180, 0.5)
        upload = user_client.upload_photo("identity.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        dup = user_client.duplicate_photo(photo_id)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        dup_photo = next(p for p in photos if p["id"] == dup_id)
        assert dup_photo["width"] == 320
        assert dup_photo["height"] == 180


# =====================================================================
# 6. Save (non-copy) syncs crop_metadata to server
# =====================================================================

class TestSaveSyncsToServer:

    @needs_ffmpeg
    def test_crop_metadata_persists_via_set_crop(self, user_client):
        """PUT /api/photos/:id/crop should persist crop_metadata that
        is returned in subsequent list_photos."""
        video = generate_landscape_mp4(320, 180, 0.5)
        upload = user_client.upload_photo("crop_sync.mp4", video, "video/mp4")
        photo_id = upload["photo_id"]
        time.sleep(2)

        crop = json.dumps({"rotate": 90, "brightness": 25})
        user_client.crop_photo(photo_id, crop)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        assert photo.get("crop_metadata") is not None
        meta = photo["crop_metadata"]
        if isinstance(meta, str):
            meta = json.loads(meta)
        assert meta.get("rotate") == 90
        assert meta.get("brightness") == 25
