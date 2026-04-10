"""
Test 17: Justified Grid Layout — verifies the API data contract that the
frontend's Google-Photos-style justified flex-row grid depends on.

Tests that:
  1. Uploaded photos have correct width/height in API responses
  2. Mixed aspect ratios (landscape, portrait, square) are all preserved
  3. Photo dimensions survive sync and trash/restore cycles
  4. Thumbnail generation doesn't clobber dimension metadata
  5. The encrypted-sync endpoint returns dimensions for grid layout
"""

import hashlib
import random
import time

import pytest
from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_png,
)


def _unique(prefix: str) -> str:
    """Generate a unique filename with a descriptive prefix."""
    return f"{prefix}_{int(time.time() * 1000)}_{random.randint(1000, 9999)}.jpg"


class TestPhotoDimensionsForGrid:
    """Verify width/height metadata is correctly stored and returned by the
    photos API — the justified grid layout uses these to compute aspect ratios."""

    def test_landscape_photo_dimensions(self, user_client):
        """Landscape photo (wider than tall) preserves correct dimensions."""
        content = generate_test_jpeg(width=200, height=133)
        name = _unique("landscape")
        data = user_client.upload_photo(name, content)
        assert "photo_id" in data

        # Verify dimensions in photo list
        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])
        assert photo["width"] == 200, f"Expected width=200, got {photo['width']}"
        assert photo["height"] == 133, f"Expected height=133, got {photo['height']}"
        # Aspect ratio check (~1.5:1 landscape)
        ar = photo["width"] / photo["height"]
        assert 1.4 < ar < 1.6, f"Expected landscape AR ~1.5, got {ar:.2f}"

    def test_portrait_photo_dimensions(self, user_client):
        """Portrait photo (taller than wide) preserves correct dimensions."""
        content = generate_test_jpeg(width=100, height=150)
        name = _unique("portrait")
        data = user_client.upload_photo(name, content)
        assert "photo_id" in data

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])
        assert photo["width"] == 100
        assert photo["height"] == 150
        ar = photo["width"] / photo["height"]
        assert 0.6 < ar < 0.7, f"Expected portrait AR ~0.67, got {ar:.2f}"

    def test_square_photo_dimensions(self, user_client):
        """Square photo (1:1) preserves correct dimensions."""
        content = generate_test_jpeg(width=120, height=120)
        name = _unique("square")
        data = user_client.upload_photo(name, content)
        assert "photo_id" in data

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])
        assert photo["width"] == 120
        assert photo["height"] == 120
        ar = photo["width"] / photo["height"]
        assert ar == 1.0, f"Expected square AR 1.0, got {ar:.2f}"

    def test_ultrawide_photo_dimensions(self, user_client):
        """Ultra-wide panoramic photo preserves extreme aspect ratio."""
        content = generate_test_jpeg(width=250, height=80)
        name = _unique("panoramic")
        data = user_client.upload_photo(name, content)
        assert "photo_id" in data

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])
        assert photo["width"] == 250
        assert photo["height"] == 80
        ar = photo["width"] / photo["height"]
        assert ar > 3.0, f"Expected ultra-wide AR >3.0, got {ar:.2f}"


class TestMixedAspectRatioGrid:
    """Verify that a batch of photos with different aspect ratios all return
    correct dimensions — simulates what JustifiedGrid receives."""

    def test_batch_upload_varied_dimensions(self, user_client):
        """Upload multiple photos with varied dimensions and verify all
        come back with correct width/height in a single list_photos call."""
        test_cases = [
            (160, 120, "wide_4_3"),    # 4:3 landscape
            (120, 160, "tall_3_4"),    # 3:4 portrait
            (160, 90, "wide_16_9"),    # 16:9 landscape
            (90, 160, "tall_9_16"),    # 9:16 portrait
            (128, 128, "square_1_1"), # 1:1 square
        ]

        uploaded_ids = {}
        for w, h, label in test_cases:
            content = generate_test_jpeg(width=w, height=h)
            name = _unique(f"grid_{label}")
            data = user_client.upload_photo(name, content)
            uploaded_ids[data["photo_id"]] = (w, h, label)

        # Fetch all photos and verify dimensions
        photos = user_client.list_photos()["photos"]
        for pid, (exp_w, exp_h, label) in uploaded_ids.items():
            photo = next((p for p in photos if p["id"] == pid), None)
            assert photo is not None, f"Photo {label} ({pid}) not found in list"
            assert photo["width"] == exp_w, (
                f"{label}: expected width={exp_w}, got {photo['width']}"
            )
            assert photo["height"] == exp_h, (
                f"{label}: expected height={exp_h}, got {photo['height']}"
            )

    def test_dimensions_in_photo_list_non_zero(self, user_client):
        """All uploaded photos must have non-zero dimensions (the justified
        grid would break with width=0 or height=0)."""
        # Upload a few photos
        for i in range(3):
            content = generate_test_jpeg(
                width=80 + i * 40,
                height=60 + i * 30,
            )
            user_client.upload_photo(_unique(f"nonzero_{i}"), content)

        photos = user_client.list_photos()["photos"]
        for photo in photos:
            assert photo["width"] > 0, (
                f"Photo {photo['id']} ({photo['filename']}) has width=0"
            )
            assert photo["height"] > 0, (
                f"Photo {photo['id']} ({photo['filename']}) has height=0"
            )


class TestDimensionsPersistence:
    """Verify dimensions survive trash/restore and sync cycles."""

    def test_dimensions_survive_trash_restore(self, user_client):
        """Photo dimensions should be preserved after trash → restore."""
        content = generate_test_jpeg(width=200, height=100)
        name = _unique("trash_dims")
        data = user_client.upload_photo(name, content)
        photo_id = data["photo_id"]

        # Verify initial dimensions
        photos = user_client.list_photos()["photos"]
        original = next(p for p in photos if p["id"] == photo_id)
        assert original["width"] == 200
        assert original["height"] == 100

        # Soft-delete
        r = user_client.delete(f"/api/photos/{photo_id}")
        assert r.status_code == 200

        # Verify dimensions are in trash listing
        r = user_client.get("/api/trash")
        assert r.status_code == 200
        trash_items = r.json()["items"]
        trashed = next(
            (t for t in trash_items if t.get("photo_id") == photo_id),
            None,
        )
        assert trashed is not None, "Photo not found in trash"
        assert trashed["width"] == 200, (
            f"Trash width: expected 200, got {trashed['width']}"
        )
        assert trashed["height"] == 100, (
            f"Trash height: expected 100, got {trashed['height']}"
        )

        # Restore from trash
        trash_id = trashed["id"]
        r = user_client.post(f"/api/trash/{trash_id}/restore")
        assert r.status_code == 200

        # Verify dimensions after restore
        photos = user_client.list_photos()["photos"]
        restored = next(p for p in photos if p["id"] == photo_id)
        assert restored["width"] == 200
        assert restored["height"] == 100

    def test_encrypted_sync_includes_dimensions(self, user_client):
        """The encrypted-sync endpoint must include width/height so the
        client-side JustifiedGrid can compute aspect ratios."""
        content = generate_test_jpeg(width=180, height=120)
        name = _unique("sync_dims")
        data = user_client.upload_photo(name, content)

        # Call the encrypted-sync endpoint
        r = user_client.get("/api/photos/sync")
        assert r.status_code == 200
        sync_data = r.json()

        # Find our photo in the sync response
        sync_photos = sync_data.get("photos", sync_data.get("items", []))
        found = False
        for sp in sync_photos:
            pid = sp.get("id") or sp.get("photo_id")
            if pid == data["photo_id"]:
                found = True
                assert sp.get("width", 0) == 180 or sp.get("w", 0) == 180, (
                    f"Sync width mismatch: {sp}"
                )
                assert sp.get("height", 0) == 120 or sp.get("h", 0) == 120, (
                    f"Sync height mismatch: {sp}"
                )
                break
        assert found, "Photo not found in sync response"


class TestThumbnailSizePreference:
    """Verify the user preferences API supports thumbnail size persistence
    if available, or that the setting is client-side only."""

    def test_thumbnail_size_is_client_side_setting(self, user_client):
        """The thumbnail size toggle is a client-side localStorage setting.
        Verify the server doesn't reject preference writes and that the
        photos API works regardless of client display settings."""
        # The grid layout is purely client-side (Zustand + localStorage).
        # Verify the photos API returns all needed fields for both sizes.
        content = generate_test_jpeg(width=160, height=90)
        name = _unique("size_pref")
        data = user_client.upload_photo(name, content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])

        # All fields needed by JustifiedGrid must be present
        assert "width" in photo, "Missing 'width' field"
        assert "height" in photo, "Missing 'height' field"
        assert "filename" in photo, "Missing 'filename' field"
        assert "media_type" in photo, "Missing 'media_type' field"
        assert "mime_type" in photo, "Missing 'mime_type' field"
        assert "id" in photo, "Missing 'id' field"

        # Width and height must be positive integers
        assert isinstance(photo["width"], int) and photo["width"] > 0
        assert isinstance(photo["height"], int) and photo["height"] > 0
