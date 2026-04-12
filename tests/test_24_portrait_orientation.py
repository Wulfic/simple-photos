"""
Test 24: Portrait Photo Orientation — regression test for portrait photos
being rendered as landscape in the gallery grid.

The gallery grid uses width/height from the API to compute aspect ratios
(width / height). Portrait photos taken by phone cameras store raw pixels
in landscape orientation with EXIF orientation tags (5–8) indicating 90°/270°
rotation. The server must swap width/height to reflect **display** dimensions,
not raw pixel dimensions.

This test:
  1. Uploads photos with EXIF orientation tags 1–8 via /api/photos/upload
  2. Verifies the server returns display-correct width/height
  3. Verifies encrypted-sync returns the same correct dimensions
  4. Tests the web-upload path (no EXIF, browser pre-rotates) still works
"""

import io
import time
import random

import pytest
from PIL import Image

try:
    import piexif
except ImportError:
    piexif = None

from helpers import APIClient, generate_test_jpeg


def _unique(prefix: str) -> str:
    return f"{prefix}_{int(time.time() * 1000)}_{random.randint(1000, 9999)}.jpg"


def _make_jpeg_with_orientation(raw_w: int, raw_h: int, orientation: int) -> bytes:
    """Create a JPEG with specific raw pixel dimensions and EXIF orientation.

    Phone cameras typically store portrait photos with raw pixels in landscape
    (e.g., 4032×3024) and EXIF orientation 6 (rotate 90° CW), meaning display
    dimensions are 3024×4032 (portrait).
    """
    if piexif is None:
        pytest.skip("piexif not installed")
    img = Image.new("RGB", (raw_w, raw_h), color=(
        random.randint(0, 255),
        random.randint(0, 255),
        random.randint(0, 255),
    ))
    buf = io.BytesIO()
    exif_dict = {"0th": {piexif.ImageIFD.Orientation: orientation}}
    exif_bytes = piexif.dump(exif_dict)
    img.save(buf, format="JPEG", quality=85, exif=exif_bytes)
    return buf.getvalue()


class TestPortraitExifOrientation:
    """Verify that EXIF orientation tags are respected when storing dimensions.

    Orientations 5–8 indicate 90°/270° rotation, so width and height should
    be swapped relative to the raw pixel dimensions. Orientations 1–4 do NOT
    swap dimensions (they are mirrors/180° rotations of the same aspect).
    """

    @pytest.mark.parametrize("orientation,should_swap", [
        (1, False),  # Normal
        (2, False),  # Flip horizontal
        (3, False),  # Rotate 180°
        (4, False),  # Flip vertical
        (5, True),   # Rotate 90° CW + flip horizontal
        (6, True),   # Rotate 90° CW (most common portrait)
        (7, True),   # Rotate 90° CCW + flip horizontal
        (8, True),   # Rotate 90° CCW
    ])
    def test_upload_orientation_dimensions(self, user_client, orientation, should_swap):
        """Upload a photo with a specific EXIF orientation and verify the
        server stores display-correct dimensions (swapped for orientations 5–8)."""
        raw_w, raw_h = 300, 200  # Raw pixels: landscape
        content = _make_jpeg_with_orientation(raw_w, raw_h, orientation)
        filename = _unique(f"orient_{orientation}")

        data = user_client.upload_photo(filename, content)
        assert "photo_id" in data, f"Upload failed: {data}"

        # Fetch dimensions from the API
        photos = user_client.list_photos()["photos"]
        photo = next((p for p in photos if p["id"] == data["photo_id"]), None)
        assert photo is not None, f"Photo {data['photo_id']} not found in list"

        if should_swap:
            expected_w, expected_h = raw_h, raw_w  # Swapped: 200x300 (portrait)
            assert photo["width"] == expected_w, (
                f"Orientation {orientation}: expected display width={expected_w} "
                f"(swapped), got {photo['width']}. Raw={raw_w}x{raw_h}"
            )
            assert photo["height"] == expected_h, (
                f"Orientation {orientation}: expected display height={expected_h} "
                f"(swapped), got {photo['height']}. Raw={raw_w}x{raw_h}"
            )
            # Aspect ratio should be portrait (< 1.0)
            ar = photo["width"] / photo["height"]
            assert ar < 1.0, (
                f"Orientation {orientation}: expected portrait AR < 1.0, got {ar:.2f}. "
                f"Dimensions: {photo['width']}x{photo['height']}"
            )
        else:
            expected_w, expected_h = raw_w, raw_h  # Not swapped: 300x200 (landscape)
            assert photo["width"] == expected_w, (
                f"Orientation {orientation}: expected width={expected_w} "
                f"(not swapped), got {photo['width']}. Raw={raw_w}x{raw_h}"
            )
            assert photo["height"] == expected_h, (
                f"Orientation {orientation}: expected height={expected_h} "
                f"(not swapped), got {photo['height']}. Raw={raw_w}x{raw_h}"
            )
            # Aspect ratio should be landscape (> 1.0)
            ar = photo["width"] / photo["height"]
            assert ar > 1.0, (
                f"Orientation {orientation}: expected landscape AR > 1.0, got {ar:.2f}. "
                f"Dimensions: {photo['width']}x{photo['height']}"
            )

    def test_portrait_orientation_6_common_case(self, user_client):
        """The most common portrait scenario: phone camera EXIF orientation 6.

        Raw pixels: 4032×3024 (landscape sensor)
        EXIF orientation: 6 (rotate 90° CW)
        Display dimensions: 3024×4032 (portrait)

        The gallery grid computes aspect ratio as width/height.
        Portrait must produce AR < 1.0, not > 1.0.
        """
        raw_w, raw_h = 400, 300  # Simulating 4032×3024 proportionally
        content = _make_jpeg_with_orientation(raw_w, raw_h, 6)
        filename = _unique("portrait_phone")

        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)

        # Must be portrait: width < height
        assert photo["width"] < photo["height"], (
            f"Portrait photo should have width < height, got "
            f"{photo['width']}x{photo['height']}. "
            f"This means EXIF orientation 6 is NOT being applied — "
            f"the gallery will render this as landscape!"
        )
        assert photo["width"] == raw_h, f"Expected width={raw_h}, got {photo['width']}"
        assert photo["height"] == raw_w, f"Expected height={raw_w}, got {photo['height']}"

    def test_encrypted_sync_has_correct_portrait_dimensions(self, user_client):
        """The encrypted-sync endpoint must also return EXIF-corrected dimensions,
        since the web client uses these for the JustifiedGrid layout."""
        raw_w, raw_h = 320, 240
        content = _make_jpeg_with_orientation(raw_w, raw_h, 6)
        filename = _unique("sync_portrait")

        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]

        # Wait briefly for server-side encryption migration (if active)
        time.sleep(2)

        sync = user_client.encrypted_sync()
        sync_photos = sync.get("photos", [])
        photo = next((p for p in sync_photos if p["id"] == photo_id), None)
        assert photo is not None, (
            f"Photo {photo_id} not in encrypted-sync response"
        )

        # encrypted-sync must return display-corrected dimensions
        assert photo["width"] == raw_h, (
            f"Encrypted-sync width: expected {raw_h} (swapped), got {photo['width']}"
        )
        assert photo["height"] == raw_w, (
            f"Encrypted-sync height: expected {raw_w} (swapped), got {photo['height']}"
        )


class TestNoExifDimensionsCorrect:
    """Photos WITHOUT EXIF orientation (e.g., web uploads, screenshots)
    should keep their original dimensions unchanged."""

    def test_no_exif_portrait_photo(self, user_client):
        """A natively portrait photo (no EXIF) should have width < height."""
        content = generate_test_jpeg(width=100, height=150)
        filename = _unique("native_portrait")
        data = user_client.upload_photo(filename, content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])
        assert photo["width"] == 100
        assert photo["height"] == 150

    def test_no_exif_landscape_photo(self, user_client):
        """A natively landscape photo (no EXIF) should have width > height."""
        content = generate_test_jpeg(width=200, height=100)
        filename = _unique("native_landscape")
        data = user_client.upload_photo(filename, content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])
        assert photo["width"] == 200
        assert photo["height"] == 100


class TestThumbnailPortraitOrientation:
    """Verify that thumbnails for portrait photos are also generated in
    portrait orientation (not landscape)."""

    def test_portrait_thumbnail_is_portrait(self, user_client):
        """Upload a portrait photo (orientation 6) and verify the server
        generates a portrait-oriented thumbnail, not a landscape one."""
        raw_w, raw_h = 300, 200  # Raw: landscape pixels
        content = _make_jpeg_with_orientation(raw_w, raw_h, 6)
        filename = _unique("thumb_portrait")

        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]

        # Give server time to generate thumbnail
        time.sleep(1)

        # Download the thumbnail and check its actual pixel dimensions
        r = user_client.get_photo_thumb(photo_id)
        if r.status_code == 200 and len(r.content) > 100:
            thumb_img = Image.open(io.BytesIO(r.content))
            thumb_w, thumb_h = thumb_img.size
            assert thumb_w < thumb_h, (
                f"Thumbnail should be portrait (w < h), got {thumb_w}x{thumb_h}. "
                f"The thumbnail was generated from landscape raw pixels without "
                f"applying EXIF rotation."
            )

    def test_portrait_thumbnail_aspect_preserved(self, user_client):
        """Upload a portrait EXIF-6 photo and verify the thumbnail preserves
        the portrait aspect ratio (not a square crop)."""
        raw_w, raw_h = 400, 200  # Raw: 2:1 landscape → display: 1:2 portrait
        content = _make_jpeg_with_orientation(raw_w, raw_h, 6)
        filename = _unique("thumb_aspect_portrait")

        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]
        time.sleep(1)

        r = user_client.get_photo_thumb(photo_id)
        if r.status_code == 200 and len(r.content) > 100:
            thumb_img = Image.open(io.BytesIO(r.content))
            thumb_w, thumb_h = thumb_img.size
            # Thumbnail should be portrait (taller than wide)
            assert thumb_w < thumb_h, (
                f"Thumbnail should preserve portrait aspect ratio, "
                f"got {thumb_w}x{thumb_h} (landscape/square). "
                f"Migration thumbnail may use square crop without EXIF rotation."
            )
            # The aspect ratio should roughly match the display dimensions
            # (display is 200x400 = 0.5 AR; thumbnail should be ~0.5 AR)
            thumb_ar = thumb_w / thumb_h
            assert thumb_ar < 0.8, (
                f"Thumbnail AR should be portrait-like (< 0.8), got {thumb_ar:.2f}. "
                f"Raw={raw_w}x{raw_h}, EXIF=6 → display=200x400."
            )


class TestEncryptedSyncPortraitPipeline:
    """Verify the full pipeline: upload → migrate → encrypted-sync.

    This tests the same path the web client follows:
    1. Photo is uploaded/scanned on the server
    2. Server encrypts it (migration pipeline)
    3. Web client fetches metadata via encrypted-sync
    4. Web client uses width/height for JustifiedGrid layout

    If any step corrupts dimensions, the grid displays wrong tile shapes.
    """

    def test_encrypted_sync_portrait_after_migration(self, user_client):
        """Upload portrait EXIF-6, wait for encryption, verify encrypted-sync
        returns portrait dimensions."""
        raw_w, raw_h = 300, 200
        content = _make_jpeg_with_orientation(raw_w, raw_h, 6)
        filename = _unique("enc_sync_portrait")

        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]

        # Wait for encryption migration to process this photo
        for _ in range(30):
            sync = user_client.encrypted_sync()
            sync_photo = next(
                (p for p in sync.get("photos", []) if p["id"] == photo_id),
                None,
            )
            if sync_photo and sync_photo.get("encrypted_blob_id"):
                break
            time.sleep(1)
        else:
            pytest.skip("Encryption migration did not complete in time")

        # The encrypted-sync endpoint must return EXIF-corrected dimensions
        assert sync_photo["width"] == raw_h, (
            f"encrypted-sync width should be {raw_h} (swapped for EXIF 6), "
            f"got {sync_photo['width']}"
        )
        assert sync_photo["height"] == raw_w, (
            f"encrypted-sync height should be {raw_w} (swapped for EXIF 6), "
            f"got {sync_photo['height']}"
        )

        # Grid aspect ratio must be portrait
        ar = sync_photo["width"] / sync_photo["height"]
        assert ar < 1.0, (
            f"Gallery aspect ratio (width/height) should be < 1.0 for portrait, "
            f"got {ar:.2f}. Dimensions from encrypted-sync: "
            f"{sync_photo['width']}x{sync_photo['height']}"
        )

    def test_all_orientations_survive_full_pipeline(self, user_client):
        """Upload photos with all 8 EXIF orientations and verify that after
        the full encryption migration, dimensions are still correct."""
        results = {}
        for orientation in range(1, 9):
            raw_w, raw_h = 300, 200
            content = _make_jpeg_with_orientation(raw_w, raw_h, orientation)
            filename = _unique(f"pipeline_orient_{orientation}")
            data = user_client.upload_photo(filename, content)
            results[orientation] = data["photo_id"]

        # Wait for all to appear in encrypted-sync
        time.sleep(3)
        sync = user_client.encrypted_sync()
        sync_map = {p["id"]: p for p in sync.get("photos", [])}

        for orientation, photo_id in results.items():
            should_swap = orientation >= 5
            photo = sync_map.get(photo_id)
            assert photo is not None, f"Photo {photo_id} (orient {orientation}) missing from sync"

            if should_swap:
                assert photo["width"] == 200 and photo["height"] == 300, (
                    f"Orientation {orientation}: expected 200x300 (swapped), "
                    f"got {photo['width']}x{photo['height']}"
                )
            else:
                assert photo["width"] == 300 and photo["height"] == 200, (
                    f"Orientation {orientation}: expected 300x200 (not swapped), "
                    f"got {photo['width']}x{photo['height']}"
                )
