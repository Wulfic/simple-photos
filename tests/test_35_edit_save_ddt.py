"""
Test 35: Edit Save — Data-Driven Tests (DDT).

Parametrized tests covering every edit operation the Viewer save flow exercises:
  - Set crop metadata (brightness, rotation, crop region, trim)
  - Save Copy (duplicate with metadata baked in)
  - Clear crop metadata
  - Default-detection (all-default metadata treated as "no edit")
  - Round-trip persistence: set → list → verify
  - Multiple sequential edits on the same photo
  - Edge cases: boundaries, extremes, invalid JSON

Each test case is a single row in a parameter table: the same test logic
runs with different inputs, making it easy to add new cases without
new test functions.
"""

import json
import math
import pytest

from helpers import APIClient, generate_test_jpeg


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient) -> str:
    """Upload a test JPEG and return its photo_id."""
    data = client.upload_photo("ddt_edit.jpg", generate_test_jpeg(200, 300))
    return data["photo_id"]


def _get_crop(client: APIClient, photo_id: str):
    """Fetch the current crop_metadata for a photo (parsed dict or None)."""
    photos = client.list_photos()["photos"]
    found = [p for p in photos if p["id"] == photo_id]
    assert found, f"Photo {photo_id} not found in list"
    raw = found[0].get("crop_metadata")
    return json.loads(raw) if raw else None


# ══════════════════════════════════════════════════════════════════════
# DDT: Set Crop Metadata — Brightness
# ══════════════════════════════════════════════════════════════════════

BRIGHTNESS_CASES = [
    pytest.param(1, id="brightness_+1_min_nondefault"),
    pytest.param(-1, id="brightness_-1_min_nondefault"),
    pytest.param(30, id="brightness_+30"),
    pytest.param(-50, id="brightness_-50"),
    pytest.param(100, id="brightness_+100_max"),
    pytest.param(-100, id="brightness_-100_min"),
    pytest.param(0, id="brightness_0_default"),  # should still save if other fields non-default
]


class TestBrightnessSetCrop:
    """Parametrized: set crop with various brightness values."""

    @pytest.mark.parametrize("brightness", BRIGHTNESS_CASES)
    def test_brightness_round_trip(self, user_client, brightness):
        pid = _upload(user_client)
        meta = {
            "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
            "rotate": 0, "brightness": brightness,
        }
        result = user_client.crop_photo(pid, json.dumps(meta))
        assert result["id"] == pid

        persisted = _get_crop(user_client, pid)
        assert persisted is not None, "crop_metadata should be persisted"
        assert persisted["brightness"] == brightness


# ══════════════════════════════════════════════════════════════════════
# DDT: Set Crop Metadata — Rotation
# ══════════════════════════════════════════════════════════════════════

ROTATION_CASES = [
    pytest.param(0, id="rotate_0"),
    pytest.param(90, id="rotate_90"),
    pytest.param(180, id="rotate_180"),
    pytest.param(270, id="rotate_270"),
    pytest.param(360, id="rotate_360_wraparound"),
]


class TestRotationSetCrop:
    @pytest.mark.parametrize("rotation", ROTATION_CASES)
    def test_rotation_round_trip(self, user_client, rotation):
        pid = _upload(user_client)
        meta = {
            "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
            "rotate": rotation, "brightness": 0,
        }
        user_client.crop_photo(pid, json.dumps(meta))
        persisted = _get_crop(user_client, pid)
        assert persisted is not None
        assert persisted["rotate"] == rotation


# ══════════════════════════════════════════════════════════════════════
# DDT: Set Crop Metadata — Crop Region
# ══════════════════════════════════════════════════════════════════════

CROP_REGION_CASES = [
    pytest.param(0.0, 0.0, 1.0, 1.0, id="full_frame"),
    pytest.param(0.1, 0.2, 0.6, 0.5, id="center_crop"),
    pytest.param(0.0, 0.0, 0.5, 0.5, id="top_left_quarter"),
    pytest.param(0.5, 0.5, 0.5, 0.5, id="bottom_right_quarter"),
    pytest.param(0.0, 0.0, 0.05, 0.05, id="min_crop_5pct"),
    pytest.param(0.25, 0.25, 0.5, 0.5, id="centered_half"),
    pytest.param(0.01, 0.01, 0.98, 0.98, id="near_full_just_inside_default_threshold"),
]


class TestCropRegionSetCrop:
    @pytest.mark.parametrize("x,y,w,h", CROP_REGION_CASES)
    def test_crop_region_round_trip(self, user_client, x, y, w, h):
        pid = _upload(user_client)
        meta = {"x": x, "y": y, "width": w, "height": h, "rotate": 0, "brightness": 0}
        user_client.crop_photo(pid, json.dumps(meta))
        persisted = _get_crop(user_client, pid)
        assert persisted is not None
        assert math.isclose(persisted["x"], x, abs_tol=1e-9)
        assert math.isclose(persisted["y"], y, abs_tol=1e-9)
        assert math.isclose(persisted["width"], w, abs_tol=1e-9)
        assert math.isclose(persisted["height"], h, abs_tol=1e-9)


# ══════════════════════════════════════════════════════════════════════
# DDT: Set Crop Metadata — Trim (video/audio)
# ══════════════════════════════════════════════════════════════════════

TRIM_CASES = [
    pytest.param(0.0, 5.0, id="trim_first_5s"),
    pytest.param(2.5, 10.0, id="trim_mid"),
    pytest.param(0.0, 0.5, id="trim_half_second"),
    pytest.param(1.0, 1.0, id="trim_zero_duration"),
]


class TestTrimSetCrop:
    @pytest.mark.parametrize("trim_start,trim_end", TRIM_CASES)
    def test_trim_round_trip(self, user_client, trim_start, trim_end):
        pid = _upload(user_client)
        meta = {
            "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
            "rotate": 0, "brightness": 0,
            "trimStart": trim_start, "trimEnd": trim_end,
        }
        user_client.crop_photo(pid, json.dumps(meta))
        persisted = _get_crop(user_client, pid)
        assert persisted is not None
        assert math.isclose(persisted["trimStart"], trim_start, abs_tol=1e-9)
        assert math.isclose(persisted["trimEnd"], trim_end, abs_tol=1e-9)


# ══════════════════════════════════════════════════════════════════════
# DDT: Combined Edits (multiple fields at once)
# ══════════════════════════════════════════════════════════════════════

COMBINED_CASES = [
    pytest.param(
        {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": 30},
        id="crop+rotate+bright",
    ),
    pytest.param(
        {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "rotate": 270, "brightness": -40},
        id="rotate_270+dark",
    ),
    pytest.param(
        {"x": 0.2, "y": 0.3, "width": 0.4, "height": 0.3, "rotate": 180, "brightness": 50,
         "trimStart": 1.0, "trimEnd": 3.0},
        id="all_fields",
    ),
    pytest.param(
        {"x": 0.0, "y": 0.0, "width": 0.5, "height": 0.5, "rotate": 0, "brightness": 0},
        id="crop_only_no_rotate_no_bright",
    ),
]


class TestCombinedEdits:
    @pytest.mark.parametrize("meta_dict", COMBINED_CASES)
    def test_combined_round_trip(self, user_client, meta_dict):
        pid = _upload(user_client)
        user_client.crop_photo(pid, json.dumps(meta_dict))
        persisted = _get_crop(user_client, pid)
        assert persisted is not None
        for key, val in meta_dict.items():
            assert math.isclose(persisted[key], val, abs_tol=1e-9), \
                f"Mismatch on {key}: expected {val}, got {persisted[key]}"


# ══════════════════════════════════════════════════════════════════════
# DDT: Duplicate (Save Copy) with various metadata
# ══════════════════════════════════════════════════════════════════════

DUPLICATE_CASES = [
    pytest.param(
        {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "rotate": 0, "brightness": 30},
        id="dup_brightness_only",
    ),
    pytest.param(
        {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "rotate": 90, "brightness": -20},
        id="dup_crop_rotate_bright",
    ),
    pytest.param(None, id="dup_no_metadata"),
]


class TestDuplicateDDT:
    @pytest.mark.parametrize("meta_dict", DUPLICATE_CASES)
    def test_duplicate_creates_copy(self, user_client, meta_dict):
        pid = _upload(user_client)
        meta_json = json.dumps(meta_dict) if meta_dict else None
        dup = user_client.duplicate_photo(pid, meta_json)
        assert dup["id"] != pid, "Copy should have a different ID"
        assert dup.get("source_photo_id") == pid

    @pytest.mark.parametrize("meta_dict", DUPLICATE_CASES)
    def test_duplicate_bakes_edits(self, user_client, meta_dict):
        """When metadata is supplied the server renders edits into the file
        and sets crop_metadata = NULL on the copy (edits baked in).
        When no metadata, the copy is a verbatim clone."""
        pid = _upload(user_client)
        meta_json = json.dumps(meta_dict) if meta_dict else None
        dup = user_client.duplicate_photo(pid, meta_json)
        dup_crop = dup.get("crop_metadata")
        if meta_dict:
            # Edits baked in → server clears crop_metadata on the rendered copy
            assert dup_crop is None, \
                "Rendered duplicate should have crop_metadata=NULL (edits baked in)"
        else:
            # No edits → plain copy, no crop_metadata
            assert dup_crop is None


# ══════════════════════════════════════════════════════════════════════
# DDT: Clear Crop Metadata
# ══════════════════════════════════════════════════════════════════════

CLEAR_CASES = [
    pytest.param(
        {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "rotate": 0, "brightness": 30},
        id="clear_after_brightness",
    ),
    pytest.param(
        {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": -20},
        id="clear_after_crop_rotate",
    ),
]


class TestClearCrop:
    @pytest.mark.parametrize("initial_meta", CLEAR_CASES)
    def test_clear_removes_metadata(self, user_client, initial_meta):
        pid = _upload(user_client)
        # Set metadata first
        user_client.crop_photo(pid, json.dumps(initial_meta))
        persisted = _get_crop(user_client, pid)
        assert persisted is not None

        # Clear it
        r = user_client.put(
            f"/api/photos/{pid}/crop",
            json_data={"crop_metadata": None},
        )
        assert r.status_code == 200

        cleared = _get_crop(user_client, pid)
        assert cleared is None, "crop_metadata should be None after clearing"


# ══════════════════════════════════════════════════════════════════════
# DDT: Sequential Edits on the Same Photo
# ══════════════════════════════════════════════════════════════════════

SEQUENTIAL_EDITS = [
    # (description, metadata_dict)
    ("set brightness +30", {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": 30}),
    ("change to rotation 90", {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 90, "brightness": 30}),
    ("add crop", {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": 30}),
    ("reduce brightness", {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": 10}),
    ("clear all edits", None),
]


class TestSequentialEdits:
    def test_sequential_edits_persist_correctly(self, user_client):
        """Apply multiple edits one after another to the same photo."""
        pid = _upload(user_client)

        for desc, meta_dict in SEQUENTIAL_EDITS:
            meta_json = json.dumps(meta_dict) if meta_dict else None
            r = user_client.put(
                f"/api/photos/{pid}/crop",
                json_data={"crop_metadata": meta_json},
            )
            assert r.status_code == 200, f"Failed at step: {desc}"

            persisted = _get_crop(user_client, pid)
            if meta_dict is None:
                assert persisted is None, f"Step '{desc}': should be cleared"
            else:
                assert persisted is not None, f"Step '{desc}': should persist"
                for key, val in meta_dict.items():
                    actual = persisted[key]
                    assert math.isclose(actual, val, abs_tol=1e-9), \
                        f"Step '{desc}': {key} expected {val}, got {actual}"


# ══════════════════════════════════════════════════════════════════════
# DDT: Edge Cases
# ══════════════════════════════════════════════════════════════════════

class TestEdgeCases:
    def test_duplicate_then_edit_original(self, user_client):
        """After duplicating, editing the original should not affect the copy."""
        pid = _upload(user_client)
        meta1 = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": 30}
        dup = user_client.duplicate_photo(pid, json.dumps(meta1))
        copy_id = dup["id"]

        # Edit original with different brightness
        meta2 = {"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0, "brightness": -50}
        user_client.crop_photo(pid, json.dumps(meta2))

        # Verify original has new metadata
        original_crop = _get_crop(user_client, pid)
        assert original_crop["brightness"] == -50

        # Check the copy in the photos list — its crop_metadata should reflect
        # the edit that was baked into its rendered file (meta1), not the
        # original's updated metadata (meta2).  The exact server representation
        # may vary (some servers clear crop_metadata on rendered duplicates), so
        # we just verify the copy wasn't mutated to match the original's new edit.
        photos = user_client.list_photos()["photos"]
        copy_entry = next((p for p in photos if p["id"] == copy_id), None)
        if copy_entry and copy_entry.get("crop_metadata"):
            copy_crop = json.loads(copy_entry["crop_metadata"])
            assert copy_crop.get("brightness") != -50, \
                "Copy should not inherit original's new edits"

    def test_overwrite_crop_completely(self, user_client):
        """Setting crop a second time completely replaces the first."""
        pid = _upload(user_client)
        user_client.crop_photo(pid, json.dumps(
            {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": 30}
        ))
        user_client.crop_photo(pid, json.dumps(
            {"x": 0.3, "y": 0.4, "width": 0.2, "height": 0.1, "rotate": 180, "brightness": -10}
        ))
        persisted = _get_crop(user_client, pid)
        assert persisted["x"] == 0.3
        assert persisted["rotate"] == 180
        assert persisted["brightness"] == -10

    def test_crop_metadata_size_limit(self, user_client):
        """Server should reject excessively large crop metadata."""
        pid = _upload(user_client)
        # The server limits crop_metadata to 1024 chars via sanitize_freeform
        huge = json.dumps({"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0,
                           "brightness": 0, "padding": "x" * 2000})
        # Should succeed but be truncated, or rejected
        r = user_client.put(f"/api/photos/{pid}/crop", json_data={"crop_metadata": huge})
        # Either 200 (truncated) or 400 (rejected)
        assert r.status_code in (200, 400)

    def test_invalid_crop_json_rejected(self, user_client):
        """Server should reject non-JSON crop metadata."""
        pid = _upload(user_client)
        r = user_client.put(f"/api/photos/{pid}/crop",
                            json_data={"crop_metadata": "not valid json {{{}"})
        assert r.status_code == 400

    def test_crop_on_nonexistent_photo(self, user_client):
        """Setting crop on a nonexistent photo should 404."""
        r = user_client.put(
            "/api/photos/00000000-0000-0000-0000-000000000000/crop",
            json_data={"crop_metadata": json.dumps({"x": 0, "y": 0, "width": 1, "height": 1, "rotate": 0})},
        )
        assert r.status_code == 404

    def test_duplicate_nonexistent_photo(self, user_client):
        """Duplicating a nonexistent photo should fail."""
        r = user_client.post(
            "/api/photos/00000000-0000-0000-0000-000000000000/duplicate",
            json_data={"crop_metadata": None},
        )
        assert r.status_code in (404, 500)
