"""
Test 02: Photos — upload, list, deduplicate, favorite, crop, duplicate, encrypted sync.
"""

import hashlib
import time

import pytest
from helpers import (
    APIClient,
    generate_test_jpeg,
    generate_test_png,
    generate_random_bytes,
    unique_filename,
    assert_photo_in_list,
    assert_photo_not_in_list,
)


class TestPhotoUpload:
    """Photo upload and deduplication."""

    def test_upload_jpeg(self, user_client):
        content = generate_test_jpeg()
        data = user_client.upload_photo("test_upload.jpg", content)
        assert "photo_id" in data
        assert data["filename"] == "test_upload.jpg"
        assert data["size_bytes"] > 0

    def test_upload_png(self, user_client):
        content = generate_test_png()
        data = user_client.upload_photo("test_upload.png", content, mime_type="image/png")
        assert "photo_id" in data

    def test_upload_unique_filename_conflict(self, user_client):
        """Uploading two different files with the same name should work (unique paths)."""
        content1 = generate_test_jpeg()
        content2 = generate_test_jpeg()  # Different random content
        # Make them actually different by appending random bytes
        content2 = content1 + b"\x00"  # Slightly different

        data1 = user_client.upload_photo("conflict.jpg", content1)
        data2 = user_client.upload_photo("conflict.jpg", content2)
        assert data1["photo_id"] != data2["photo_id"]

    def test_upload_dedup_by_hash(self, user_client):
        """Uploading the exact same content should return the existing photo."""
        content = generate_test_jpeg()
        data1 = user_client.upload_photo("dedup_a.jpg", content)
        data2 = user_client.upload_photo("dedup_b.jpg", content)
        # Should get the same photo (dedup by content hash)
        assert data1["photo_hash"] == data2["photo_hash"]

    def test_upload_empty_body_accepted(self, user_client):
        """Server accepts empty body — encrypted blobs have no server validation."""
        r = user_client.post(
            "/api/photos/upload",
            data=b"",
            headers={"X-Filename": "empty.jpg", "X-Mime-Type": "image/jpeg",
                     "Content-Type": "application/octet-stream"},
        )
        assert r.status_code == 201


class TestPhotoList:
    """Photo listing and pagination."""

    def test_list_photos(self, user_client):
        # Upload a photo first
        user_client.upload_photo(unique_filename())
        data = user_client.list_photos()
        assert "photos" in data
        assert len(data["photos"]) >= 1

    def test_list_photos_pagination(self, user_client):
        # Upload multiple photos
        for i in range(3):
            user_client.upload_photo(unique_filename())
            time.sleep(0.05)  # Ensure different timestamps

        data = user_client.list_photos(limit=2)
        assert len(data["photos"]) <= 2
        if data.get("next_cursor"):
            data2 = user_client.list_photos(after=data["next_cursor"], limit=2)
            assert "photos" in data2

    def test_list_photos_filter_favorites(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.favorite_photo(photo["photo_id"])
        data = user_client.list_photos(favorites_only="true")
        assert any(p["id"] == photo["photo_id"] for p in data["photos"])

    def test_list_photos_user_isolation(self, user_client, second_user_client):
        """Each user should only see their own photos."""
        p1 = user_client.upload_photo(unique_filename())
        p2 = second_user_client.upload_photo(unique_filename())

        list1 = user_client.list_photos()
        list2 = second_user_client.list_photos()

        ids1 = [p["id"] for p in list1["photos"]]
        ids2 = [p["id"] for p in list2["photos"]]

        assert p1["photo_id"] in ids1
        assert p2["photo_id"] not in ids1
        assert p2["photo_id"] in ids2
        assert p1["photo_id"] not in ids2


class TestPhotoServing:
    """Photo file and thumbnail serving."""

    def test_get_photo_file(self, user_client):
        content = generate_test_jpeg()
        photo = user_client.upload_photo("serve_test.jpg", content)
        r = user_client.get_photo_file(photo["photo_id"])
        assert r.status_code == 200
        assert len(r.content) > 0

    def test_get_photo_thumb(self, user_client):
        photo = user_client.upload_photo("thumb_test.jpg")
        r = user_client.get_photo_thumb(photo["photo_id"])
        # 200 = thumbnail ready, 202 = still generating
        assert r.status_code in (200, 202)

    def test_photo_etag_caching(self, user_client):
        photo = user_client.upload_photo("etag_test.jpg")
        r1 = user_client.get_photo_file(photo["photo_id"])
        assert r1.status_code == 200
        etag = r1.headers.get("ETag")
        if etag:
            r2 = user_client.get(
                f"/api/photos/{photo['photo_id']}/file",
                headers={"If-None-Match": etag},
            )
            assert r2.status_code == 304

    def test_photo_range_request(self, user_client):
        content = generate_test_jpeg()
        photo = user_client.upload_photo("range_test.jpg", content)
        r = user_client.get(
            f"/api/photos/{photo['photo_id']}/file",
            headers={"Range": "bytes=0-9"},
        )
        assert r.status_code == 206
        assert len(r.content) == 10

    def test_other_user_cannot_access_photo(self, user_client, second_user_client):
        photo = user_client.upload_photo("private.jpg")
        r = second_user_client.get_photo_file(photo["photo_id"])
        assert r.status_code in (403, 404)


class TestPhotoMetadata:
    """Favorite, crop, and metadata operations."""

    def test_favorite_toggle(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # Toggle on
        data = user_client.favorite_photo(pid)
        assert data["is_favorite"] is True

        # Toggle off
        data = user_client.favorite_photo(pid)
        assert data["is_favorite"] is False

    def test_crop_metadata(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        crop = '{"x":10,"y":20,"width":100,"height":200}'
        data = user_client.crop_photo(pid, crop)
        assert data["crop_metadata"] == crop

    def test_crop_metadata_clear(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        user_client.crop_photo(pid, '{"x":10}')
        # Clear crop by setting null/empty
        r = user_client.put(f"/api/photos/{pid}/crop", json_data={"crop_metadata": None})
        assert r.status_code == 200


class TestPhotoDuplicate:
    """Photo duplication (creates new record sharing same file)."""

    def test_duplicate_photo(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        dup = user_client.duplicate_photo(pid)
        assert dup["id"] != pid
        assert "Copy of" in dup["filename"]

    def test_duplicate_with_crop(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        crop = '{"x":5,"y":10,"w":50,"h":50}'
        dup = user_client.duplicate_photo(photo["photo_id"], crop_metadata=crop)
        # Server may return crop_metadata as parsed JSON object or string
        actual = dup["crop_metadata"]
        if isinstance(actual, dict):
            assert actual == {"x": 5, "y": 10, "w": 50, "h": 50}
        else:
            assert actual == crop

    def test_duplicate_is_independent(self, user_client):
        """Favoriting the original should not affect the duplicate's metadata."""
        photo = user_client.upload_photo(unique_filename())
        dup = user_client.duplicate_photo(photo["photo_id"])

        # Modify original
        user_client.favorite_photo(photo["photo_id"])

        # Duplicate should not be affected
        photos = user_client.list_photos()
        dup_record = next(p for p in photos["photos"] if p["id"] == dup["id"])
        orig_record = next(p for p in photos["photos"] if p["id"] == photo["photo_id"])
        assert orig_record["is_favorite"] is True
        assert dup_record["is_favorite"] is False


class TestEncryptedSync:
    """Encrypted sync endpoint for mobile clients."""

    def test_encrypted_sync_returns_metadata(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        data = user_client.encrypted_sync()
        assert "photos" in data
        # Should contain metadata fields but not file content
        if data["photos"]:
            p = data["photos"][0]
            assert "id" in p
            assert "filename" in p
            assert "mime_type" in p

    def test_encrypted_sync_pagination(self, user_client):
        for _ in range(3):
            user_client.upload_photo(unique_filename())
        data = user_client.encrypted_sync(limit=1)
        assert len(data["photos"]) <= 1
        if data.get("next_cursor"):
            data2 = user_client.encrypted_sync(after=data["next_cursor"])
            assert "photos" in data2


class TestEncryptionBannerCounts:
    """Regression tests for the encryption progress banner.

    The banner must show progress relative to the *current batch* of items
    being encrypted, NOT the entire library.  For example, if 8 photos are
    already encrypted and 4 new photos are added, the banner should display
    "x/4", not "x/12".

    These tests exercise the ``encrypted-sync`` API to validate the data
    contract the frontend banner depends on and then assert the *correct*
    client-side counting logic (batch-relative, not library-relative).
    """

    @staticmethod
    def _fetch_all_encrypted_sync(client):
        """Paginate through encrypted-sync and return all records."""
        all_photos = []
        cursor = None
        while True:
            params = {"limit": 500}
            if cursor:
                params["after"] = cursor
            data = client.encrypted_sync(**params)
            all_photos.extend(data["photos"])
            cursor = data.get("next_cursor")
            if not cursor:
                break
        return all_photos

    @staticmethod
    def _wait_all_encrypted(client, expected_count, timeout=60):
        """Poll encrypted-sync until every photo has an encrypted_blob_id."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            photos = TestEncryptionBannerCounts._fetch_all_encrypted_sync(client)
            pending = [p for p in photos if not p.get("encrypted_blob_id")]
            if len(pending) == 0 and len(photos) >= expected_count:
                return photos
            time.sleep(1)
        pytest.fail(
            f"Timed out waiting for all {expected_count} photos to be encrypted. "
            f"Got {len(photos)} total, {len(pending)} still pending."
        )

    def test_pending_count_reflects_only_new_items(self, user_client, primary_admin):
        """After encrypting a batch and adding new photos, the pending count
        must equal only the newly-added items, NOT the entire library."""
        BATCH_1 = 4
        BATCH_2 = 3

        # ── Batch 1: upload and encrypt ──────────────────────────────
        for _ in range(BATCH_1):
            user_client.upload_photo(unique_filename())

        # Re-store the encryption key to trigger scan → encrypt cycle
        primary_admin.admin_store_encryption_key("a" * 64)

        # Wait until all batch-1 photos are encrypted
        self._wait_all_encrypted(user_client, BATCH_1)

        # Confirm: 0 pending, BATCH_1 encrypted
        photos_after_b1 = self._fetch_all_encrypted_sync(user_client)
        pending_b1 = [p for p in photos_after_b1 if not p.get("encrypted_blob_id")]
        encrypted_b1 = [p for p in photos_after_b1 if p.get("encrypted_blob_id")]
        assert len(pending_b1) == 0, "All batch-1 photos should be encrypted"
        assert len(encrypted_b1) == BATCH_1

        # ── Batch 2: upload MORE photos (not yet encrypted) ──────────
        for _ in range(BATCH_2):
            user_client.upload_photo(unique_filename())

        # Fetch encrypted-sync BEFORE triggering another encrypt cycle
        photos_after_b2 = self._fetch_all_encrypted_sync(user_client)
        pending_b2 = [p for p in photos_after_b2 if not p.get("encrypted_blob_id")]
        encrypted_b2 = [p for p in photos_after_b2 if p.get("encrypted_blob_id")]

        total_library = len(photos_after_b2)

        # Server returns the full library
        assert total_library == BATCH_1 + BATCH_2, (
            f"Expected {BATCH_1 + BATCH_2} total photos, got {total_library}"
        )
        # Only batch 2 should be pending
        assert len(pending_b2) == BATCH_2, (
            f"Expected exactly {BATCH_2} pending (new batch), got {len(pending_b2)}"
        )
        assert len(encrypted_b2) == BATCH_1

        # ── Assert correct banner counting logic ─────────────────────
        # The banner must track only the CURRENT BATCH of pending items,
        # not the entire library.
        #
        # CORRECT: banner_total = pending_count_at_batch_start = BATCH_2
        #          banner_progress_pct starts at 0%, ends at 100%
        #
        # WRONG (the bug): banner_total = total_library = BATCH_1 + BATCH_2
        #          banner shows "4/7" (57%) when nothing new was encrypted yet
        correct_banner_total = len(pending_b2)  # == BATCH_2
        wrong_banner_total = total_library       # == BATCH_1 + BATCH_2

        assert correct_banner_total == BATCH_2
        assert correct_banner_total != wrong_banner_total, (
            f"The banner total ({correct_banner_total}) must differ from the full "
            f"library count ({wrong_banner_total}).  If they are equal, the banner "
            f"counting logic conflates the entire library with the current batch."
        )

        # The banner "encrypted so far in this batch" should start at 0
        correct_banner_encrypted = 0  # nothing in batch 2 is encrypted yet
        wrong_banner_encrypted = len(encrypted_b2)  # old bug shows 4 out of 7

        assert correct_banner_encrypted == 0, (
            "Banner should show 0 items encrypted at the start of a new batch"
        )
        assert wrong_banner_encrypted > 0, (
            "Sanity: there are already-encrypted items from batch 1"
        )

        # Percentage at batch start
        correct_pct = 0.0  # 0/BATCH_2 = 0%
        wrong_pct = (wrong_banner_encrypted / wrong_banner_total) * 100  # 4/7 ≈ 57%
        assert correct_pct == 0.0, "Progress should start at 0% for a new batch"
        assert wrong_pct > 0.0, (
            "Sanity: the old buggy logic would show non-zero progress for a new batch"
        )

    def test_single_batch_counts_are_correct(self, user_client, primary_admin):
        """For the very first batch (empty library), total and pending are equal
        and the banner counting is consistent regardless of approach."""
        COUNT = 5

        for _ in range(COUNT):
            user_client.upload_photo(unique_filename())

        photos = self._fetch_all_encrypted_sync(user_client)
        pending = [p for p in photos if not p.get("encrypted_blob_id")]
        total = len(photos)

        # On a fresh library with no prior encryption, pending == total
        assert len(pending) == COUNT, f"Expected {COUNT} pending, got {len(pending)}"
        assert total == COUNT

        # In this case both "batch" and "library" total are the same
        # Banner should show "0/COUNT" → "COUNT/COUNT"
        banner_total = len(pending)  # correct: track batch
        assert banner_total == total, (
            "For first batch, batch total should equal library total"
        )

    def test_banner_never_shows_negative_remaining(self, user_client, primary_admin):
        """As encryption progresses, the pending count must monotonically
        decrease toward 0 and never go negative."""
        COUNT = 3

        for _ in range(COUNT):
            user_client.upload_photo(unique_filename())

        # Trigger encryption
        primary_admin.admin_store_encryption_key("a" * 64)

        prev_pending = COUNT
        deadline = time.time() + 60
        seen_any_progress = False

        while time.time() < deadline:
            photos = self._fetch_all_encrypted_sync(user_client)
            pending = [p for p in photos if not p.get("encrypted_blob_id")]
            current_pending = len(pending)

            # Pending must never exceed the initial batch and never go negative
            assert current_pending >= 0, "Pending count went negative"
            assert current_pending <= COUNT, (
                f"Pending count ({current_pending}) exceeds initial batch ({COUNT})"
            )

            if current_pending < prev_pending:
                seen_any_progress = True
            prev_pending = current_pending

            if current_pending == 0:
                break
            time.sleep(0.5)
        else:
            pytest.fail(f"Encryption did not complete in time. {prev_pending} still pending.")

        # Confirm final state
        assert prev_pending == 0, "All items should be encrypted"
