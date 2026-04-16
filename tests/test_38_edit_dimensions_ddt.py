"""
Test 38: Edit Dimensions — Data-Driven Tests.

Parametrized tests verifying that duplicate (Save As Copy) produces the
correct output dimensions for every combination of crop region × rotation.

Each test case specifies:
  - Source image size (width × height)
  - Crop region (x, y, width, height as 0–1 fractions)
  - Rotation (0, 90, 180, 270)
  - Expected output dimensions

The expected dimensions follow the formula:
  1. Cropped size: floor(source_w × crop_w), floor(source_h × crop_h)
  2. If rotation swaps (90° or 270°): swap width ↔ height
"""

import json
import time

import pytest

from helpers import APIClient, generate_test_jpeg


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient, w: int, h: int) -> str:
    img = generate_test_jpeg(w, h)
    resp = client.upload_photo("ddt_dims.jpg", img, "image/jpeg")
    return resp["photo_id"]


def _dup_dims(client: APIClient, photo_id: str, meta_dict: dict) -> tuple:
    """Duplicate with given metadata and return (width, height) of copy."""
    dup = client.duplicate_photo(photo_id, json.dumps(meta_dict))
    dup_id = dup["id"]
    time.sleep(2)
    photos = client.list_photos()["photos"]
    copy = next(p for p in photos if p["id"] == dup_id)
    return copy["width"], copy["height"]


# ══════════════════════════════════════════════════════════════════════
# DDT: Rotation-only (no crop)
# ══════════════════════════════════════════════════════════════════════

ROTATION_DIM_CASES = [
    #  src_w, src_h, rotate, exp_w, exp_h
    pytest.param(100, 80, 0,   100, 80,  id="100x80_rot0"),
    pytest.param(100, 80, 90,  80,  100, id="100x80_rot90"),
    pytest.param(100, 80, 180, 100, 80,  id="100x80_rot180"),
    pytest.param(100, 80, 270, 80,  100, id="100x80_rot270"),
    pytest.param(200, 200, 90, 200, 200, id="200x200_square_rot90"),
    pytest.param(60,  40,  90, 40,  60,  id="60x40_rot90"),
    pytest.param(60,  40,  270, 40, 60,  id="60x40_rot270"),
]


class TestRotationDimensions:
    @pytest.mark.parametrize("src_w,src_h,rotate,exp_w,exp_h", ROTATION_DIM_CASES)
    def test_rotation_dimensions(self, user_client, src_w, src_h, rotate, exp_w, exp_h):
        pid = _upload(user_client, src_w, src_h)
        w, h = _dup_dims(user_client, pid, {"rotate": rotate})
        assert (w, h) == (exp_w, exp_h), (
            f"{src_w}×{src_h} rot{rotate}° → expected {exp_w}×{exp_h}, got {w}×{h}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: Crop-only (no rotation)
# ══════════════════════════════════════════════════════════════════════

CROP_DIM_CASES = [
    #  src_w, src_h, crop_x, crop_y, crop_w, crop_h, exp_w, exp_h
    pytest.param(100, 80, 0.0, 0.0, 1.0, 1.0, 100, 80, id="full_frame"),
    pytest.param(100, 80, 0.0, 0.0, 0.5, 0.5, 50,  40, id="top_left_quarter"),
    pytest.param(100, 80, 0.25, 0.25, 0.5, 0.5, 50, 40, id="centered_half"),
    pytest.param(200, 100, 0.0, 0.0, 0.5, 1.0, 100, 100, id="left_half"),
    pytest.param(200, 100, 0.5, 0.0, 0.5, 1.0, 100, 100, id="right_half"),
    pytest.param(200, 160, 0.1, 0.1, 0.8, 0.8, 160, 128, id="80pct_inset"),
]


class TestCropDimensions:
    @pytest.mark.parametrize("src_w,src_h,cx,cy,cw,ch,exp_w,exp_h", CROP_DIM_CASES)
    def test_crop_dimensions(self, user_client, src_w, src_h, cx, cy, cw, ch, exp_w, exp_h):
        pid = _upload(user_client, src_w, src_h)
        meta = {"x": cx, "y": cy, "width": cw, "height": ch, "rotate": 0}
        w, h = _dup_dims(user_client, pid, meta)
        assert (w, h) == (exp_w, exp_h), (
            f"{src_w}×{src_h} crop({cx},{cy},{cw},{ch}) → expected {exp_w}×{exp_h}, got {w}×{h}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: Crop + Rotation combined
# ══════════════════════════════════════════════════════════════════════

CROP_ROTATE_CASES = [
    #  src_w, src_h, cx,  cy,  cw,  ch,  rot, exp_w, exp_h
    # 100×80 crop to 80×64 then rotate 90° → 64×80
    pytest.param(100, 80, 0.1, 0.1, 0.8, 0.8, 90, 64, 80, id="80pct_rot90"),
    # 100×80 crop to 50×40 then rotate 270° → 40×50
    pytest.param(100, 80, 0.0, 0.0, 0.5, 0.5, 270, 40, 50, id="quarter_rot270"),
    # 100×80 crop to 50×40 then rotate 180° → 50×40 (no swap)
    pytest.param(100, 80, 0.0, 0.0, 0.5, 0.5, 180, 50, 40, id="quarter_rot180"),
    # 200×100 left-half crop to 100×100 then rotate 90° → 100×100 (square stays)
    pytest.param(200, 100, 0.0, 0.0, 0.5, 1.0, 90, 100, 100, id="square_result_rot90"),
    # 200×160 inset 80% (160×128) then rotate 270° → 128×160
    pytest.param(200, 160, 0.1, 0.1, 0.8, 0.8, 270, 128, 160, id="inset_rot270"),
]


class TestCropRotateDimensions:
    @pytest.mark.parametrize("src_w,src_h,cx,cy,cw,ch,rot,exp_w,exp_h", CROP_ROTATE_CASES)
    def test_crop_rotate_dimensions(self, user_client, src_w, src_h, cx, cy, cw, ch, rot, exp_w, exp_h):
        pid = _upload(user_client, src_w, src_h)
        meta = {"x": cx, "y": cy, "width": cw, "height": ch, "rotate": rot}
        w, h = _dup_dims(user_client, pid, meta)
        assert (w, h) == (exp_w, exp_h), (
            f"{src_w}×{src_h} crop({cx},{cy},{cw},{ch}) rot{rot}° "
            f"→ expected {exp_w}×{exp_h}, got {w}×{h}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: Crop + Rotation + Brightness (shouldn't affect dimensions)
# ══════════════════════════════════════════════════════════════════════

BRIGHTNESS_DIM_CASES = [
    pytest.param(-100, id="brightness_-100"),
    pytest.param(-50,  id="brightness_-50"),
    pytest.param(0,    id="brightness_0"),
    pytest.param(50,   id="brightness_+50"),
    pytest.param(100,  id="brightness_+100"),
]


class TestBrightnessPreservesDimensions:
    """Brightness should never alter output dimensions."""

    @pytest.mark.parametrize("brightness", BRIGHTNESS_DIM_CASES)
    def test_brightness_does_not_change_dimensions(self, user_client, brightness):
        pid = _upload(user_client, 100, 80)
        meta = {
            "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8,
            "rotate": 90, "brightness": brightness,
        }
        w, h = _dup_dims(user_client, pid, meta)
        # 100×80 → crop 80×64 → rot90 → 64×80
        assert (w, h) == (64, 80), (
            f"Brightness {brightness} should not affect dimensions: "
            f"expected 64×80, got {w}×{h}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: No-metadata duplicate (plain copy)
# ══════════════════════════════════════════════════════════════════════

PLAIN_COPY_CASES = [
    pytest.param(100, 80,  id="100x80"),
    pytest.param(200, 200, id="200x200_square"),
    pytest.param(50,  120, id="50x120_portrait"),
]


class TestPlainCopyDimensions:
    """Duplicate without metadata should preserve exact dimensions."""

    @pytest.mark.parametrize("src_w,src_h", PLAIN_COPY_CASES)
    def test_plain_copy_preserves_dimensions(self, user_client, src_w, src_h):
        pid = _upload(user_client, src_w, src_h)
        dup = user_client.duplicate_photo(pid)
        dup_id = dup["id"]
        time.sleep(2)
        photos = user_client.list_photos()["photos"]
        copy = next(p for p in photos if p["id"] == dup_id)
        assert (copy["width"], copy["height"]) == (src_w, src_h), (
            f"Plain copy should preserve {src_w}×{src_h}, "
            f"got {copy['width']}×{copy['height']}"
        )


# ══════════════════════════════════════════════════════════════════════
# DDT: Edit-Copies CRUD (metadata-only versions)
# ══════════════════════════════════════════════════════════════════════

EDIT_COPY_META_CASES = [
    pytest.param(
        {"x": 0.1, "y": 0.2, "width": 0.6, "height": 0.5, "rotate": 90, "brightness": 30},
        id="crop_rotate_bright",
    ),
    pytest.param(
        {"rotate": 180},
        id="rotation_only",
    ),
    pytest.param(
        {"brightness": -50},
        id="brightness_only",
    ),
    pytest.param(
        {"x": 0.0, "y": 0.0, "width": 0.5, "height": 0.5},
        id="crop_only",
    ),
    pytest.param(
        {"trimStart": 1.0, "trimEnd": 5.0},
        id="trim_only",
    ),
]


class TestEditCopiesCRUD:
    """Edit copies are lightweight metadata-only snapshots (no file dup)."""

    @pytest.mark.parametrize("meta_dict", EDIT_COPY_META_CASES)
    def test_create_and_list_edit_copy(self, user_client, meta_dict):
        pid = _upload(user_client, 80, 60)
        meta_json = json.dumps(meta_dict)

        # Create via helper (uses edit_metadata field)
        copy = user_client.create_edit_copy(pid, edit_metadata=meta_json)
        copy_id = copy["id"]

        # List — should contain the copy we just made
        data = user_client.list_edit_copies(pid)
        ids = [c["id"] for c in data["copies"]]
        assert copy_id in ids

        # The stored metadata should parse back to our input
        stored = next(c for c in data["copies"] if c["id"] == copy_id)
        raw = stored["edit_metadata"]
        stored_meta = json.loads(raw) if isinstance(raw, str) else raw
        for key, val in meta_dict.items():
            assert stored_meta[key] == pytest.approx(val, abs=1e-9), (
                f"Field {key}: expected {val}, got {stored_meta[key]}"
            )

    @pytest.mark.parametrize("meta_dict", EDIT_COPY_META_CASES)
    def test_delete_edit_copy(self, user_client, meta_dict):
        pid = _upload(user_client, 80, 60)
        meta_json = json.dumps(meta_dict)

        copy = user_client.create_edit_copy(pid, edit_metadata=meta_json)
        copy_id = copy["id"]

        # Delete
        del_resp = user_client.delete_edit_copy(pid, copy_id)
        assert del_resp.status_code == 200

        # Verify gone
        data = user_client.list_edit_copies(pid)
        ids = [c["id"] for c in data["copies"]]
        assert copy_id not in ids
