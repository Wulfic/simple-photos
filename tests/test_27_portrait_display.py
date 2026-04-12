"""
Test 27: Portrait Photo Display — E2E + regression tests for portrait photos
in thumbnails and the justified flex grid.

Current known issues:
  - Screenshots (natively portrait pixel layout, no EXIF) display correctly
    because width < height in the raw pixels.
  - Camera portrait photos (landscape raw pixels + EXIF orientation 5–8) may
    have wrong thumbnail orientation or wrong stored dimensions, causing them
    to render as tiny slivers or landscape tiles in the flex grid.
  - The JustifiedGrid clamps aspect ratios to [0.3, 4.0]. A 9:16 portrait
    (AR ≈ 0.5625) is valid, but a 2:3 AR of 0.667 means the tile gets
    ~2/3 the width of a square tile in the same row, making it visually small.

This test suite:
  1. Creates realistic test images in multiple portrait styles
  2. Uploads them and verifies stored dimensions + aspect ratios
  3. Downloads thumbnails and verifies they are portrait-oriented
  4. Simulates the JustifiedGrid row algorithm to detect display sizing issues
  5. Verifies the full encrypted-sync pipeline preserves portrait metadata
"""

import io
import json
import math
import random
import struct
import time
from typing import List, Tuple

import pytest
from PIL import Image

try:
    import piexif
except ImportError:
    piexif = None

from helpers import APIClient, generate_test_jpeg


# ── Test image generators ────────────────────────────────────────────


def _unique(prefix: str) -> str:
    return f"{prefix}_{int(time.time() * 1000)}_{random.randint(1000, 9999)}.jpg"


def make_native_portrait(width: int, height: int) -> bytes:
    """Create a natively portrait JPEG (no EXIF orientation needed).

    Like a screenshot — pixel dimensions are already portrait (width < height).
    """
    assert width < height, "Native portrait must have width < height"
    img = Image.new("RGB", (width, height))
    # Draw a gradient so the image isn't uniform (helps detect crop/rotation bugs)
    pixels = img.load()
    for y in range(height):
        for x in range(width):
            pixels[x, y] = (
                int(255 * x / max(width - 1, 1)),
                int(255 * y / max(height - 1, 1)),
                128,
            )
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=85)
    return buf.getvalue()


def make_exif_portrait(raw_w: int, raw_h: int, orientation: int = 6) -> bytes:
    """Create a JPEG with landscape raw pixels and EXIF orientation tag.

    Simulates a phone camera: sensor captures 4032×3024 (landscape) but
    EXIF orientation 6 means "rotate 90° CW" → display as 3024×4032 (portrait).

    Args:
        raw_w: Raw pixel width (should be > raw_h for typical phone photo)
        raw_h: Raw pixel height
        orientation: EXIF orientation tag (5–8 for portrait rotation)
    """
    if piexif is None:
        pytest.skip("piexif not installed — needed for EXIF portrait tests")

    img = Image.new("RGB", (raw_w, raw_h))
    pixels = img.load()
    # Asymmetric gradient to detect incorrect rotation
    for y in range(raw_h):
        for x in range(raw_w):
            pixels[x, y] = (
                int(200 * x / max(raw_w - 1, 1)),
                50,
                int(200 * y / max(raw_h - 1, 1)),
            )

    buf = io.BytesIO()
    exif_dict = {"0th": {piexif.ImageIFD.Orientation: orientation}}
    exif_bytes = piexif.dump(exif_dict)
    img.save(buf, format="JPEG", quality=85, exif=exif_bytes)
    return buf.getvalue()


def make_phone_portrait_realistic(aspect: str = "3:4") -> Tuple[bytes, int, int]:
    """Create a realistic phone-camera portrait photo.

    Returns (jpeg_bytes, expected_display_width, expected_display_height).

    Common phone aspect ratios for portrait:
      - 3:4  (standard phone camera, e.g. 3024×4032)
      - 9:16 (full-screen capture, e.g. 1080×1920)
      - 1:2  (tall/narrow, some phones)
    """
    if piexif is None:
        pytest.skip("piexif not installed")

    ratios = {
        "3:4": (400, 300),    # Raw: 400×300 landscape → display: 300×400 portrait
        "9:16": (320, 180),   # Raw: 320×180 → display: 180×320
        "1:2": (400, 200),    # Raw: 400×200 → display: 200×400
        "2:3": (300, 200),    # Raw: 300×200 → display: 200×300
    }
    raw_w, raw_h = ratios.get(aspect, (400, 300))
    content = make_exif_portrait(raw_w, raw_h, orientation=6)
    # After EXIF rotation 6 (90° CW): display dims are swapped
    return content, raw_h, raw_w


# ── JustifiedGrid simulation (Python port) ───────────────────────────

def compute_justified_rows(
    aspect_ratios: List[float],
    container_width: int,
    target_row_height: int,
    gap: int = 4,
) -> List[dict]:
    """Python port of the JustifiedGrid computeRows() algorithm.

    Returns a list of row dicts:
      { "start": int, "count": int, "height": float, "full": bool }
    """
    if container_width <= 0 or not aspect_ratios:
        return []

    rows = []
    row_start = 0
    row_ar_sum = 0.0

    for i, ar in enumerate(aspect_ratios):
        row_ar_sum += ar
        item_count = i - row_start + 1
        total_gap = (item_count - 1) * gap
        natural_width = row_ar_sum * target_row_height + total_gap

        if natural_width >= container_width:
            available = container_width - total_gap
            row_height = available / row_ar_sum
            rows.append({
                "start": row_start,
                "count": item_count,
                "height": row_height,
                "full": True,
            })
            row_start = i + 1
            row_ar_sum = 0.0

    # Last incomplete row
    if row_start < len(aspect_ratios):
        rows.append({
            "start": row_start,
            "count": len(aspect_ratios) - row_start,
            "height": float(target_row_height),
            "full": False,
        })

    return rows


def compute_tile_sizes(
    aspect_ratios: List[float],
    container_width: int,
    target_row_height: int,
    gap: int = 4,
) -> List[dict]:
    """Compute the pixel dimensions each tile gets in the justified grid.

    Returns a list of dicts: { "width": float, "height": float, "ar": float }
    """
    rows = compute_justified_rows(aspect_ratios, container_width, target_row_height, gap)
    tiles = []
    for row in rows:
        row_ars = aspect_ratios[row["start"]:row["start"] + row["count"]]
        for ar in row_ars:
            if row["full"]:
                # flex: ar 1 0% — width is proportional to AR within the row
                total_gap = (row["count"] - 1) * gap
                available = container_width - total_gap
                ar_sum = sum(row_ars)
                tile_w = (ar / ar_sum) * available
                tile_h = row["height"]
            else:
                # fixed width for incomplete row
                tile_w = ar * row["height"]
                tile_h = row["height"]
            tiles.append({"width": tile_w, "height": tile_h, "ar": ar})
    return tiles


# ── Fixtures ─────────────────────────────────────────────────────────


PORTRAIT_VARIATIONS = [
    # (label, raw_w, raw_h, exif_orientation, expected_display_w, expected_display_h)
    ("screenshot_portrait",     750, 1334,  None, 750, 1334),   # iPhone screenshot (native portrait)
    ("screenshot_tall",         1080, 2400, None, 1080, 2400),  # Android tall screenshot
    ("phone_camera_3_4",        400, 300,   6,    300, 400),    # 3:4 phone camera
    ("phone_camera_9_16",       320, 180,   6,    180, 320),    # 9:16 full-frame
    ("phone_camera_2_3",        300, 200,   6,    200, 300),    # 2:3 DSLR portrait
    ("phone_camera_1_2",        400, 200,   6,    200, 400),    # 1:2 narrow portrait
    ("camera_orient_5",         300, 200,   5,    200, 300),    # Orientation 5 (90°CW + flipH)
    ("camera_orient_7",         300, 200,   7,    200, 300),    # Orientation 7 (90°CCW + flipH)
    ("camera_orient_8",         300, 200,   8,    200, 300),    # Orientation 8 (90°CCW)
]

LANDSCAPE_REFERENCES = [
    # For mixed-grid testing: landscape and square photos alongside portraits
    ("landscape_4_3",     400, 300, None, 400, 300),
    ("landscape_16_9",    320, 180, None, 320, 180),
    ("square",            300, 300, None, 300, 300),
    ("landscape_3_2",     300, 200, None, 300, 200),
]


# ══════════════════════════════════════════════════════════════════════
#  E2E TESTS: Portrait Photo Upload & Dimension Verification
# ══════════════════════════════════════════════════════════════════════


class TestPortraitUploadDimensions:
    """Upload portrait photos in various styles and verify the server
    returns correct display dimensions."""

    @pytest.mark.parametrize(
        "label,raw_w,raw_h,exif_orient,exp_w,exp_h",
        PORTRAIT_VARIATIONS,
        ids=[v[0] for v in PORTRAIT_VARIATIONS],
    )
    def test_portrait_stored_dimensions(
        self, user_client, label, raw_w, raw_h, exif_orient, exp_w, exp_h,
    ):
        """Each portrait variation must have width < height in the API response."""
        if exif_orient is not None:
            content = make_exif_portrait(raw_w, raw_h, exif_orient)
        else:
            content = make_native_portrait(raw_w, raw_h) if raw_w < raw_h else generate_test_jpeg(raw_w, raw_h)

        filename = _unique(label)
        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]

        photos = user_client.list_photos()["photos"]
        photo = next((p for p in photos if p["id"] == photo_id), None)
        assert photo is not None, f"Photo {photo_id} ({label}) not in list"

        assert photo["width"] == exp_w, (
            f"[{label}] width: expected {exp_w}, got {photo['width']}. "
            f"Raw={raw_w}x{raw_h}, EXIF={exif_orient}"
        )
        assert photo["height"] == exp_h, (
            f"[{label}] height: expected {exp_h}, got {photo['height']}. "
            f"Raw={raw_w}x{raw_h}, EXIF={exif_orient}"
        )

        # All portrait variations must have AR < 1.0
        ar = photo["width"] / photo["height"]
        assert ar < 1.0, (
            f"[{label}] expected portrait AR < 1.0, got {ar:.3f}. "
            f"Dims={photo['width']}x{photo['height']}. "
            f"This means the grid will render this as landscape!"
        )

    def test_portrait_non_zero_dimensions(self, user_client):
        """Width and height must NEVER be zero — the grid breaks completely
        with division-by-zero in aspect ratio calculation."""
        content = make_exif_portrait(300, 200, 6)
        filename = _unique("nonzero_check")
        data = user_client.upload_photo(filename, content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])

        assert photo["width"] > 0, (
            f"Width is 0! Grid will break (division by zero in AR calc). "
            f"Stored dims: {photo['width']}x{photo['height']}"
        )
        assert photo["height"] > 0, (
            f"Height is 0! Grid will break (division by zero in AR calc). "
            f"Stored dims: {photo['width']}x{photo['height']}"
        )


# ══════════════════════════════════════════════════════════════════════
#  E2E TESTS: Thumbnail Portrait Orientation
# ══════════════════════════════════════════════════════════════════════


class TestPortraitThumbnailCorrectness:
    """Verify that server-generated thumbnails for portrait photos are
    actually portrait-oriented (taller than wide)."""

    @pytest.mark.parametrize(
        "label,raw_w,raw_h,exif_orient",
        [
            ("exif6_3_4",   400, 300, 6),   # Most common phone portrait
            ("exif6_9_16",  320, 180, 6),   # Full-frame portrait
            ("exif6_1_2",   400, 200, 6),   # Narrow portrait
            ("exif8_3_4",   300, 200, 8),   # Orientation 8 (90° CCW)
            ("native_3_4",  300, 400, None), # Native portrait (no EXIF)
        ],
        ids=["exif6_3_4", "exif6_9_16", "exif6_1_2", "exif8_3_4", "native_3_4"],
    )
    def test_thumbnail_is_portrait_oriented(
        self, user_client, label, raw_w, raw_h, exif_orient,
    ):
        """Download the thumbnail and verify its pixel dimensions are portrait."""
        if exif_orient is not None:
            content = make_exif_portrait(raw_w, raw_h, exif_orient)
        else:
            content = make_native_portrait(raw_w, raw_h)

        filename = _unique(f"thumb_{label}")
        data = user_client.upload_photo(filename, content)
        photo_id = data["photo_id"]

        # Wait for thumbnail generation
        thumb_data = None
        for attempt in range(10):
            time.sleep(1)
            r = user_client.get_photo_thumb(photo_id)
            if r.status_code == 200 and len(r.content) > 100:
                thumb_data = r.content
                break

        assert thumb_data is not None, (
            f"[{label}] Thumbnail not generated within 10s. "
            f"Status={r.status_code}, size={len(r.content) if r.status_code == 200 else 'N/A'}"
        )

        thumb_img = Image.open(io.BytesIO(thumb_data))
        thumb_w, thumb_h = thumb_img.size

        assert thumb_w < thumb_h, (
            f"[{label}] Thumbnail should be portrait (w < h), got {thumb_w}x{thumb_h}. "
            f"Raw={raw_w}x{raw_h}, EXIF={exif_orient}. "
            f"The thumbnail was generated WITHOUT applying EXIF rotation — "
            f"portrait photos will show sideways or as landscape tiles."
        )

    def test_thumbnail_preserves_portrait_aspect_ratio(self, user_client):
        """A 3:4 portrait photo's thumbnail should have AR close to 0.75, not 1.0 (square)."""
        content = make_exif_portrait(400, 300, 6)  # display: 300×400 → AR = 0.75
        filename = _unique("thumb_ar_check")
        data = user_client.upload_photo(filename, content)

        time.sleep(2)
        r = user_client.get_photo_thumb(data["photo_id"])
        assert r.status_code == 200 and len(r.content) > 100

        thumb = Image.open(io.BytesIO(r.content))
        tw, th = thumb.size
        thumb_ar = tw / th

        # Expected AR ≈ 0.75. Allow some tolerance (thumbnail scaling).
        assert 0.6 < thumb_ar < 0.9, (
            f"Thumbnail AR should be ~0.75 (portrait 3:4), got {thumb_ar:.3f} "
            f"({tw}x{th}). If AR ≈ 1.0, thumbnail is square-cropped. "
            f"If AR > 1.0, thumbnail is landscape (EXIF not applied)."
        )

    def test_screenshot_thumbnail_matches_source(self, user_client):
        """A native portrait screenshot's thumbnail should also be portrait,
        since there's no EXIF ambiguity."""
        content = make_native_portrait(375, 812)  # iPhone-like screenshot
        filename = _unique("screenshot_thumb")
        data = user_client.upload_photo(filename, content)

        time.sleep(2)
        r = user_client.get_photo_thumb(data["photo_id"])
        assert r.status_code == 200 and len(r.content) > 100

        thumb = Image.open(io.BytesIO(r.content))
        tw, th = thumb.size
        assert tw < th, (
            f"Screenshot thumbnail should be portrait, got {tw}x{th}. "
            f"Screenshot was 375×812 (natively portrait, no EXIF)."
        )


# ══════════════════════════════════════════════════════════════════════
#  E2E TESTS: Justified Grid Layout — Portrait Tile Sizing
# ══════════════════════════════════════════════════════════════════════


class TestPortraitGridLayout:
    """Simulate the JustifiedGrid algorithm with uploaded portrait photo
    dimensions to detect display sizing issues.

    The key concern: portrait tiles should be reasonably visible alongside
    landscape and square tiles. If they're too small, the user sees tiny
    slivers where portrait photos should be."""

    CONTAINER_WIDTH = 1200  # Typical desktop gallery width
    TARGET_ROW_HEIGHT_NORMAL = 180
    TARGET_ROW_HEIGHT_LARGE = 280
    GAP = 4

    def _upload_mixed_set(self, user_client) -> List[dict]:
        """Upload a mix of portrait, landscape, and square photos.
        Returns list of photo dicts from the API."""
        test_photos = [
            ("portrait_3_4",  make_exif_portrait(400, 300, 6)),
            ("portrait_9_16", make_exif_portrait(320, 180, 6)),
            ("portrait_2_3",  make_exif_portrait(300, 200, 6)),
            ("landscape_16_9", generate_test_jpeg(320, 180)),
            ("landscape_4_3",  generate_test_jpeg(400, 300)),
            ("square_1_1",     generate_test_jpeg(300, 300)),
            ("screenshot",     make_native_portrait(375, 667)),
            ("wide_pano",      generate_test_jpeg(500, 150)),
        ]

        photo_ids = []
        for label, content in test_photos:
            data = user_client.upload_photo(_unique(label), content)
            photo_ids.append(data["photo_id"])

        photos = user_client.list_photos()["photos"]
        return [next(p for p in photos if p["id"] == pid) for pid in photo_ids]

    def test_portrait_tiles_minimum_width(self, user_client):
        """Portrait tiles should have a minimum visible width in the grid.

        At normal row height (180px), a 3:4 portrait (AR=0.75) gets a tile
        135px wide. A 9:16 (AR=0.5625) gets 101px wide. These should not
        be unreasonably small.

        DETECTION: If AR is stored as landscape (>1.0) instead of portrait
        (<1.0), tile widths will be large in the wrong direction.
        """
        uploaded = self._upload_mixed_set(user_client)
        aspect_ratios = []
        for p in uploaded:
            assert p["width"] > 0 and p["height"] > 0, (
                f"Photo {p['filename']} has zero dimension: {p['width']}x{p['height']}"
            )
            ar = p["width"] / p["height"]
            # Same clamp as the frontend
            ar = max(0.3, min(ar, 4.0))
            aspect_ratios.append(ar)

        tiles = compute_tile_sizes(
            aspect_ratios,
            self.CONTAINER_WIDTH,
            self.TARGET_ROW_HEIGHT_NORMAL,
            self.GAP,
        )

        for i, (photo, tile) in enumerate(zip(uploaded, tiles)):
            # A tile should never be narrower than 40px — that's practically invisible
            assert tile["width"] >= 40, (
                f"Photo '{photo['filename']}' ({photo['width']}x{photo['height']}) "
                f"gets a tile only {tile['width']:.0f}px wide at "
                f"{self.TARGET_ROW_HEIGHT_NORMAL}px row height. "
                f"AR={photo['width']/photo['height']:.3f}. This is too small to see."
            )

    def test_portrait_tiles_reasonable_area(self, user_client):
        """Portrait tiles should have visible area (not just thin slivers).

        The minimum area should be at least 15% of what a square tile
        of the same row height gets.
        """
        uploaded = self._upload_mixed_set(user_client)
        aspect_ratios = []
        for p in uploaded:
            ar = max(0.3, min(p["width"] / p["height"], 4.0))
            aspect_ratios.append(ar)

        tiles = compute_tile_sizes(
            aspect_ratios,
            self.CONTAINER_WIDTH,
            self.TARGET_ROW_HEIGHT_NORMAL,
            self.GAP,
        )

        square_area = self.TARGET_ROW_HEIGHT_NORMAL ** 2  # Reference area

        for i, (photo, tile) in enumerate(zip(uploaded, tiles)):
            tile_area = tile["width"] * tile["height"]
            area_ratio = tile_area / square_area

            # Portrait tiles should have at least 15% the area of a square
            if photo["width"] < photo["height"]:  # Portrait photo
                assert area_ratio >= 0.15, (
                    f"Portrait '{photo['filename']}' ({photo['width']}x{photo['height']}) "
                    f"tile area is only {area_ratio:.1%} of a square tile. "
                    f"Tile: {tile['width']:.0f}x{tile['height']:.0f}px. "
                    f"This is too small for a portrait photo!"
                )

    def test_portrait_ar_stored_correctly_for_grid(self, user_client):
        """The aspect ratio from the API must be < 1.0 for all portrait uploads.

        This is the most critical check: if AR > 1.0, the grid treats the
        photo as landscape and the tile shape is WRONG."""
        test_cases = [
            ("phone_3_4",    make_exif_portrait(400, 300, 6),  0.75),   # 300/400
            ("phone_9_16",   make_exif_portrait(320, 180, 6),  0.5625), # 180/320
            ("screenshot",   make_native_portrait(375, 667),   0.5622), # 375/667
            ("phone_orient8", make_exif_portrait(300, 200, 8), 0.6667), # 200/300
        ]

        for label, content, expected_ar in test_cases:
            data = user_client.upload_photo(_unique(label), content)
            photos = user_client.list_photos()["photos"]
            photo = next(p for p in photos if p["id"] == data["photo_id"])

            actual_ar = photo["width"] / photo["height"]
            assert actual_ar < 1.0, (
                f"[{label}] AR should be < 1.0 (portrait), got {actual_ar:.4f}. "
                f"Stored: {photo['width']}x{photo['height']}. "
                f"The grid will render this as a landscape tile!"
            )
            assert abs(actual_ar - expected_ar) < 0.01, (
                f"[{label}] AR should be ~{expected_ar:.4f}, got {actual_ar:.4f}. "
                f"Stored: {photo['width']}x{photo['height']}"
            )

    def test_mixed_grid_row_heights(self, user_client):
        """In a mixed grid (portrait + landscape + square), row heights
        should be within a reasonable range. Extreme row height shrinkage
        would indicate that portrait ARs are being treated incorrectly."""
        uploaded = self._upload_mixed_set(user_client)
        aspect_ratios = [
            max(0.3, min(p["width"] / p["height"], 4.0))
            for p in uploaded
        ]

        rows = compute_justified_rows(
            aspect_ratios,
            self.CONTAINER_WIDTH,
            self.TARGET_ROW_HEIGHT_NORMAL,
            self.GAP,
        )

        for i, row in enumerate(rows):
            if row["full"]:
                # Full rows may be shorter than target, but not absurdly so
                # (less than 40% of target means something is wrong)
                min_acceptable = self.TARGET_ROW_HEIGHT_NORMAL * 0.4
                assert row["height"] >= min_acceptable, (
                    f"Row {i} height {row['height']:.0f}px is too small "
                    f"(target={self.TARGET_ROW_HEIGHT_NORMAL}px, "
                    f"minimum acceptable={min_acceptable:.0f}px). "
                    f"Contains items {row['start']}–{row['start']+row['count']-1}"
                )


# ══════════════════════════════════════════════════════════════════════
#  E2E TESTS: Encrypted Sync Pipeline — Portrait Dimensions
# ══════════════════════════════════════════════════════════════════════


class TestPortraitEncryptedSync:
    """Verify portrait dimensions survive the full encrypted-sync pipeline:
    upload → server storage → encrypted-sync → web client grid."""

    def test_portrait_dims_in_encrypted_sync(self, user_client):
        """Portrait EXIF-6 photo dimensions must be correct in encrypted-sync."""
        content = make_exif_portrait(400, 300, 6)
        data = user_client.upload_photo(_unique("enc_portrait"), content)
        photo_id = data["photo_id"]

        time.sleep(2)
        sync = user_client.encrypted_sync()
        photo = next(
            (p for p in sync.get("photos", []) if p["id"] == photo_id),
            None,
        )
        assert photo is not None, f"Photo {photo_id} not in encrypted-sync"

        assert photo["width"] == 300 and photo["height"] == 400, (
            f"encrypted-sync should return 300x400 (portrait), "
            f"got {photo['width']}x{photo['height']}"
        )

    def test_portrait_screenshot_in_encrypted_sync(self, user_client):
        """Native portrait screenshot dims must survive encrypted-sync."""
        content = make_native_portrait(375, 812)
        data = user_client.upload_photo(_unique("enc_screenshot"), content)
        photo_id = data["photo_id"]

        time.sleep(2)
        sync = user_client.encrypted_sync()
        photo = next(
            (p for p in sync.get("photos", []) if p["id"] == photo_id),
            None,
        )
        assert photo is not None

        assert photo["width"] == 375 and photo["height"] == 812, (
            f"encrypted-sync should return 375x812, "
            f"got {photo['width']}x{photo['height']}"
        )

    def test_all_portrait_types_in_encrypted_sync(self, user_client):
        """Upload multiple portrait types and verify all have correct
        dimensions via encrypted-sync."""
        uploads = []

        # EXIF portrait variations
        for orient in [5, 6, 7, 8]:
            content = make_exif_portrait(300, 200, orient)
            data = user_client.upload_photo(_unique(f"enc_orient_{orient}"), content)
            uploads.append((data["photo_id"], 200, 300, f"orient_{orient}"))

        # Native portrait
        content = make_native_portrait(200, 300)
        data = user_client.upload_photo(_unique("enc_native"), content)
        uploads.append((data["photo_id"], 200, 300, "native"))

        time.sleep(3)
        sync = user_client.encrypted_sync()
        sync_map = {p["id"]: p for p in sync.get("photos", [])}

        for photo_id, exp_w, exp_h, label in uploads:
            photo = sync_map.get(photo_id)
            assert photo is not None, f"[{label}] missing from encrypted-sync"
            assert photo["width"] == exp_w and photo["height"] == exp_h, (
                f"[{label}] expected {exp_w}x{exp_h}, "
                f"got {photo['width']}x{photo['height']}"
            )


# ══════════════════════════════════════════════════════════════════════
#  REGRESSION TESTS: Known Portrait Display Bugs
# ══════════════════════════════════════════════════════════════════════


class TestPortraitDisplayRegressions:
    """Regression tests for known portrait display bugs.

    These tests are designed to FAIL when the bugs are present and
    PASS once fixed. Run these to verify the current state of portrait
    rendering."""

    def test_regression_camera_portrait_not_landscape(self, user_client):
        """BUG: Phone camera portraits (EXIF 6) stored with landscape
        dimensions, causing grid to render them as landscape tiles.

        Expected after fix: width < height (portrait)."""
        content = make_exif_portrait(400, 300, 6)
        data = user_client.upload_photo(_unique("regression_landscape"), content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])

        # THE BUG: If width > height, the EXIF was not applied
        assert photo["width"] < photo["height"], (
            f"REGRESSION: Camera portrait stored as {photo['width']}x{photo['height']} "
            f"(landscape). EXIF orientation 6 not applied during upload. "
            f"Grid will render with AR={photo['width']/photo['height']:.2f} (landscape)."
        )

    def test_regression_thumbnail_not_sideways(self, user_client):
        """BUG: Portrait thumbnail generated from landscape raw pixels
        without EXIF rotation, so portrait photos appear sideways.

        Expected after fix: thumbnail is portrait (w < h)."""
        content = make_exif_portrait(400, 300, 6)
        data = user_client.upload_photo(_unique("regression_sideways"), content)

        time.sleep(2)
        r = user_client.get_photo_thumb(data["photo_id"])
        if r.status_code != 200 or len(r.content) < 100:
            pytest.skip("Thumbnail not ready")

        thumb = Image.open(io.BytesIO(r.content))
        tw, th = thumb.size
        assert tw < th, (
            f"REGRESSION: Portrait thumbnail is {tw}x{th} (landscape/square). "
            f"EXIF rotation not applied during thumbnail generation."
        )

    def test_regression_thumbnail_not_square_crop(self, user_client):
        """BUG: Thumbnail generated as 256×256 square crop (old migration code)
        instead of aspect-preserving 512px thumbnail.

        Expected after fix: thumbnail preserves portrait aspect ratio."""
        # Use extreme 1:2 portrait to make square crop obvious
        content = make_exif_portrait(400, 200, 6)  # display: 200×400
        data = user_client.upload_photo(_unique("regression_square"), content)

        time.sleep(2)
        r = user_client.get_photo_thumb(data["photo_id"])
        if r.status_code != 200 or len(r.content) < 100:
            pytest.skip("Thumbnail not ready")

        thumb = Image.open(io.BytesIO(r.content))
        tw, th = thumb.size
        thumb_ar = tw / th

        assert thumb_ar < 0.7, (
            f"REGRESSION: Thumbnail AR is {thumb_ar:.3f} ({tw}x{th}), expected ~0.5. "
            f"If AR ≈ 1.0, thumbnail was square-cropped (old migration bug)."
        )

    def test_regression_portrait_too_small_in_grid(self, user_client):
        """BUG: Portrait photos render as tiny tiles in the justified grid
        because their aspect ratio is very low (< 0.5) and the grid allocates
        proportionally less width.

        This test uploads a mix and verifies portrait tiles aren't too small.
        Currently expected to detect the sizing issue."""
        photos = []
        for label, content in [
            ("portrait_3_4",  make_exif_portrait(400, 300, 6)),
            ("landscape_4_3", generate_test_jpeg(400, 300)),
            ("portrait_9_16", make_exif_portrait(320, 180, 6)),
            ("landscape_16_9", generate_test_jpeg(320, 180)),
            ("square",        generate_test_jpeg(300, 300)),
        ]:
            data = user_client.upload_photo(_unique(label), content)
            photos_resp = user_client.list_photos()["photos"]
            p = next(x for x in photos_resp if x["id"] == data["photo_id"])
            photos.append(p)

        # Simulate grid layout
        aspect_ratios = [max(0.3, min(p["width"] / p["height"], 4.0)) for p in photos]
        tiles = compute_tile_sizes(aspect_ratios, 1200, 180, 4)

        for photo, tile in zip(photos, tiles):
            is_portrait = photo["width"] < photo["height"]
            if is_portrait:
                # Portrait tiles should be at least 60px wide
                assert tile["width"] >= 60, (
                    f"PORTRAIT TOO SMALL: '{photo['filename']}' "
                    f"({photo['width']}x{photo['height']}, AR={photo['width']/photo['height']:.3f}) "
                    f"gets only {tile['width']:.0f}px wide in grid. "
                    f"Should be at least 60px for visibility."
                )

    def test_regression_dimensions_persist_across_syncs(self, user_client):
        """BUG: Portrait dimensions got overwritten to landscape on subsequent
        sync cycles (IDB dimension mismatch).

        Upload portrait, then do 3 sync cycles and verify dimensions stay portrait."""
        content = make_exif_portrait(300, 200, 6)
        data = user_client.upload_photo(_unique("regression_sync_persist"), content)
        photo_id = data["photo_id"]

        for cycle in range(3):
            time.sleep(1)
            sync = user_client.encrypted_sync()
            photo = next(
                (p for p in sync.get("photos", []) if p["id"] == photo_id),
                None,
            )
            assert photo is not None, f"Cycle {cycle}: photo missing from sync"
            assert photo["width"] == 200, (
                f"Cycle {cycle}: width should be 200 (portrait), "
                f"got {photo['width']}"
            )
            assert photo["height"] == 300, (
                f"Cycle {cycle}: height should be 300 (portrait), "
                f"got {photo['height']}"
            )


# ══════════════════════════════════════════════════════════════════════
#  UNIT TESTS: JustifiedGrid Algorithm with Portrait Aspect Ratios
# ══════════════════════════════════════════════════════════════════════


class TestJustifiedGridAlgorithm:
    """Pure Python tests for the JustifiedGrid row algorithm behavior
    with portrait aspect ratios. No server needed — tests the display
    logic directly."""

    def test_portrait_tile_gets_adequate_width(self):
        """A 3:4 portrait (AR=0.75) at 180px row height should get ~135px wide.
        This is small but acceptable."""
        tiles = compute_tile_sizes([0.75], 1200, 180, 4)
        # Last row (incomplete) → fixed width = 0.75 * 180 = 135
        assert tiles[0]["width"] == pytest.approx(135, abs=1)

    def test_extreme_portrait_clamped(self):
        """Very narrow portraits (AR < 0.3) are clamped to 0.3 by the frontend.
        At 180px row height → 54px wide, which is the minimum grid allows."""
        # 1:5 portrait → AR = 0.2 → clamped to 0.3
        ar = max(0.3, min(0.2, 4.0))
        tiles = compute_tile_sizes([ar], 1200, 180, 4)
        assert tiles[0]["width"] == pytest.approx(54, abs=1)

    def test_portrait_vs_landscape_in_same_row(self):
        """In a full row with mixed aspect ratios, portrait tiles get
        proportionally less width. Verify the ratio is reasonable."""
        # 3:4 portrait (0.75) + 4:3 landscape (1.333) + square (1.0)
        ars = [0.75, 1.333, 1.0]
        tiles = compute_tile_sizes(ars, 1200, 180, 4)

        portrait_w = tiles[0]["width"]
        landscape_w = tiles[1]["width"]
        square_w = tiles[2]["width"]

        # Portrait should be roughly 0.75/1.333 = 56% of landscape width
        ratio = portrait_w / landscape_w
        assert 0.45 < ratio < 0.65, (
            f"Portrait/landscape width ratio should be ~0.56, got {ratio:.3f}. "
            f"Portrait={portrait_w:.0f}px, Landscape={landscape_w:.0f}px"
        )

        # All tiles should be visible
        assert portrait_w >= 50, f"Portrait tile only {portrait_w:.0f}px wide"
        assert landscape_w >= 50, f"Landscape tile only {landscape_w:.0f}px wide"

    def test_all_portrait_row(self):
        """A row of only portrait photos should have taller tiles than the
        target row height (row fills width with narrow tiles → height increases)."""
        # 3 portrait 3:4 photos
        ars = [0.75, 0.75, 0.75]
        rows = compute_justified_rows(ars, 1200, 180, 4)

        # With 3 portraits at AR=0.75: natural width = 3 * 0.75 * 180 + 8 = 413px
        # Container is 1200px, so row won't be full → uses target height
        assert rows[0]["height"] == 180

        # Need more portraits to fill a row
        ars = [0.75] * 8  # 8 portraits: natural = 8 * 0.75 * 180 + 28 = 1108px
        rows = compute_justified_rows(ars, 1200, 180, 4)
        # Still might not fill... need 9: 9 * 0.75 * 180 + 32 = 1247px > 1200
        ars = [0.75] * 9
        rows = compute_justified_rows(ars, 1200, 180, 4)
        assert len(rows) >= 1
        if rows[0]["full"]:
            # Row height should be reasonable
            assert rows[0]["height"] > 100, (
                f"All-portrait row height {rows[0]['height']:.0f}px is too small"
            )

    def test_9_16_portrait_sizing(self):
        """9:16 portraits (AR ≈ 0.5625) are narrower than 3:4 — verify
        they still get reasonable tile dimensions."""
        # 9:16 portrait in last row
        tiles = compute_tile_sizes([0.5625], 1200, 180, 4)
        # Width = 0.5625 * 180 ≈ 101px
        assert tiles[0]["width"] > 90, (
            f"9:16 portrait only {tiles[0]['width']:.0f}px wide"
        )

    def test_portrait_with_large_size_setting(self):
        """At 'large' thumbnail size (280px), portrait tiles should be bigger."""
        # 3:4 portrait at 280px height → width = 0.75 * 280 = 210px
        tiles = compute_tile_sizes([0.75], 1200, 280, 4)
        assert tiles[0]["width"] == pytest.approx(210, abs=1)

        # 9:16 portrait at 280px → width = 0.5625 * 280 ≈ 157px
        tiles = compute_tile_sizes([0.5625], 1200, 280, 4)
        assert tiles[0]["width"] == pytest.approx(157.5, abs=1)


# ══════════════════════════════════════════════════════════════════════
#  BATCH DIMENSION UPDATE TESTS
# ══════════════════════════════════════════════════════════════════════


class TestBatchDimensionUpdate:
    """Verify the PATCH /api/photos/dimensions endpoint which the Android
    repairExifDimensions() function uses to fix double-swapped dimensions."""

    def test_batch_update_by_photo_id(self, user_client):
        """Correcting dimensions by photo_id should be reflected in list_photos."""
        content = make_native_portrait(300, 400)
        data = user_client.upload_photo(_unique("batchdim_pid"), content)
        photo_id = data["photo_id"]

        # Verify initial dims
        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        assert photo["width"] == 300 and photo["height"] == 400

        # Simulate Android repair: send corrected dimensions via batch update
        r = user_client.patch("/api/photos/dimensions", json_data={
            "updates": [{"photo_id": photo_id, "width": 400, "height": 300}]
        })
        assert r.status_code == 200
        assert r.json()["updated"] == 1

        # Verify update took effect
        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        assert photo["width"] == 400
        assert photo["height"] == 300

    def test_batch_update_rejects_zero_dimensions(self, user_client):
        """Zero or negative dimensions should be silently skipped."""
        content = make_native_portrait(300, 400)
        data = user_client.upload_photo(_unique("batchdim_zero"), content)
        photo_id = data["photo_id"]

        r = user_client.patch("/api/photos/dimensions", json_data={
            "updates": [{"photo_id": photo_id, "width": 0, "height": 400}]
        })
        assert r.status_code == 200
        assert r.json()["updated"] == 0  # skipped

    def test_double_swap_simulation(self, user_client):
        """Simulate the Android double-swap bug: upload a portrait EXIF photo,
        then call batch_update with the WRONG (double-swapped) dimensions,
        then call it again with the correct dimensions.

        This mimics what happens when:
        1. scanImages() swaps dims to portrait ✓
        2. BackupWorker swaps again to landscape ✗ (the bug)
        3. repairExifDimensions() swaps BACK to portrait ✓ (the repair)"""
        # Upload: EXIF orient=6 means raw 400x300 → display 300x400
        content = make_exif_portrait(400, 300, 6)
        data = user_client.upload_photo(_unique("doubswap"), content)
        photo_id = data["photo_id"]

        # Server should store correct portrait dimensions
        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        assert photo["width"] == 300 and photo["height"] == 400, \
            "Server should apply EXIF orientation on upload"

        # Simulate double-swap: Android repair sets WRONG landscape dims
        r = user_client.patch("/api/photos/dimensions", json_data={
            "updates": [{"photo_id": photo_id, "width": 400, "height": 300}]
        })
        assert r.status_code == 200

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        # Now dims are wrong (landscape for a portrait photo)
        ar = photo["width"] / photo["height"]
        assert ar > 1.0, "Double-swap produced landscape dimensions"

        # Simulate correct repair: set back to portrait
        r = user_client.patch("/api/photos/dimensions", json_data={
            "updates": [{"photo_id": photo_id, "width": 300, "height": 400}]
        })
        assert r.status_code == 200

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == photo_id)
        assert photo["width"] == 300 and photo["height"] == 400, \
            "Repair should restore correct portrait dimensions"


# ══════════════════════════════════════════════════════════════════════
#  DETECTION TESTS: Designed to catch current failures
# ══════════════════════════════════════════════════════════════════════


class TestCurrentStateDetection:
    """These tests probe the CURRENT state of the system to detect
    specific known failure modes. They're designed to fail when bugs
    are present, providing clear diagnostic output."""

    def test_detect_landscape_stored_for_portrait_upload(self, user_client):
        """DETECTION: Upload a phone-camera portrait and check if the server
        returns landscape dimensions (the most common portrait bug).

        If this test FAILS, the portrait → landscape dimension bug is present."""
        raw_w, raw_h = 400, 300  # landscape raw pixels
        content = make_exif_portrait(raw_w, raw_h, 6)
        data = user_client.upload_photo(_unique("detect_dims"), content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])

        # If landscape dims were stored (400×300 instead of 300×400)
        if photo["width"] == raw_w and photo["height"] == raw_h:
            pytest.fail(
                f"BUG DETECTED: Server stored raw pixel dimensions {raw_w}x{raw_h} "
                f"instead of display dimensions {raw_h}x{raw_w}. "
                f"EXIF orientation 6 was NOT applied during upload. "
                f"The justified grid will render this as landscape (AR={raw_w/raw_h:.2f}) "
                f"instead of portrait (AR={raw_h/raw_w:.2f})."
            )

        assert photo["width"] == raw_h and photo["height"] == raw_w

    def test_detect_thumbnail_wrong_orientation(self, user_client):
        """DETECTION: Check if portrait thumbnail pixels are landscape.

        If this test FAILS, thumbnails are generated without EXIF rotation."""
        content = make_exif_portrait(400, 300, 6)
        data = user_client.upload_photo(_unique("detect_thumb"), content)

        time.sleep(2)
        r = user_client.get_photo_thumb(data["photo_id"])
        if r.status_code != 200 or len(r.content) < 100:
            pytest.skip("Thumbnail not generated yet")

        thumb = Image.open(io.BytesIO(r.content))
        tw, th = thumb.size

        if tw >= th:
            pytest.fail(
                f"BUG DETECTED: Portrait thumbnail is {tw}x{th} (landscape/square). "
                f"EXIF rotation not applied during thumbnail generation. "
                f"Portrait photos will appear sideways in the gallery."
            )

    def test_detect_zero_dimensions(self, user_client):
        """DETECTION: Check for zero width/height which breaks the grid entirely."""
        content = make_exif_portrait(300, 200, 6)
        data = user_client.upload_photo(_unique("detect_zero"), content)

        photos = user_client.list_photos()["photos"]
        photo = next(p for p in photos if p["id"] == data["photo_id"])

        if photo["width"] == 0 or photo["height"] == 0:
            pytest.fail(
                f"BUG DETECTED: Dimensions are {photo['width']}x{photo['height']}. "
                f"Zero dimensions cause division-by-zero in the grid layout. "
                f"All grid rendering will be broken."
            )
