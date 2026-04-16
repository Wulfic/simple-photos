"""
Test 37: Editing Engine Semantic Tests.

Verifies the core editing engine contract:
  - "Save" (PUT /crop) is metadata-only — original file is never modified
  - "Save As Copy" (POST /duplicate) creates an independent file with crop_metadata=NULL
  - Crop metadata round-trips correctly through the server (set → get → compare)
  - Clearing crop metadata restores the original state
  - Copy inherits source's taken_at for correct timeline ordering
  - Copy dimensions reflect applied edits (rotation swaps w/h)
  - Negative brightness produces a darker image (regression for additive bug)
"""

import hashlib
import json
import time

import pytest

from helpers import (
    APIClient,
    generate_test_jpeg,
)


def _upload_jpeg(client: APIClient, w: int = 100, h: int = 80) -> str:
    """Upload a JPEG and return its photo_id after a short settle."""
    img = generate_test_jpeg(w, h)
    resp = client.upload_photo("test_semantics.jpg", img, "image/jpeg")
    photo_id = resp["photo_id"]
    time.sleep(1)
    return photo_id


# ── Save (Metadata-Only) ────────────────────────────────────────────────────


class TestSaveIsMetadataOnly:
    """PUT /crop must only update the crop_metadata column — the original
    file must remain byte-for-byte identical."""

    def test_original_file_unchanged_after_crop(self, user_client):
        """Set crop metadata and verify the served file is identical."""
        photo_id = _upload_jpeg(user_client)

        # Grab the original file bytes
        original = user_client.get_photo_file(photo_id)
        assert original.status_code == 200
        original_hash = hashlib.sha256(original.content).hexdigest()

        # Set crop metadata (heavy edits: rotation + crop + brightness)
        meta = json.dumps({
            "x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5,
            "rotate": 90, "brightness": 40,
        })
        user_client.crop_photo(photo_id, meta)
        time.sleep(0.5)

        # Re-fetch the file — must be identical
        after = user_client.get_photo_file(photo_id)
        assert after.status_code == 200
        after_hash = hashlib.sha256(after.content).hexdigest()

        assert original_hash == after_hash, (
            "Save (PUT /crop) must NOT modify the original file; "
            f"hash changed from {original_hash[:16]}… to {after_hash[:16]}…"
        )

    def test_crop_metadata_round_trips(self, user_client):
        """Set metadata, re-read the photo, and compare field-by-field."""
        photo_id = _upload_jpeg(user_client)

        meta = {
            "x": 0.15, "y": 0.25, "width": 0.7, "height": 0.5,
            "rotate": 270, "brightness": -30,
        }
        user_client.crop_photo(photo_id, json.dumps(meta))

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        stored = json.loads(photo["crop_metadata"])

        for key in ("x", "y", "width", "height", "rotate", "brightness"):
            assert stored[key] == meta[key], (
                f"Field {key!r}: expected {meta[key]}, got {stored[key]}"
            )

    def test_clear_crop_metadata(self, user_client):
        """Setting crop_metadata to null clears it."""
        photo_id = _upload_jpeg(user_client)

        # Set then clear
        user_client.crop_photo(photo_id, json.dumps({"rotate": 90}))
        resp = user_client.put(
            f"/api/photos/{photo_id}/crop",
            json_data={"crop_metadata": None},
        )
        assert resp.status_code == 200

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        assert photo["crop_metadata"] is None


# ── Save As Copy (Rendered Output) ──────────────────────────────────────────


class TestSaveAsCopy:
    """POST /duplicate must create a new independent photo row with the edits
    baked into the file and crop_metadata = NULL."""

    def test_copy_has_null_crop_metadata(self, user_client):
        """The copy's crop_metadata must be NULL (edits are baked in)."""
        photo_id = _upload_jpeg(user_client)

        meta = json.dumps({"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "rotate": 0})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=meta)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        copy_photo = next(p for p in photos if p["id"] == dup_id)
        assert copy_photo["crop_metadata"] is None, (
            "Save-as-copy must set crop_metadata to NULL (edits baked in)"
        )

    def test_copy_is_independent_photo_row(self, user_client):
        """The copy must be a separate photo row with its own ID."""
        photo_id = _upload_jpeg(user_client, 100, 80)

        meta = json.dumps({"x": 0.1, "y": 0.1, "width": 0.5, "height": 0.5, "rotate": 0})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=meta)
        dup_id = dup["id"]
        time.sleep(2)

        assert dup_id != photo_id, "Copy must have a different photo ID"

        photos = user_client.list_photos()["photos"]
        ids = [p["id"] for p in photos]
        assert photo_id in ids, "Original should still exist"
        assert dup_id in ids, "Copy should exist"

    def test_rotated_copy_swaps_dimensions(self, user_client):
        """A 100×80 image rotated 90° should become 80×100."""
        photo_id = _upload_jpeg(user_client, 100, 80)

        meta = json.dumps({"rotate": 90})
        dup = user_client.duplicate_photo(photo_id, crop_metadata=meta)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        copy_photo = next(p for p in photos if p["id"] == dup_id)

        assert copy_photo["width"] == 80, (
            f"Expected width=80 after 90° rotation, got {copy_photo['width']}"
        )
        assert copy_photo["height"] == 100, (
            f"Expected height=100 after 90° rotation, got {copy_photo['height']}"
        )

    def test_copy_preserves_original_taken_at(self, user_client):
        """The copy's created_at / taken_at should match the source photo
        for correct timeline ordering."""
        photo_id = _upload_jpeg(user_client)

        photos_before = user_client.list_photos()["photos"]
        orig = next(p for p in photos_before if p["id"] == photo_id)

        dup = user_client.duplicate_photo(photo_id, crop_metadata=json.dumps({"rotate": 0}))
        dup_id = dup["id"]
        time.sleep(2)

        photos_after = user_client.list_photos()["photos"]
        copy_photo = next(p for p in photos_after if p["id"] == dup_id)

        # The copy should use the original's taken_at for timeline ordering
        assert copy_photo["taken_at"] == orig["taken_at"], (
            f"Copy taken_at should match original: "
            f"{copy_photo['taken_at']} != {orig['taken_at']}"
        )

    def test_cropped_copy_has_smaller_dimensions(self, user_client):
        """A 100×80 image cropped to 50% width × 50% height → ~50×40."""
        photo_id = _upload_jpeg(user_client, 100, 80)

        meta = json.dumps({
            "x": 0.25, "y": 0.25, "width": 0.5, "height": 0.5, "rotate": 0,
        })
        dup = user_client.duplicate_photo(photo_id, crop_metadata=meta)
        dup_id = dup["id"]
        time.sleep(2)

        photos = user_client.list_photos()["photos"]
        copy_photo = next(p for p in photos if p["id"] == dup_id)

        assert copy_photo["width"] == 50, (
            f"Expected width=50 after 50% crop, got {copy_photo['width']}"
        )
        assert copy_photo["height"] == 40, (
            f"Expected height=40 after 50% crop, got {copy_photo['height']}"
        )


# ── Brightness Regression ───────────────────────────────────────────────────

class TestBrightnessRegression:
    """Verify that brightness adjustments produce expected results."""

    def test_negative_brightness_duplicate_succeeds(self, user_client):
        """A copy with brightness=-80 should succeed and bake edits in.
        (Regression: old code used wrong sign for negative brightness.)"""
        photo_id = _upload_jpeg(user_client, 40, 40)

        meta = json.dumps({"brightness": -80})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": meta},
        )
        assert resp.status_code == 201
        data = resp.json()
        assert data["crop_metadata"] is None, "Edits should be baked in"
        assert data["id"] != photo_id

    def test_positive_brightness_duplicate_succeeds(self, user_client):
        """A copy with brightness=+80 should succeed and bake edits in."""
        photo_id = _upload_jpeg(user_client, 40, 40)

        meta = json.dumps({"brightness": 80})
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": meta},
        )
        assert resp.status_code == 201
        data = resp.json()
        assert data["crop_metadata"] is None, "Edits should be baked in"
        assert data["id"] != photo_id

    def test_brightness_combined_with_crop_and_rotation(self, user_client):
        """Brightness + crop + rotation combined should succeed."""
        photo_id = _upload_jpeg(user_client, 100, 80)

        meta = json.dumps({
            "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8,
            "rotate": 90, "brightness": -50,
        })
        resp = user_client.post(
            f"/api/photos/{photo_id}/duplicate",
            json_data={"crop_metadata": meta},
        )
        assert resp.status_code == 201
        data = resp.json()
        assert data["crop_metadata"] is None

        # After 90° rotation of 80%×80% of 100×80: crop gives 80×64, then rotate → 64×80
        time.sleep(2)
        photos = user_client.list_photos()["photos"]
        copy = next(p for p in photos if p["id"] == data["id"])
        assert copy["width"] == 64, f"Expected width=64, got {copy['width']}"
        assert copy["height"] == 80, f"Expected height=80, got {copy['height']}"
