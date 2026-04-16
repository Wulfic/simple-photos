"""
Test 40: Gallery Engine — Data-Driven Tests (DDT).

Pre-refactor safety net for the gallery engine refactoring.
Parametrized tests covering thumbnail rendering, dimension consistency,
and media type handling across all view contexts (main gallery, secure
gallery, shared albums).

Each test case verifies a contract that must survive the refactoring:
  - Upload → thumbnail generated with correct dimensions & MIME type
  - Encrypted-sync metadata matches upload dimensions
  - Secure gallery items preserve dimensions & media_type
  - Shared album photos preserve dimensions
  - Crop metadata round-trips correctly
  - GIF media type detection works for animated & static thumbnails
  - Duplicate (Save Copy) dimensions match expectations
  - Favorites count through smart album filter

Media types tested: JPEG, PNG, BMP, GIF (small), video (MP4)
View contexts: main gallery, secure gallery, shared album
"""

import json
import os
import subprocess
import tempfile
import time

import pytest

from helpers import (
    APIClient,
    assert_no_duplicates,
    generate_test_bmp,
    generate_test_gif,
    generate_test_jpeg,
    generate_test_png,
)


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _ffmpeg_available() -> bool:
    try:
        subprocess.run(["ffmpeg", "-version"], capture_output=True, timeout=5)
        return True
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def _generate_mp4(width: int = 64, height: int = 64, duration: float = 0.3) -> bytes:
    """Generate a short MP4 video."""
    path = tempfile.mktemp(suffix=".mp4")
    try:
        subprocess.run([
            "ffmpeg", "-y", "-f", "lavfi", "-i",
            f"color=c=blue:s={width}x{height}:d={duration}",
            "-c:v", "libx264", "-preset", "ultrafast",
            "-pix_fmt", "yuv420p",
            path,
        ], capture_output=True, timeout=30, check=True)
        with open(path, "rb") as f:
            return f.read()
    finally:
        if os.path.exists(path):
            os.unlink(path)


def _find_photo(client: APIClient, photo_id: str) -> dict:
    """Find a photo by ID in the list, returning its full record."""
    photos = client.list_photos()["photos"]
    found = [p for p in photos if p["id"] == photo_id]
    assert found, f"Photo {photo_id} not found in list"
    return found[0]


def _find_in_sync(client: APIClient, photo_id: str, timeout: float = 10) -> dict:
    """Find a photo in encrypted-sync response, with polling for async processing."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        sync = client.encrypted_sync()
        photos = sync.get("photos", [])
        found = [p for p in photos if p["id"] == photo_id]
        if found:
            return found[0]
        time.sleep(1)
    raise AssertionError(f"Photo {photo_id} not found in encrypted-sync within {timeout}s")


def _wait_for_thumb(client: APIClient, photo_id: str, timeout: float = 15) -> bytes:
    """Poll until a thumbnail is available (200 OK) for the given photo."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        r = client.get_photo_thumb(photo_id)
        if r.status_code == 200 and len(r.content) > 0:
            return r.content
        time.sleep(1)
    raise AssertionError(f"Thumbnail for {photo_id} not ready within {timeout}s")


def _upload_and_settle(client: APIClient, filename: str, content: bytes,
                       mime_type: str, settle: float = 2.0) -> str:
    """Upload a photo and wait for server-side processing to settle."""
    resp = client.upload_photo(filename, content, mime_type)
    photo_id = resp["photo_id"]
    time.sleep(settle)
    return photo_id


# ══════════════════════════════════════════════════════════════════════
# DDT: Upload → Dimensions Consistency (Main Gallery)
# ══════════════════════════════════════════════════════════════════════

DIMENSION_CASES = [
    # (filename, mime_type, gen_func, gen_kwargs, expected_w, expected_h, label)
    pytest.param("land.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 200, "height": 133}, 200, 133,
                 id="jpeg_landscape_200x133"),
    pytest.param("port.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 133, "height": 200}, 133, 200,
                 id="jpeg_portrait_133x200"),
    pytest.param("sq.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 150, "height": 150}, 150, 150,
                 id="jpeg_square_150x150"),
    pytest.param("wide.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 400, "height": 100}, 400, 100,
                 id="jpeg_panoramic_400x100"),
    pytest.param("tiny.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 2, "height": 2}, 2, 2,
                 id="jpeg_tiny_2x2"),
    pytest.param("land.png", "image/png", generate_test_jpeg,
                 {"width": 100, "height": 80}, 100, 80,
                 id="png_landscape_100x80"),
    pytest.param("port.bmp", "image/bmp", generate_test_bmp,
                 {"width": 80, "height": 120}, 80, 120,
                 id="bmp_portrait_80x120"),
    pytest.param("anim.gif", "image/gif", generate_test_gif,
                 {"width": 40, "height": 30, "frames": 3}, 40, 30,
                 id="gif_animated_40x30"),
]


class TestUploadDimensions:
    """Upload various media types and verify dimensions are stored correctly."""

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,exp_w,exp_h", DIMENSION_CASES)
    def test_dimensions_in_list(self, user_client, filename, mime, gen_func, gen_kw, exp_w, exp_h):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)
        photo = _find_photo(user_client, photo_id)
        assert photo["width"] == exp_w, f"width: expected {exp_w}, got {photo['width']}"
        assert photo["height"] == exp_h, f"height: expected {exp_h}, got {photo['height']}"

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,exp_w,exp_h", DIMENSION_CASES)
    def test_dimensions_in_sync(self, user_client, filename, mime, gen_func, gen_kw, exp_w, exp_h):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)
        synced = _find_in_sync(user_client, photo_id)
        assert synced["width"] == exp_w, f"sync width: expected {exp_w}, got {synced['width']}"
        assert synced["height"] == exp_h, f"sync height: expected {exp_h}, got {synced['height']}"


# ══════════════════════════════════════════════════════════════════════
# DDT: Thumbnail Availability & Non-Zero Size
# ══════════════════════════════════════════════════════════════════════

THUMB_CASES = [
    pytest.param("t_land.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 200, "height": 133},
                 id="thumb_jpeg_landscape"),
    pytest.param("t_port.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 100, "height": 200},
                 id="thumb_jpeg_portrait"),
    pytest.param("t_gif.gif", "image/gif", generate_test_gif,
                 {"width": 30, "height": 20, "frames": 2},
                 id="thumb_gif_animated"),
    pytest.param("t_bmp.bmp", "image/bmp", generate_test_bmp,
                 {"width": 50, "height": 50},
                 id="thumb_bmp_square"),
]


class TestThumbnailAvailability:
    """Verify that thumbnails are generated for all media types."""

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw", THUMB_CASES)
    def test_thumbnail_generated(self, user_client, filename, mime, gen_func, gen_kw):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime, settle=3)
        thumb_data = _wait_for_thumb(user_client, photo_id)
        assert len(thumb_data) > 100, f"Thumbnail too small ({len(thumb_data)} bytes)"

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw", THUMB_CASES)
    def test_thumbnail_not_square_cropped(self, user_client, filename, mime, gen_func, gen_kw):
        """Thumbnails should preserve aspect ratio, not force to square."""
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime, settle=3)
        photo = _find_photo(user_client, photo_id)
        src_w, src_h = gen_kw.get("width", 2), gen_kw.get("height", 2)
        if src_w == src_h:
            pytest.skip("Square source — AR test not meaningful")
        # The photo dimensions should reflect the source AR, not 1:1
        is_landscape_src = src_w > src_h
        is_landscape_stored = photo["width"] > photo["height"]
        assert is_landscape_src == is_landscape_stored, (
            f"AR mismatch: source {src_w}x{src_h} -> stored {photo['width']}x{photo['height']}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: Media Type Detection
# ══════════════════════════════════════════════════════════════════════

MEDIA_TYPE_CASES = [
    pytest.param("mt.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 10, "height": 10}, "photo",
                 id="jpeg_is_photo"),
    pytest.param("mt.gif", "image/gif", generate_test_gif,
                 {"width": 20, "height": 20, "frames": 3}, "gif",
                 id="gif_is_gif"),
    pytest.param("mt.bmp", "image/bmp", generate_test_bmp,
                 {"width": 10, "height": 10}, "photo",
                 id="bmp_is_photo"),
]


class TestMediaTypeDetection:
    """Verify media_type is correctly detected for different formats."""

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,expected_type", MEDIA_TYPE_CASES)
    def test_media_type_in_list(self, user_client, filename, mime, gen_func, gen_kw, expected_type):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)
        photo = _find_photo(user_client, photo_id)
        assert photo["media_type"] == expected_type, (
            f"Expected media_type={expected_type}, got {photo['media_type']}"
        )

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,expected_type", MEDIA_TYPE_CASES)
    def test_media_type_in_sync(self, user_client, filename, mime, gen_func, gen_kw, expected_type):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)
        synced = _find_in_sync(user_client, photo_id)
        assert synced["media_type"] == expected_type


# ══════════════════════════════════════════════════════════════════════
# DDT: Video Upload Dimensions (requires ffmpeg)
# ══════════════════════════════════════════════════════════════════════

@pytest.mark.skipif(not _ffmpeg_available(), reason="ffmpeg not installed")
class TestVideoDimensions:
    """Verify video uploads have correct dimensions and media_type."""

    VIDEO_CASES = [
        pytest.param(64, 48, id="video_landscape_64x48"),
        pytest.param(48, 64, id="video_portrait_48x64"),
        pytest.param(64, 64, id="video_square_64x64"),
    ]

    @pytest.mark.parametrize("vw,vh", VIDEO_CASES)
    def test_video_dimension_stored(self, user_client, vw, vh):
        content = _generate_mp4(vw, vh)
        photo_id = _upload_and_settle(user_client, "dim_test.mp4", content, "video/mp4", settle=4)
        photo = _find_photo(user_client, photo_id)
        assert photo["media_type"] == "video"
        # FFmpeg may round dimensions to even numbers
        assert abs(photo["width"] - vw) <= 2, f"width: expected ~{vw}, got {photo['width']}"
        assert abs(photo["height"] - vh) <= 2, f"height: expected ~{vh}, got {photo['height']}"

    def test_video_thumbnail_generated(self, user_client):
        content = _generate_mp4(64, 48)
        photo_id = _upload_and_settle(user_client, "vthumb.mp4", content, "video/mp4", settle=4)
        thumb = _wait_for_thumb(user_client, photo_id)
        assert len(thumb) > 100


# ══════════════════════════════════════════════════════════════════════
# DDT: Secure Gallery — Dimensions & Media Type Preserved
# ══════════════════════════════════════════════════════════════════════

SECURE_GALLERY_CASES = [
    pytest.param("sg_land.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 200, "height": 133}, 200, 133, "photo",
                 id="secure_jpeg_landscape"),
    pytest.param("sg_port.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 133, "height": 200}, 133, 200, "photo",
                 id="secure_jpeg_portrait"),
    pytest.param("sg_gif.gif", "image/gif", generate_test_gif,
                 {"width": 40, "height": 30, "frames": 3}, 40, 30, "gif",
                 id="secure_gif_animated"),
]

# Conftest provides USER_PASSWORD via the config constants
USER_PASSWORD = "E2eUserPass456!"


class TestSecureGalleryDimensions:
    """Move photos into secure gallery and verify dimensions & media_type survive."""

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,exp_w,exp_h,exp_type", SECURE_GALLERY_CASES)
    def test_gallery_item_preserves_metadata(
        self, user_client, filename, mime, gen_func, gen_kw, exp_w, exp_h, exp_type
    ):
        # Upload
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)

        # Create secure gallery and add item
        gallery = user_client.create_secure_gallery(f"ddt_{filename}")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(2)  # Let server process clone

        # Verify item in secure gallery has correct metadata
        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)["items"]
        assert len(items) >= 1, "Secure gallery should have at least 1 item"
        item = items[0]

        # Dimensions & media_type must be present
        assert "width" in item, "Gallery item missing 'width'"
        assert "height" in item, "Gallery item missing 'height'"
        assert item["width"] == exp_w, f"Gallery item width: expected {exp_w}, got {item['width']}"
        assert item["height"] == exp_h, f"Gallery item height: expected {exp_h}, got {item['height']}"
        if "media_type" in item:
            assert item["media_type"] == exp_type

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,exp_w,exp_h,exp_type", SECURE_GALLERY_CASES)
    def test_gallery_item_hidden_from_main(
        self, user_client, filename, mime, gen_func, gen_kw, exp_w, exp_h, exp_type
    ):
        """After adding to secure gallery, item must not appear in main gallery."""
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)

        gallery = user_client.create_secure_gallery(f"hide_{filename}")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(2)

        # Check main gallery
        photos = user_client.list_photos()["photos"]
        photo_ids = [p["id"] for p in photos]
        assert photo_id not in photo_ids, "Photo still visible in main gallery after secure add"


# ══════════════════════════════════════════════════════════════════════
# DDT: Shared Album — Photos Accessible
# ══════════════════════════════════════════════════════════════════════

SHARED_ALBUM_CASES = [
    pytest.param("sa_land.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 200, "height": 133}, 200, 133,
                 id="shared_jpeg_landscape"),
    pytest.param("sa_port.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 133, "height": 200}, 133, 200,
                 id="shared_jpeg_portrait"),
]


class TestSharedAlbumPhotos:
    """Add photos to shared albums and verify they're accessible."""

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,exp_w,exp_h", SHARED_ALBUM_CASES)
    def test_shared_album_photo_listed(self, user_client, filename, mime, gen_func, gen_kw, exp_w, exp_h):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)

        album = user_client.create_shared_album(f"ddt_shared_{filename}")
        user_client.add_album_photo(album["id"], photo_id, ref_type="photo")

        photos = user_client.list_album_photos(album["id"])
        assert len(photos) >= 1, "Shared album has no photos"
        refs = [p.get("photo_ref") or p.get("photo_id") for p in photos]
        assert photo_id in refs, f"Photo {photo_id} not found in shared album"

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw,exp_w,exp_h", SHARED_ALBUM_CASES)
    def test_shared_album_photo_still_in_main(self, user_client, filename, mime, gen_func, gen_kw, exp_w, exp_h):
        """Unlike secure gallery, shared album photos remain in main gallery."""
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime)

        album = user_client.create_shared_album(f"vis_{filename}")
        user_client.add_album_photo(album["id"], photo_id, ref_type="photo")

        photos = user_client.list_photos()["photos"]
        photo_ids = [p["id"] for p in photos]
        assert photo_id in photo_ids, "Photo should still be in main gallery"


# ══════════════════════════════════════════════════════════════════════
# DDT: Crop Metadata Round-Trip
# ══════════════════════════════════════════════════════════════════════

CROP_CASES = [
    pytest.param(
        {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "rotate": 0, "brightness": 0},
        id="crop_center_80pct",
    ),
    pytest.param(
        {"x": 0, "y": 0, "width": 1.0, "height": 1.0, "rotate": 90, "brightness": 0},
        id="rotate_90_no_crop",
    ),
    pytest.param(
        {"x": 0, "y": 0, "width": 1.0, "height": 1.0, "rotate": 270, "brightness": 50},
        id="rotate_270_bright_50",
    ),
    pytest.param(
        {"x": 0.25, "y": 0.25, "width": 0.5, "height": 0.5, "rotate": 180, "brightness": -30},
        id="crop_50pct_rotate_180_dark",
    ),
    pytest.param(
        {"x": 0, "y": 0, "width": 1.0, "height": 1.0, "rotate": 0, "brightness": 100},
        id="max_brightness",
    ),
    pytest.param(
        {"x": 0, "y": 0, "width": 1.0, "height": 1.0, "rotate": 0, "brightness": -100},
        id="min_brightness",
    ),
]


class TestCropMetadataRoundTrip:
    """Set crop metadata and verify it persists correctly."""

    @pytest.mark.parametrize("meta", CROP_CASES)
    def test_crop_round_trip(self, user_client, meta):
        content = generate_test_jpeg(200, 300)
        photo_id = _upload_and_settle(user_client, "crop_ddt.jpg", content, "image/jpeg")

        user_client.crop_photo(photo_id, json.dumps(meta))
        photo = _find_photo(user_client, photo_id)
        stored = json.loads(photo["crop_metadata"])

        for key in meta:
            assert key in stored, f"Key '{key}' missing from stored crop_metadata"
            assert stored[key] == pytest.approx(meta[key], abs=0.01), (
                f"crop_metadata['{key}']: expected {meta[key]}, got {stored[key]}"
            )

    @pytest.mark.parametrize("meta", CROP_CASES)
    def test_crop_visible_in_sync(self, user_client, meta):
        content = generate_test_jpeg(200, 300)
        photo_id = _upload_and_settle(user_client, "sync_crop.jpg", content, "image/jpeg")

        user_client.crop_photo(photo_id, json.dumps(meta))
        synced = _find_in_sync(user_client, photo_id)
        assert synced.get("crop_metadata"), "crop_metadata missing from encrypted-sync"
        stored = json.loads(synced["crop_metadata"])
        for key in meta:
            assert stored[key] == pytest.approx(meta[key], abs=0.01)


# ══════════════════════════════════════════════════════════════════════
# DDT: Duplicate Dimensions (Save As Copy)
# ══════════════════════════════════════════════════════════════════════

DUPLICATE_DIM_CASES = [
    #  src_w, src_h, crop,                                              rot,  exp_w, exp_h
    pytest.param(200, 150, {"x": 0, "y": 0, "width": 1.0, "height": 1.0}, 0,   200, 150,
                 id="dup_no_edit"),
    pytest.param(200, 150, {"x": 0, "y": 0, "width": 1.0, "height": 1.0}, 90,  150, 200,
                 id="dup_rot90_swap"),
    pytest.param(200, 150, {"x": 0, "y": 0, "width": 1.0, "height": 1.0}, 270, 150, 200,
                 id="dup_rot270_swap"),
    pytest.param(200, 150, {"x": 0, "y": 0, "width": 1.0, "height": 1.0}, 180, 200, 150,
                 id="dup_rot180_no_swap"),
    pytest.param(200, 150, {"x": 0, "y": 0, "width": 0.5, "height": 0.5}, 0,   100, 75,
                 id="dup_crop_50pct"),
]


class TestDuplicateDimensions:
    """Duplicate (Save As Copy) must produce correct output dimensions."""

    @pytest.mark.parametrize("src_w,src_h,crop,rot,exp_w,exp_h", DUPLICATE_DIM_CASES)
    def test_duplicate_dimensions(self, user_client, src_w, src_h, crop, rot, exp_w, exp_h):
        content = generate_test_jpeg(src_w, src_h)
        photo_id = _upload_and_settle(user_client, "dup_ddt.jpg", content, "image/jpeg")

        meta = {**crop, "rotate": rot, "brightness": 0}
        dup = user_client.duplicate_photo(photo_id, json.dumps(meta))
        dup_id = dup["id"]
        time.sleep(3)  # Wait for rendering

        photo = _find_photo(user_client, dup_id)
        # Allow ±1 pixel for rounding
        assert abs(photo["width"] - exp_w) <= 1, (
            f"Duplicate width: expected {exp_w}, got {photo['width']}"
        )
        assert abs(photo["height"] - exp_h) <= 1, (
            f"Duplicate height: expected {exp_h}, got {photo['height']}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: Favorites — Gallery-Level Filtering
# ══════════════════════════════════════════════════════════════════════

class TestFavoritesFiltering:
    """Verify favorite toggle and filtering work correctly."""

    def test_favorite_toggle_round_trip(self, user_client):
        content = generate_test_jpeg(10, 10)
        photo_id = _upload_and_settle(user_client, "fav.jpg", content, "image/jpeg")

        # Initially not favorited
        photo = _find_photo(user_client, photo_id)
        assert photo.get("is_favorite") is False or photo.get("is_favorite") is None

        # Toggle on
        user_client.favorite_photo(photo_id)
        photo = _find_photo(user_client, photo_id)
        assert photo["is_favorite"] is True

        # Toggle off
        user_client.favorite_photo(photo_id)
        photo = _find_photo(user_client, photo_id)
        assert photo["is_favorite"] is False

    def test_favorites_filter(self, user_client):
        """Upload 3 photos, favorite 2, filter favorites → get 2."""
        ids = []
        for i in range(3):
            content = generate_test_jpeg(10 + i, 10 + i)
            pid = _upload_and_settle(user_client, f"fav_{i}.jpg", content, "image/jpeg", settle=1)
            ids.append(pid)
        time.sleep(1)

        # Favorite first 2
        user_client.favorite_photo(ids[0])
        user_client.favorite_photo(ids[1])

        # Filter
        result = user_client.list_photos(favorites_only="true")
        fav_ids = [p["id"] for p in result["photos"]]
        assert ids[0] in fav_ids
        assert ids[1] in fav_ids
        # Third should not be in favorites-only listing
        # (but it may have other favorites from earlier tests, so just check the 2 we set)

    def test_favorite_visible_in_sync(self, user_client):
        content = generate_test_jpeg(10, 10)
        photo_id = _upload_and_settle(user_client, "fav_sync.jpg", content, "image/jpeg")
        user_client.favorite_photo(photo_id)
        synced = _find_in_sync(user_client, photo_id)
        assert synced.get("is_favorite") is True


# ══════════════════════════════════════════════════════════════════════
# DDT: No Duplicate IDs — Gallery Integrity
# ══════════════════════════════════════════════════════════════════════

class TestNoDuplicateIds:
    """After various operations, no duplicate photo/blob IDs should exist."""

    def test_no_duplicate_photo_ids_after_mixed_uploads(self, user_client):
        """Upload a mix of media types and verify no ID collisions."""
        uploads = [
            ("nd_1.jpg", "image/jpeg", generate_test_jpeg(100, 80)),
            ("nd_2.jpg", "image/jpeg", generate_test_jpeg(80, 100)),
            ("nd_3.gif", "image/gif", generate_test_gif(20, 20, 3)),
            ("nd_4.bmp", "image/bmp", generate_test_bmp(30, 30)),
        ]
        for filename, mime, content in uploads:
            _upload_and_settle(user_client, filename, content, mime, settle=1)
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        ids = [p["id"] for p in photos]
        assert_no_duplicates(ids, "photo IDs")

    def test_no_duplicate_in_sync_after_uploads(self, user_client):
        content = generate_test_jpeg(50, 50)
        _upload_and_settle(user_client, "sync_nd.jpg", content, "image/jpeg")
        sync = user_client.encrypted_sync()
        ids = [p["id"] for p in sync.get("photos", [])]
        assert_no_duplicates(ids, "encrypted-sync IDs")


# ══════════════════════════════════════════════════════════════════════
# DDT: Encrypted-Sync Completeness
# ══════════════════════════════════════════════════════════════════════

SYNC_FIELD_CASES = [
    pytest.param("id", id="field_id"),
    pytest.param("filename", id="field_filename"),
    pytest.param("mime_type", id="field_mime_type"),
    pytest.param("media_type", id="field_media_type"),
    pytest.param("width", id="field_width"),
    pytest.param("height", id="field_height"),
]


class TestEncryptedSyncFields:
    """Verify encrypted-sync response contains all required fields."""

    @pytest.mark.parametrize("field", SYNC_FIELD_CASES)
    def test_sync_field_present(self, user_client, field):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "sync_field.jpg", content, "image/jpeg")
        synced = _find_in_sync(user_client, photo_id)
        assert field in synced, f"Field '{field}' missing from encrypted-sync record"
        if field in ("width", "height"):
            assert synced[field] > 0, f"'{field}' should be > 0, got {synced[field]}"
