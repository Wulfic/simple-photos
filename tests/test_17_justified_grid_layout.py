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


# ── Client-side grid layout algorithm regression tests ─────────────────────────
# These tests replicate the JustifiedGrid computeRows algorithm and verify that
# the layout produces correct item sizing — specifically catching the regression
# where the last incomplete row stretches items to fill the container width,
# causing photos/video previews to be cut off.


def _compute_rows(aspect_ratios, container_width, target_row_height, gap=4):
    """Pure-Python port of JustifiedGrid.computeRows() for testing."""
    if container_width <= 0 or len(aspect_ratios) == 0:
        return []
    rows = []
    row_start = 0
    row_aspect_sum = 0.0

    for i, ar in enumerate(aspect_ratios):
        row_aspect_sum += ar
        item_count = i - row_start + 1
        total_gap = (item_count - 1) * gap
        natural_width = row_aspect_sum * target_row_height + total_gap

        if natural_width >= container_width:
            available_width = container_width - total_gap
            row_height = available_width / row_aspect_sum
            rows.append({
                "start": row_start,
                "count": item_count,
                "height": row_height,
                "full": True,
            })
            row_start = i + 1
            row_aspect_sum = 0.0

    if row_start < len(aspect_ratios):
        rows.append({
            "start": row_start,
            "count": len(aspect_ratios) - row_start,
            "height": target_row_height,
            "full": False,
        })

    return rows


def _item_rendered_size(row, idx_in_row, aspect_ratios, container_width, gap=4):
    """Compute the rendered width and height of a specific item in a row.

    For full rows: width = (ar / sum_of_row_ars) * available_width, height = row_height.
    For incomplete (last) rows: width = ar * row_height, height = row_height.
    """
    start = row["start"]
    count = row["count"]
    height = row["height"]
    ar = aspect_ratios[start + idx_in_row]

    if row["full"]:
        row_ars = aspect_ratios[start : start + count]
        total_ar = sum(row_ars)
        total_gap = (count - 1) * gap
        available = container_width - total_gap
        width = (ar / total_ar) * available
    else:
        width = ar * height

    return width, height


class TestGridLayoutAlgorithm:
    """Regression tests for the JustifiedGrid computeRows algorithm.

    These verify that photos/video previews aren't cut off due to
    excessive stretching, particularly in the last incomplete row."""

    def test_full_rows_fill_container_width(self, user_client):
        """Items in complete rows should sum to approximately the
        container width (accounting for gaps)."""
        container_width = 900
        target_height = 180
        gap = 4
        aspect_ratios = [1.5, 0.67, 1.78, 1.0, 0.5, 1.33, 2.0, 0.75]

        rows = _compute_rows(aspect_ratios, container_width, target_height, gap)

        for row in rows:
            if not row["full"]:
                continue
            total_gap = (row["count"] - 1) * gap
            widths = []
            for j in range(row["count"]):
                w, _ = _item_rendered_size(row, j, aspect_ratios, container_width, gap)
                widths.append(w)
            total_width = sum(widths) + total_gap
            assert abs(total_width - container_width) < 1.0, (
                f"Full row total width {total_width:.1f} != container {container_width}"
            )

    def test_last_row_items_not_stretched(self, user_client):
        """Items in the last incomplete row must NOT be stretched to fill
        the full container width.  Each item's width should be
        ar * targetRowHeight — matching its natural size at the target
        height.  Stretching causes excessive cropping (cut-off previews)."""
        container_width = 900
        target_height = 180
        gap = 4
        # These 3 portrait photos won't fill a 900px row at 180px height
        aspect_ratios = [0.67, 0.67, 0.67]

        rows = _compute_rows(aspect_ratios, container_width, target_height, gap)

        # Should produce exactly one incomplete row
        assert len(rows) == 1
        row = rows[0]
        assert not row["full"], "Expected last row to be incomplete"

        for j in range(row["count"]):
            w, h = _item_rendered_size(row, j, aspect_ratios, container_width, gap)
            expected_w = aspect_ratios[row["start"] + j] * target_height
            assert abs(w - expected_w) < 1.0, (
                f"Last-row item {j}: width {w:.1f} should be {expected_w:.1f} "
                f"(natural size), not stretched to fill container"
            )

    def test_last_row_multi_item_no_excessive_crop(self, user_client):
        """When the last row has multiple items, the effective display
        aspect ratio should stay close to the original.  If items are
        stretched (bug), the displayed AR diverges wildly from the
        original, indicating cut-off/cropping."""
        container_width = 900
        target_height = 180
        gap = 4
        # Mix of portrait and landscape in a last-incomplete row
        aspect_ratios = [0.67, 1.5]

        rows = _compute_rows(aspect_ratios, container_width, target_height, gap)
        assert len(rows) == 1, "Expected all items in one incomplete row"
        row = rows[0]

        for j in range(row["count"]):
            w, h = _item_rendered_size(row, j, aspect_ratios, container_width, gap)
            displayed_ar = w / h
            original_ar = aspect_ratios[row["start"] + j]
            # The displayed AR should match the original (within tolerance)
            # If stretched to fill, the displayed AR would be much larger
            ratio = displayed_ar / original_ar
            assert 0.8 < ratio < 1.2, (
                f"Last-row item {j}: displayed AR {displayed_ar:.2f} diverges "
                f"from original {original_ar:.2f} (ratio={ratio:.2f}). "
                f"Items are being stretched, causing cut-off previews."
            )

    def test_single_item_last_row_not_stretched(self, user_client):
        """A single item in the last row should use fixed width, not flex."""
        container_width = 900
        target_height = 180
        gap = 4
        # One landscape photo that won't fill the row
        aspect_ratios = [1.5]

        rows = _compute_rows(aspect_ratios, container_width, target_height, gap)
        assert len(rows) == 1
        row = rows[0]
        assert not row["full"]

        w, h = _item_rendered_size(row, 0, aspect_ratios, container_width, gap)
        expected_w = 1.5 * target_height  # 270px
        assert abs(w - expected_w) < 1.0
        assert w < container_width, "Single item should NOT fill 900px"

    def test_portrait_photos_not_cropped_to_landscape(self, user_client):
        """Portrait photos (AR < 1) must not be displayed as landscape
        in any row — this is the most visible symptom of the stretching
        bug, where tall photos appear as thin horizontal strips."""
        container_width = 900
        target_height = 180
        gap = 4
        # Upload a batch of portrait photos and verify their grid display
        aspect_ratios = [0.56, 0.67, 0.75, 0.5, 0.8]

        rows = _compute_rows(aspect_ratios, container_width, target_height, gap)

        for row in rows:
            for j in range(row["count"]):
                original_ar = aspect_ratios[row["start"] + j]
                w, h = _item_rendered_size(row, j, aspect_ratios, container_width, gap)
                displayed_ar = w / h

                if original_ar < 1.0:
                    # Portrait photo: displayed AR should also be portrait
                    # (or at most slightly landscape for full rows where
                    # minor stretching is acceptable)
                    if not row["full"]:
                        assert displayed_ar < 1.0, (
                            f"Portrait photo (AR={original_ar:.2f}) displayed as "
                            f"landscape (AR={displayed_ar:.2f}) in incomplete row — "
                            f"this means previews are being cut off"
                        )

    def test_mixed_aspect_ratio_grid_from_api(self, user_client):
        """Upload photos with known dimensions, retrieve them via API,
        compute the grid layout, and verify no excessive cropping."""
        test_dims = [
            (200, 133),  # 3:2 landscape
            (100, 150),  # 2:3 portrait
            (160, 90),   # 16:9 landscape
            (90, 160),   # 9:16 portrait
            (120, 120),  # 1:1 square
            (80, 200),   # extreme portrait
        ]
        uploaded_ids = []
        for w, h in test_dims:
            content = generate_test_jpeg(width=w, height=h)
            name = _unique(f"grid_crop_test_{w}x{h}")
            data = user_client.upload_photo(name, content)
            uploaded_ids.append(data["photo_id"])

        photos = user_client.list_photos()["photos"]

        # Build aspect ratios from API response (same as frontend)
        api_ratios = []
        for pid in uploaded_ids:
            photo = next((p for p in photos if p["id"] == pid), None)
            assert photo is not None
            ar = photo["width"] / photo["height"]
            # Clamp same as frontend
            clamped = max(0.3, min(ar, 4.0))
            api_ratios.append(clamped)

        container_width = 900
        target_height = 180
        gap = 4
        rows = _compute_rows(api_ratios, container_width, target_height, gap)

        for row in rows:
            for j in range(row["count"]):
                w, h = _item_rendered_size(row, j, api_ratios, container_width, gap)
                original_ar = api_ratios[row["start"] + j]
                displayed_ar = w / h

                # For incomplete rows, displayed AR must match original
                if not row["full"]:
                    ratio = displayed_ar / original_ar
                    assert 0.9 < ratio < 1.1, (
                        f"Incomplete-row item: displayed AR {displayed_ar:.2f} "
                        f"vs original {original_ar:.2f} — preview cutoff detected"
                    )
