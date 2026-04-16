"""
Test 36: Edit Save E2E Regression Tests.

End-to-end tests that verify the complete edit lifecycle:
  - Upload → Edit (set crop) → Verify persistence → Re-read → Verify still there
  - Upload → Edit → Save Copy → Verify copy created with correct dimensions
  - Upload → Edit → Clear → Verify cleared
  - Multiple formats: JPEG, PNG, BMP, GIF
  - Encrypted mode: upload blob → edit → verify
  - Concurrent edits: edit photo A, edit photo B, verify both persist
  - Re-enter edit: save, then re-enter edit mode, verify loaded values

These tests are designed to catch regressions from Viewer.tsx changes,
save code path modifications, and frontend↔server sync issues.
"""

import json
import math
import time

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_png,
    generate_test_bmp,
    generate_test_gif,
)


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient, filename="test.jpg", content=None, mime="image/jpeg") -> str:
    data = client.upload_photo(filename, content or generate_test_jpeg(200, 300), mime)
    return data["photo_id"]


def _get_photo(client: APIClient, photo_id: str) -> dict:
    photos = client.list_photos()["photos"]
    found = [p for p in photos if p["id"] == photo_id]
    assert found, f"Photo {photo_id} not found"
    return found[0]


def _get_crop(client: APIClient, photo_id: str):
    photo = _get_photo(client, photo_id)
    raw = photo.get("crop_metadata")
    return json.loads(raw) if raw else None


# ══════════════════════════════════════════════════════════════════════
# 1. Full Edit Lifecycle — set, verify, re-read
# ══════════════════════════════════════════════════════════════════════

class TestEditLifecycle:
    """Complete edit save → verify → re-read cycle."""

    def test_brightness_persists_across_rereads(self, user_client):
        """Set brightness, re-read photo multiple times."""
        pid = _upload(user_client)
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": 42}
        user_client.crop_photo(pid, json.dumps(meta))

        # Read 3 times to verify persistence
        for _ in range(3):
            crop = _get_crop(user_client, pid)
            assert crop is not None
            assert crop["brightness"] == 42

    def test_rotation_persists_across_rereads(self, user_client):
        pid = _upload(user_client)
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 270, "brightness": 0}
        user_client.crop_photo(pid, json.dumps(meta))

        for _ in range(3):
            crop = _get_crop(user_client, pid)
            assert crop is not None
            assert crop["rotate"] == 270

    def test_crop_region_persists(self, user_client):
        pid = _upload(user_client)
        meta = {"x": 0.15, "y": 0.25, "width": 0.5, "height": 0.4, "rotate": 0, "brightness": 0}
        user_client.crop_photo(pid, json.dumps(meta))

        crop = _get_crop(user_client, pid)
        assert math.isclose(crop["x"], 0.15)
        assert math.isclose(crop["y"], 0.25)
        assert math.isclose(crop["width"], 0.5)
        assert math.isclose(crop["height"], 0.4)

    def test_edit_then_clear_then_edit_again(self, user_client):
        """Set edit → clear → set different edit → verify."""
        pid = _upload(user_client)

        # First edit
        meta1 = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 30}
        user_client.crop_photo(pid, json.dumps(meta1))
        assert _get_crop(user_client, pid)["rotate"] == 90

        # Clear
        r = user_client.put(f"/api/photos/{pid}/crop", json_data={"crop_metadata": None})
        assert r.status_code == 200
        assert _get_crop(user_client, pid) is None

        # Second edit (different values)
        meta2 = {"x": 0.1, "y": 0.2, "width": 0.8, "height": 0.6, "rotate": 180, "brightness": -20}
        user_client.crop_photo(pid, json.dumps(meta2))
        crop = _get_crop(user_client, pid)
        assert crop["rotate"] == 180
        assert crop["brightness"] == -20
        assert math.isclose(crop["x"], 0.1)


# ══════════════════════════════════════════════════════════════════════
# 2. Save Copy (Duplicate) — full lifecycle
# ══════════════════════════════════════════════════════════════════════

class TestSaveCopyLifecycle:
    def test_save_copy_creates_independent_photo(self, user_client):
        """Duplicate with edits creates a new photo that's independent."""
        pid = _upload(user_client)
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 30}
        dup = user_client.duplicate_photo(pid, json.dumps(meta))
        copy_id = dup["id"]

        # Copy exists in photo list
        photos = user_client.list_photos()["photos"]
        copy_entry = next((p for p in photos if p["id"] == copy_id), None)
        assert copy_entry is not None, "Copy should appear in photo list"

        # Copy has its own ID, different from original
        assert copy_id != pid

        # Copy has source_photo_id pointing to original
        assert dup.get("source_photo_id") == pid

    def test_save_copy_with_rotation_changes_dimensions(self, user_client):
        """A 200×300 portrait JPEG rotated 90° should produce ~300×200 landscape copy."""
        pid = _upload(user_client, content=generate_test_jpeg(200, 300))
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 0}
        dup = user_client.duplicate_photo(pid, json.dumps(meta))
        copy_id = dup["id"]

        copy_photo = _get_photo(user_client, copy_id)
        # After 90° rotation, width and height should swap
        assert copy_photo["width"] == 300, f"Expected width=300, got {copy_photo['width']}"
        assert copy_photo["height"] == 200, f"Expected height=200, got {copy_photo['height']}"

    def test_save_copy_has_null_crop_metadata(self, user_client):
        """Rendered copy should have crop_metadata=NULL (edits baked in)."""
        pid = _upload(user_client)
        meta = {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": 30}
        dup = user_client.duplicate_photo(pid, json.dumps(meta))
        assert dup.get("crop_metadata") is None

    def test_save_copy_original_untouched(self, user_client):
        """After saving a copy, the original's metadata is unchanged."""
        pid = _upload(user_client)
        # Set some edits on original first
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": 50}
        user_client.crop_photo(pid, json.dumps(meta))

        # Now duplicate with different edits
        dup_meta = {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "rotate": 180, "brightness": -30}
        user_client.duplicate_photo(pid, json.dumps(dup_meta))

        # Original should still have its original edits
        original_crop = _get_crop(user_client, pid)
        assert original_crop["brightness"] == 50
        assert original_crop["rotate"] == 0

    def test_save_copy_no_edits_plain_copy(self, user_client):
        """Duplicate without edits creates a plain file copy."""
        pid = _upload(user_client)
        dup = user_client.duplicate_photo(pid, None)
        copy_id = dup["id"]

        original = _get_photo(user_client, pid)
        copy = _get_photo(user_client, copy_id)

        # Dimensions should match
        assert copy["width"] == original["width"]
        assert copy["height"] == original["height"]

    def test_save_copy_filename_prefix(self, user_client):
        """Copy should have 'Copy of ' prefix in filename."""
        data = user_client.upload_photo("my_photo.jpg", generate_test_jpeg(100, 100))
        pid = data["photo_id"]
        dup = user_client.duplicate_photo(pid, None)
        assert dup["filename"].startswith("Copy of "), \
            f"Expected 'Copy of ...' but got '{dup['filename']}'"


# ══════════════════════════════════════════════════════════════════════
# 3. Multiple Formats
# ══════════════════════════════════════════════════════════════════════

class TestMultiFormatEdits:
    """Verify edits work across JPEG, PNG, BMP, GIF."""

    def test_edit_jpeg(self, user_client):
        pid = _upload(user_client, "test.jpg", generate_test_jpeg(150, 200), "image/jpeg")
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 25}
        user_client.crop_photo(pid, json.dumps(meta))
        assert _get_crop(user_client, pid)["brightness"] == 25

    def test_edit_png(self, user_client):
        pid = _upload(user_client, "test.png", generate_test_png(), "image/png")
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 180, "brightness": -10}
        user_client.crop_photo(pid, json.dumps(meta))
        crop = _get_crop(user_client, pid)
        assert crop["rotate"] == 180
        assert crop["brightness"] == -10

    def test_edit_bmp(self, user_client):
        pid = _upload(user_client, "test.bmp", generate_test_bmp(100, 100), "image/bmp")
        meta = {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "rotate": 0, "brightness": 40}
        user_client.crop_photo(pid, json.dumps(meta))
        crop = _get_crop(user_client, pid)
        assert crop["brightness"] == 40
        assert math.isclose(crop["x"], 0.1)

    def test_edit_gif(self, user_client):
        pid = _upload(user_client, "test.gif", generate_test_gif(), "image/gif")
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 270, "brightness": 0}
        user_client.crop_photo(pid, json.dumps(meta))
        assert _get_crop(user_client, pid)["rotate"] == 270

    def test_duplicate_png_with_rotation(self, user_client):
        """Duplicate a PNG with rotation and verify dimensions."""
        pid = _upload(user_client, "test.png", generate_test_png(), "image/png")
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 0}
        dup = user_client.duplicate_photo(pid, json.dumps(meta))
        copy = _get_photo(user_client, dup["id"])
        original = _get_photo(user_client, pid)
        # After 90° rotation, dimensions should swap
        assert copy["width"] == original["height"]
        assert copy["height"] == original["width"]


# ══════════════════════════════════════════════════════════════════════
# 4. Concurrent Edits on Different Photos
# ══════════════════════════════════════════════════════════════════════

class TestConcurrentEdits:
    def test_edit_two_photos_independently(self, user_client):
        """Editing photo A should not affect photo B."""
        pid_a = _upload(user_client, "photo_a.jpg", generate_test_jpeg(201, 301))
        pid_b = _upload(user_client, "photo_b.jpg", generate_test_jpeg(202, 302))

        meta_a = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 30}
        meta_b = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 180, "brightness": -50}

        user_client.crop_photo(pid_a, json.dumps(meta_a))
        user_client.crop_photo(pid_b, json.dumps(meta_b))

        crop_a = _get_crop(user_client, pid_a)
        crop_b = _get_crop(user_client, pid_b)

        assert crop_a["rotate"] == 90
        assert crop_a["brightness"] == 30
        assert crop_b["rotate"] == 180
        assert crop_b["brightness"] == -50

    def test_edit_many_photos_sequentially(self, user_client):
        """Edit 5 photos with different metadata, verify all persist."""
        photos = []
        for i in range(5):
            pid = _upload(user_client, f"batch_{i}.jpg", generate_test_jpeg(200 + i, 300 + i))
            meta = {"x": 0, "y": 0, "width": 1, "height": 1,
                    "rotate": (i * 90) % 360, "brightness": (i + 1) * 10}
            user_client.crop_photo(pid, json.dumps(meta))
            photos.append((pid, meta))

        for pid, expected_meta in photos:
            crop = _get_crop(user_client, pid)
            assert crop["rotate"] == expected_meta["rotate"]
            assert crop["brightness"] == expected_meta["brightness"]


# ══════════════════════════════════════════════════════════════════════
# 5. Encrypted Mode (blob-based upload)
# ══════════════════════════════════════════════════════════════════════

class TestEncryptedEdits:
    """Verify edit operations work in encrypted blob mode."""

    def test_encrypted_photo_crop_roundtrip(self, user_client):
        """Upload photo, set crop, verify — uses standard upload which
        the test server auto-encrypts when encryption is enabled."""
        pid = _upload(user_client, "encrypted_test.jpg", generate_test_jpeg(203, 303))

        # Set crop metadata
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 25}
        user_client.crop_photo(pid, json.dumps(meta))

        # Verify persistence
        crop = _get_crop(user_client, pid)
        assert crop is not None
        assert crop["rotate"] == 90
        assert crop["brightness"] == 25


# ══════════════════════════════════════════════════════════════════════
# 6. Regression: Save Edit After Viewer Reopen (simulates re-enter)
# ══════════════════════════════════════════════════════════════════════

class TestReopenReEdit:
    """Simulate the client reopening a photo with existing edits and
    modifying them — the 're-enter edit mode' scenario."""

    def test_reopen_and_modify_brightness(self, user_client):
        """Set brightness=30, then 'reopen' and change to brightness=60."""
        pid = _upload(user_client)

        # First edit
        meta1 = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": 30}
        user_client.crop_photo(pid, json.dumps(meta1))
        assert _get_crop(user_client, pid)["brightness"] == 30

        # Simulate re-enter: read current crop, modify, save
        current = _get_crop(user_client, pid)
        current["brightness"] = 60
        user_client.crop_photo(pid, json.dumps(current))
        assert _get_crop(user_client, pid)["brightness"] == 60

    def test_reopen_add_rotation_to_existing_crop(self, user_client):
        """Start with crop only, then add rotation on re-edit."""
        pid = _upload(user_client)

        meta1 = {"x": 0.1, "y": 0.2, "width": 0.8, "height": 0.6, "rotate": 0, "brightness": 0}
        user_client.crop_photo(pid, json.dumps(meta1))

        current = _get_crop(user_client, pid)
        current["rotate"] = 90
        user_client.crop_photo(pid, json.dumps(current))

        final_crop = _get_crop(user_client, pid)
        assert final_crop["rotate"] == 90
        assert math.isclose(final_crop["x"], 0.1)

    def test_reopen_remove_brightness_keep_rotation(self, user_client):
        """Start with rotation+brightness, then remove brightness on re-edit."""
        pid = _upload(user_client)

        meta1 = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 180, "brightness": 40}
        user_client.crop_photo(pid, json.dumps(meta1))

        current = _get_crop(user_client, pid)
        current["brightness"] = 0
        user_client.crop_photo(pid, json.dumps(current))

        final_crop = _get_crop(user_client, pid)
        assert final_crop["rotate"] == 180
        assert final_crop["brightness"] == 0


# ══════════════════════════════════════════════════════════════════════
# 7. Cross-User Isolation
# ══════════════════════════════════════════════════════════════════════

class TestCrossUserIsolation:
    """Verify that one user's edits don't affect another user's photos."""

    def test_other_user_cannot_edit_my_photo(self, user_client, second_user_client):
        """Second user should not be able to set crop on first user's photo."""
        pid = _upload(user_client)

        # Second user tries to set crop on first user's photo
        r = second_user_client.put(
            f"/api/photos/{pid}/crop",
            json_data={"crop_metadata": json.dumps({"rotate": 90})},
        )
        assert r.status_code == 404, "Should not be able to edit another user's photo"

    def test_other_user_cannot_duplicate_my_photo(self, user_client, second_user_client):
        """Second user should not be able to duplicate first user's photo."""
        pid = _upload(user_client)

        r = second_user_client.post(
            f"/api/photos/{pid}/duplicate",
            json_data={"crop_metadata": None},
        )
        assert r.status_code == 404


# ══════════════════════════════════════════════════════════════════════
# 8. Server Response Validation
# ══════════════════════════════════════════════════════════════════════

class TestServerResponseValidation:
    """Verify the server returns well-formed JSON responses."""

    def test_set_crop_response_format(self, user_client):
        pid = _upload(user_client)
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 30}
        result = user_client.crop_photo(pid, json.dumps(meta))
        assert "id" in result
        assert result["id"] == pid
        assert "crop_metadata" in result

    def test_duplicate_response_format(self, user_client):
        pid = _upload(user_client)
        meta = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": 20}
        dup = user_client.duplicate_photo(pid, json.dumps(meta))
        assert "id" in dup
        assert "source_photo_id" in dup
        assert "filename" in dup
        assert dup["source_photo_id"] == pid

    def test_clear_crop_response_format(self, user_client):
        pid = _upload(user_client)
        # Set then clear
        user_client.crop_photo(pid, json.dumps({"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0}))
        r = user_client.put(f"/api/photos/{pid}/crop", json_data={"crop_metadata": None})
        assert r.status_code == 200
        data = r.json()
        assert data["id"] == pid
        assert data["crop_metadata"] is None
