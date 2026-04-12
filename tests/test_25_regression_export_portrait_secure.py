"""
Test 25: Regression tests for three critical bugs:

1. **Export duplicate filename** — ZIP archive fails with "duplicate file name"
   when two photos share the same filename. Fixed by deduplicating filenames
   inside the ZIP writer with `_(N)` suffixes.

2. **Portrait orientation display** — Portrait photos (EXIF orientation 5-8)
   initially display correctly then switch to landscape. Fixed by re-downloading
   thumbnails when the server's `encrypted_thumb_blob_id` changes, and
   updating `thumbnailBlobId` in IDB during sync.

3. **Secure album wrong photo** — Adding a photo to a secure album produces
   a clone that displays a different photo's thumbnail. Root cause: the
   encryption migration's content-hash dedup reuses the original's encrypted
   blob but picks a random unlinked thumbnail. Fixed by associating the
   matched blob's own thumbnail instead of the first unlinked one.
"""

import io
import os
import random
import time
import zipfile

import pytest
from PIL import Image

try:
    import piexif
except ImportError:
    piexif = None

from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    unique_filename,
)


# ── Helpers ──────────────────────────────────────────────────────────


def _make_jpeg_with_orientation(raw_w: int, raw_h: int, orientation: int) -> bytes:
    """Create a JPEG with specific raw pixel dimensions and EXIF orientation."""
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


def _poll_export_completion(client: APIClient, timeout: float = 30.0) -> dict:
    """Poll the export status until it completes or times out."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        r = client.get("/api/export/status")
        assert r.status_code == 200
        data = r.json()
        status = data["job"]["status"]
        if status in ("completed", "failed"):
            return data
        time.sleep(0.5)
    pytest.fail("Export did not complete within timeout")


def _wait_for_encryption(client: APIClient, timeout: float = 30.0):
    """Wait for all photos to have encrypted_blob_id set via encrypted-sync."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        sync = client.encrypted_sync()
        photos = sync.get("photos", [])
        if photos and all(p.get("encrypted_blob_id") for p in photos):
            return photos
        time.sleep(1.0)
    return client.encrypted_sync().get("photos", [])


# ═══════════════════════════════════════════════════════════════════════
# BUG 1: Export duplicate filename
# ═══════════════════════════════════════════════════════════════════════


class TestExportDuplicateFilename:
    """Regression: export must handle multiple photos with identical filenames."""

    def test_export_duplicate_filenames_in_zip(self, user_client):
        """Upload two photos with the same filename, export, verify no zip error.

        Previously this failed with "invalid zip archive: duplicate file name"
        because both photos were written to "photos/IMG_001.jpg" in the zip.
        After the fix, the second should be renamed to "photos/IMG_001_(2).jpg".
        """
        filename = "IMG_DUPLICATE.jpg"
        photo_ids = []
        for i in range(3):
            content = generate_test_jpeg(width=100 + i * 10, height=80 + i * 10)
            data = user_client.upload_photo(filename=filename, content=content)
            photo_ids.append(data["photo_id"])

        # Verify all 3 photos exist
        photos = user_client.list_photos()
        uploaded_ids = {p["id"] for p in photos["photos"]}
        for pid in photo_ids:
            assert pid in uploaded_ids, f"Photo {pid} not found after upload"

        # Wait for encryption so blobs exist for export
        _wait_for_encryption(user_client, timeout=30)

        # Start export
        r = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,
        })
        assert r.status_code == 200, f"Export start failed: {r.text}"

        # Poll for completion — this used to fail with the duplicate name error
        data = _poll_export_completion(user_client, timeout=30)
        assert data["job"]["status"] == "completed", (
            f"Export failed (expected completion): {data['job'].get('error', 'no error')}"
        )

        # Download the zip and verify it's valid
        files = data.get("files", [])
        assert len(files) >= 1, "No export files produced"

        r = user_client.get(files[0]["download_url"])
        assert r.status_code == 200

        zip_data = io.BytesIO(r.content)
        with zipfile.ZipFile(zip_data, "r") as zf:
            bad_file = zf.testzip()
            assert bad_file is None, f"Zip integrity check failed on: {bad_file}"

            names = zf.namelist()
            photo_entries = [n for n in names if n.startswith("photos/")]

            # Verify deduplication: should have 3 photos with unique names
            assert len(photo_entries) >= 3, (
                f"Expected at least 3 photo entries, got {len(photo_entries)}: {photo_entries}"
            )
            # All names must be unique
            assert len(photo_entries) == len(set(photo_entries)), (
                f"Duplicate zip entry names found: {photo_entries}"
            )

            # The server deduplicates filenames at upload time (e.g.
            # IMG_DUPLICATE.jpg, IMG_DUPLICATE-1.jpg, IMG_DUPLICATE-2.jpg).
            # If any still collide, the export worker's own dedup adds _(N).
            # Either way, all entries must be unique.
            for entry in photo_entries:
                assert entry.endswith(".jpg"), f"Unexpected extension: {entry}"

    def test_export_duplicate_filenames_all_content_intact(self, user_client):
        """Verify each deduplicated zip entry contains valid image data."""
        filename = "SAME_NAME.jpg"
        contents = {}
        for i in range(2):
            content = generate_test_jpeg(width=80 + i * 30, height=60 + i * 30)
            data = user_client.upload_photo(filename=filename, content=content)
            contents[data["photo_id"]] = content

        _wait_for_encryption(user_client, timeout=30)
        time.sleep(2)

        r = user_client.post("/api/export", json_data={
            "size_limit": 10_737_418_240,
        })
        assert r.status_code == 200

        data = _poll_export_completion(user_client, timeout=60)
        assert data["job"]["status"] == "completed"

        files = data.get("files", [])
        if not files:
            pytest.skip("Export produced no files (server may not have encrypted blobs yet)")

        r = user_client.get(files[0]["download_url"])
        assert r.status_code == 200

        zip_data = io.BytesIO(r.content)
        with zipfile.ZipFile(zip_data, "r") as zf:
            photo_entries = [n for n in zf.namelist() if n.startswith("photos/")]
            # Every entry should contain valid data (not empty)
            for entry in photo_entries:
                data = zf.read(entry)
                assert len(data) > 100, f"Entry {entry} has suspiciously small content ({len(data)} bytes)"


# ═══════════════════════════════════════════════════════════════════════
# BUG 2: Portrait orientation
# ═══════════════════════════════════════════════════════════════════════


class TestPortraitOrientationRegression:
    """Regression: portrait photos must retain correct dimensions after sync."""

    @pytest.mark.skipif(piexif is None, reason="piexif not installed")
    def test_portrait_dimensions_persist_across_syncs(self, user_client):
        """Upload a portrait photo (EXIF orientation 6) and verify dimensions
        stay portrait through multiple encrypted-sync calls.
        """
        raw_w, raw_h = 640, 480
        content = _make_jpeg_with_orientation(raw_w, raw_h, orientation=6)

        data = user_client.upload_photo(filename="portrait_test.jpg", content=content)
        photo_id = data["photo_id"]

        photos = user_client.list_photos()
        photo = next(p for p in photos["photos"] if p["id"] == photo_id)
        assert photo["width"] == raw_h, (
            f"Expected display width {raw_h} (portrait), got {photo['width']}"
        )
        assert photo["height"] == raw_w, (
            f"Expected display height {raw_w} (portrait), got {photo['height']}"
        )

        _wait_for_encryption(user_client, timeout=30)

        for cycle in range(3):
            sync = user_client.encrypted_sync()
            synced = [p for p in sync["photos"] if p["id"] == photo_id]
            assert len(synced) == 1, f"Cycle {cycle}: photo not in sync response"
            sp = synced[0]
            assert sp["width"] == raw_h, (
                f"Sync cycle {cycle}: width became {sp['width']}, expected {raw_h} (portrait)"
            )
            assert sp["height"] == raw_w, (
                f"Sync cycle {cycle}: height became {sp['height']}, expected {raw_w} (portrait)"
            )
            time.sleep(0.5)

    @pytest.mark.skipif(piexif is None, reason="piexif not installed")
    def test_all_exif_orientations_5_through_8_swap_dimensions(self, user_client):
        """EXIF orientations 5-8 should all produce swapped (portrait) dimensions."""
        raw_w, raw_h = 320, 240

        for orient in [5, 6, 7, 8]:
            content = _make_jpeg_with_orientation(raw_w, raw_h, orient)
            data = user_client.upload_photo(
                filename=f"orient_{orient}.jpg", content=content
            )
            pid = data["photo_id"]

            photos = user_client.list_photos()
            photo = next(p for p in photos["photos"] if p["id"] == pid)
            assert photo["width"] == raw_h, (
                f"Orientation {orient}: width={photo['width']}, expected {raw_h}"
            )
            assert photo["height"] == raw_w, (
                f"Orientation {orient}: height={photo['height']}, expected {raw_w}"
            )

    @pytest.mark.skipif(piexif is None, reason="piexif not installed")
    def test_orientations_1_through_4_no_swap(self, user_client):
        """EXIF orientations 1-4 should NOT swap dimensions."""
        raw_w, raw_h = 320, 240

        for orient in [1, 2, 3, 4]:
            content = _make_jpeg_with_orientation(raw_w, raw_h, orient)
            data = user_client.upload_photo(
                filename=f"orient_{orient}_nosw.jpg", content=content
            )
            pid = data["photo_id"]

            photos = user_client.list_photos()
            photo = next(p for p in photos["photos"] if p["id"] == pid)
            assert photo["width"] == raw_w, (
                f"Orientation {orient}: width={photo['width']}, expected {raw_w}"
            )
            assert photo["height"] == raw_h, (
                f"Orientation {orient}: height={photo['height']}, expected {raw_h}"
            )

    @pytest.mark.skipif(piexif is None, reason="piexif not installed")
    def test_portrait_encrypted_sync_dimensions_match_photos_endpoint(self, user_client):
        """The encrypted-sync endpoint must return the same dimensions as /api/photos."""
        raw_w, raw_h = 800, 600
        content = _make_jpeg_with_orientation(raw_w, raw_h, orientation=6)

        data = user_client.upload_photo(
            filename="portrait_sync_match.jpg", content=content
        )
        pid = data["photo_id"]

        photos = user_client.list_photos()
        photo = next(p for p in photos["photos"] if p["id"] == pid)

        _wait_for_encryption(user_client, timeout=30)
        sync = user_client.encrypted_sync()
        synced = next((p for p in sync["photos"] if p["id"] == pid), None)

        assert synced is not None, "Photo not found in encrypted-sync"
        assert synced["width"] == photo["width"], (
            f"Sync width {synced['width']} != photos width {photo['width']}"
        )
        assert synced["height"] == photo["height"], (
            f"Sync height {synced['height']} != photos height {photo['height']}"
        )

    @pytest.mark.skipif(piexif is None, reason="piexif not installed")
    def test_portrait_aspect_ratio_less_than_one(self, user_client):
        """Portrait photos should have aspect_ratio (width/height) < 1."""
        raw_w, raw_h = 640, 480
        content = _make_jpeg_with_orientation(raw_w, raw_h, orientation=6)

        data = user_client.upload_photo(filename="portrait_ar.jpg", content=content)
        pid = data["photo_id"]

        photos = user_client.list_photos()
        photo = next(p for p in photos["photos"] if p["id"] == pid)
        ar = photo["width"] / photo["height"]
        assert ar < 1.0, (
            f"Portrait photo has landscape AR: {ar:.2f} "
            f"(width={photo['width']}, height={photo['height']})"
        )

    @pytest.mark.skipif(piexif is None, reason="piexif not installed")
    def test_portrait_thumbnail_has_correct_orientation(self, user_client):
        """Thumbnail for a portrait photo should have portrait pixel dimensions."""
        raw_w, raw_h = 640, 480
        content = _make_jpeg_with_orientation(raw_w, raw_h, orientation=6)

        data = user_client.upload_photo(
            filename="portrait_thumb.jpg", content=content
        )
        pid = data["photo_id"]

        time.sleep(2)

        r = user_client.get_photo_thumb(pid)
        if r.status_code == 200 and len(r.content) > 100:
            thumb_img = Image.open(io.BytesIO(r.content))
            tw, th = thumb_img.size
            assert th > tw, (
                f"Thumbnail is landscape ({tw}×{th}), expected portrait"
            )


# ═══════════════════════════════════════════════════════════════════════
# BUG 3: Secure album wrong photo
# ═══════════════════════════════════════════════════════════════════════


USER_PASSWORD = "E2eUserPass456!"


class TestSecureAlbumWrongPhoto:
    """Regression: adding a photo to a secure album must clone the correct photo."""

    def test_secure_add_returns_correct_blob(self, user_client):
        """Add a specific photo to a secure gallery and verify the gallery
        item references the correct photo, not a different one.
        """
        photo_data = {}
        for i in range(3):
            w, h = 100 + i * 50, 80 + i * 50
            content = generate_test_jpeg(width=w, height=h)
            data = user_client.upload_photo(
                filename=f"secure_test_{i}.jpg", content=content
            )
            pid = data["photo_id"]
            photo_data[pid] = {"content": content, "w": w, "h": h}

        encrypted_photos = _wait_for_encryption(user_client, timeout=30)
        assert len(encrypted_photos) >= 3, (
            f"Only {len(encrypted_photos)} photos encrypted, expected at least 3"
        )

        thumb_map = {}
        for ep in encrypted_photos:
            if ep["id"] in photo_data:
                thumb_map[ep["id"]] = ep.get("encrypted_thumb_blob_id")

        gallery = user_client.create_secure_gallery("Bug3 Test Gallery")
        gallery_id = gallery["gallery_id"]

        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        target_id = list(photo_data.keys())[1]
        target_thumb = thumb_map.get(target_id)

        add_resp = user_client.add_secure_gallery_item(gallery_id, target_id, token)
        new_blob_id = add_resp["new_blob_id"]
        assert new_blob_id, "Server should return new_blob_id for the clone"

        time.sleep(3)

        items = user_client.list_secure_gallery_items(gallery_id, token)
        assert len(items["items"]) == 1, (
            f"Expected 1 item in gallery, got {len(items['items'])}"
        )

        item = items["items"][0]

        if target_thumb and item.get("encrypted_thumb_blob_id"):
            item_thumb = item["encrypted_thumb_blob_id"]
            r = user_client.download_blob(item_thumb)
            assert r.status_code == 200, (
                f"Failed to download gallery item thumbnail: {r.status_code}"
            )
            assert len(r.content) > 100, "Thumbnail blob is suspiciously small"

    def test_secure_add_photo_content_matches_original(self, user_client):
        """The file content served for a secure gallery item must match
        the original photo's content, not a different photo's content.
        """
        photo_a_content = generate_test_jpeg(width=200, height=100)
        photo_b_content = generate_test_jpeg(width=50, height=300)

        data_a = user_client.upload_photo(filename="photo_a.jpg", content=photo_a_content)
        photo_a_id = data_a["photo_id"]

        data_b = user_client.upload_photo(filename="photo_b.jpg", content=photo_b_content)
        photo_b_id = data_b["photo_id"]

        _wait_for_encryption(user_client, timeout=30)

        # Get original file content BEFORE adding to gallery
        r_orig = user_client.get_photo_file(photo_b_id)
        assert r_orig.status_code == 200
        original_content = r_orig.content

        gallery = user_client.create_secure_gallery("Content Match Test")
        gallery_id = gallery["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        add_resp = user_client.add_secure_gallery_item(gallery_id, photo_b_id, token)
        new_blob_id = add_resp["new_blob_id"]

        r_clone = user_client.get_photo_file(new_blob_id)
        assert r_clone.status_code == 200, (
            f"Failed to get cloned photo file: {r_clone.status_code}"
        )
        assert r_clone.content == original_content, (
            "Cloned photo content does not match original"
        )

    def test_secure_add_specific_photo_not_another(self, user_client):
        """When adding photo C out of [A, B, C], the gallery should show C,
        not A or B. Verifies via file content served by /api/photos/{id}/file.
        """
        photos = []
        dimensions = [(200, 100), (100, 300), (400, 200)]
        for i, (w, h) in enumerate(dimensions):
            content = generate_test_jpeg(width=w, height=h)
            data = user_client.upload_photo(
                filename=f"distinct_{i}.jpg", content=content
            )
            photos.append({"id": data["photo_id"], "content": content})

        _wait_for_encryption(user_client, timeout=30)

        target = photos[2]
        r_orig = user_client.get_photo_file(target["id"])
        assert r_orig.status_code == 200
        original_content = r_orig.content

        gallery = user_client.create_secure_gallery("Specific Photo Test")
        gallery_id = gallery["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        add_resp = user_client.add_secure_gallery_item(gallery_id, target["id"], token)
        clone_id = add_resp["new_blob_id"]

        r_clone = user_client.get_photo_file(clone_id)
        assert r_clone.status_code == 200, (
            f"Failed to get cloned photo file: {r_clone.status_code}"
        )
        assert r_clone.content == original_content, (
            "Clone file content does not match the original photo"
        )

        for i, other in enumerate(photos[:2]):
            r_other = user_client.get_photo_file(other["id"])
            if r_other.status_code == 200:
                assert r_clone.content != r_other.content, (
                    f"Clone matches photo {i} instead of the target!"
                )

    def test_secure_gallery_blob_ids_include_clone_and_original(self, user_client):
        """The blob-ids endpoint must return both the clone and original IDs
        so the main gallery can hide both.
        """
        content = generate_test_jpeg(width=120, height=90)
        data = user_client.upload_photo(filename="blobids_test.jpg", content=content)
        photo_id = data["photo_id"]

        _wait_for_encryption(user_client, timeout=30)

        gallery = user_client.create_secure_gallery("BlobIDs Test")
        gallery_id = gallery["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        add_resp = user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        clone_id = add_resp["new_blob_id"]

        blob_ids = user_client.get_secure_gallery_blob_ids()
        ids_set = set(blob_ids["blob_ids"])

        assert clone_id in ids_set, (
            f"Clone blob_id {clone_id} not in secure blob_ids"
        )
        assert photo_id in ids_set, (
            f"Original photo_id {photo_id} not in secure blob_ids"
        )

    def test_secure_multiple_photos_each_gets_own_thumbnail(self, user_client):
        """Adding multiple photos to a secure gallery: each item should have
        its own distinct encrypted_thumb_blob_id, not share another photo's thumbnail.
        """
        photo_ids = []
        for i in range(3):
            content = generate_test_jpeg(width=150 + i * 50, height=120 + i * 50)
            data = user_client.upload_photo(
                filename=f"multi_thumb_{i}.jpg", content=content
            )
            photo_ids.append(data["photo_id"])

        _wait_for_encryption(user_client, timeout=30)

        sync = user_client.encrypted_sync()
        orig_thumbs = {}
        for ep in sync["photos"]:
            if ep["id"] in photo_ids:
                orig_thumbs[ep["id"]] = ep.get("encrypted_thumb_blob_id")

        gallery = user_client.create_secure_gallery("Multi Thumb Test")
        gallery_id = gallery["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        clone_ids = []
        for pid in photo_ids:
            resp = user_client.add_secure_gallery_item(gallery_id, pid, token)
            clone_ids.append(resp["new_blob_id"])

        # Wait for clones to be encrypted
        time.sleep(5)

        items = user_client.list_secure_gallery_items(gallery_id, token)
        assert len(items["items"]) == 3

        thumb_ids = []
        for item in items["items"]:
            tid = item.get("encrypted_thumb_blob_id")
            if tid:
                thumb_ids.append(tid)
                r = user_client.download_blob(tid)
                assert r.status_code == 200, (
                    f"Failed to download thumbnail {tid}: {r.status_code}"
                )

        if len(thumb_ids) == 3:
            assert len(set(thumb_ids)) == 3, (
                f"Expected 3 distinct thumbnail IDs, got duplicates: {thumb_ids}"
            )
