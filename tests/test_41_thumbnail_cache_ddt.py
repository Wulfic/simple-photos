"""
Test 41: Thumbnail Cache Lifecycle — Data-Driven Tests.

Pre-refactor safety net verifying thumbnail availability through the full
photo lifecycle: upload, secure gallery move, trash, restore, encryption.

These tests verify the SERVER-SIDE contracts that the client cache depends on.
The web client's IndexedDB cache derives from these server responses, so if
these pass, the client cache has the correct data available.

Lifecycle states tested:
  - Fresh upload → thumbnail available via /photos/{id}/thumb
  - Encrypted-sync → encrypted_thumb_blob_id populated
  - Secure gallery move → thumbnail available in gallery item listing
  - Trash → thumbnail NOT available (404)
  - Restore from trash → thumbnail available again
  - Blob upload (encrypted workflow) → blob downloadable
"""

import json
import time

import pytest

from helpers import (
    APIClient,
    generate_test_gif,
    generate_test_jpeg,
)


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

USER_PASSWORD = "E2eUserPass456!"


def _upload_and_settle(client: APIClient, filename: str, content: bytes,
                       mime_type: str, settle: float = 2.0) -> str:
    resp = client.upload_photo(filename, content, mime_type)
    photo_id = resp["photo_id"]
    time.sleep(settle)
    return photo_id


def _wait_for_thumb(client: APIClient, photo_id: str, timeout: float = 15) -> bytes:
    deadline = time.time() + timeout
    while time.time() < deadline:
        r = client.get_photo_thumb(photo_id)
        if r.status_code == 200 and len(r.content) > 0:
            return r.content
        time.sleep(1)
    raise AssertionError(f"Thumbnail for {photo_id} not ready within {timeout}s")


def _wait_encryption(client: APIClient, photo_id: str, timeout: float = 60,
                     admin_client: APIClient = None) -> dict:
    """Trigger a scan (which triggers encryption migration) then poll encrypted-sync."""
    # Trigger scan via admin → auto_migrate_after_scan encrypts unencrypted photos
    trigger_client = admin_client or client
    try:
        trigger_client.admin_trigger_scan()
    except Exception:
        pass  # Best-effort trigger
    deadline = time.time() + timeout
    retrigger_at = time.time() + 15  # Re-trigger once midway
    while time.time() < deadline:
        sync = client.encrypted_sync()
        for p in sync.get("photos", []):
            if p["id"] == photo_id and p.get("encrypted_blob_id"):
                return p
        if admin_client and time.time() >= retrigger_at:
            try:
                admin_client.admin_trigger_scan()
            except Exception:
                pass
            retrigger_at = deadline + 1  # Only re-trigger once
        time.sleep(2)
    raise AssertionError(f"Photo {photo_id} not encrypted within {timeout}s")


# ══════════════════════════════════════════════════════════════════════
# Test: Thumbnail Available After Upload
# ══════════════════════════════════════════════════════════════════════

UPLOAD_CASES = [
    pytest.param("lifecycle_land.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 200, "height": 133},
                 id="upload_jpeg_landscape"),
    pytest.param("lifecycle_port.jpg", "image/jpeg", generate_test_jpeg,
                 {"width": 100, "height": 200},
                 id="upload_jpeg_portrait"),
    pytest.param("lifecycle_gif.gif", "image/gif", generate_test_gif,
                 {"width": 30, "height": 20, "frames": 2},
                 id="upload_gif_animated"),
]


class TestThumbnailAfterUpload:
    """Thumbnails must be available immediately after upload."""

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw", UPLOAD_CASES)
    def test_thumb_200_after_upload(self, user_client, filename, mime, gen_func, gen_kw):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime, settle=3)
        thumb = _wait_for_thumb(user_client, photo_id)
        assert len(thumb) > 50, f"Thumbnail too small: {len(thumb)} bytes"

    @pytest.mark.parametrize("filename,mime,gen_func,gen_kw", UPLOAD_CASES)
    def test_thumb_content_type_header(self, user_client, filename, mime, gen_func, gen_kw):
        content = gen_func(**gen_kw)
        photo_id = _upload_and_settle(user_client, filename, content, mime, settle=3)
        _wait_for_thumb(user_client, photo_id)  # Ensure ready
        r = user_client.get_photo_thumb(photo_id)
        ct = r.headers.get("content-type", "")
        assert "image" in ct, f"Thumbnail content-type should be image/*, got: {ct}"


# ══════════════════════════════════════════════════════════════════════
# Test: Encrypted Thumbnail Blob ID Populated
# ══════════════════════════════════════════════════════════════════════

class TestEncryptedThumbnailTracking:
    """After encryption migration, encrypted_thumb_blob_id must be set."""

    def test_encrypted_thumb_blob_id_set(self, user_client, admin_client):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "enc_thumb.jpg", content, "image/jpeg")
        synced = _wait_encryption(user_client, photo_id, admin_client=admin_client)
        assert synced.get("encrypted_thumb_blob_id"), (
            "encrypted_thumb_blob_id should be set after encryption"
        )

    def test_encrypted_thumb_distinct_from_blob(self, user_client, admin_client):
        """Encrypted thumb blob should be different from the photo blob."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "enc_distinct.jpg", content, "image/jpeg")
        synced = _wait_encryption(user_client, photo_id, admin_client=admin_client)
        enc_blob = synced.get("encrypted_blob_id")
        enc_thumb = synced.get("encrypted_thumb_blob_id")
        assert enc_blob and enc_thumb
        assert enc_blob != enc_thumb, "Encrypted blob and thumb should be distinct"


# ══════════════════════════════════════════════════════════════════════
# Test: Trash → Thumbnail Gone, Restore → Thumbnail Back
# ══════════════════════════════════════════════════════════════════════

class TestThumbnailTrashRestore:
    """Thumbnail lifecycle through trash and restore."""

    def test_thumb_404_after_trash(self, user_client):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "trash_thumb.jpg", content, "image/jpeg", settle=3)
        _wait_for_thumb(user_client, photo_id)  # Ensure it's ready

        # Soft-delete (trash) via DELETE /api/photos/{id}
        r = user_client.delete(f"/api/photos/{photo_id}")
        assert r.status_code == 200
        time.sleep(1)

        # Thumbnail should be 404 / not available
        r = user_client.get_photo_thumb(photo_id)
        assert r.status_code in (404, 410), (
            f"Expected 404/410 after trash, got {r.status_code}"
        )

    def test_thumb_restored_after_untrash(self, user_client):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "restore_thumb.jpg", content, "image/jpeg", settle=3)
        _wait_for_thumb(user_client, photo_id)

        # Trash it via DELETE /api/photos/{id}
        r = user_client.delete(f"/api/photos/{photo_id}")
        assert r.status_code == 200
        time.sleep(1)

        # Find in trash and restore
        trash = user_client.list_trash()
        trash_items = trash.get("items", trash) if isinstance(trash, dict) else trash
        trash_entry = None
        for item in trash_items:
            if item.get("photo_id") == photo_id:
                trash_entry = item
                break
        assert trash_entry, f"Photo {photo_id} not found in trash"

        user_client.restore_trash(trash_entry["id"])
        time.sleep(2)

        # Thumbnail should be back
        thumb = _wait_for_thumb(user_client, photo_id)
        assert len(thumb) > 50


# ══════════════════════════════════════════════════════════════════════
# Test: Secure Gallery — Thumbnail Info In Item Listing
# ══════════════════════════════════════════════════════════════════════

class TestSecureGalleryThumbnailInfo:
    """Secure gallery items must include thumbnail blob ID for client rendering."""

    def test_gallery_item_has_thumb_blob_id(self, user_client, admin_client):
        """After adding to secure gallery, item listing includes encrypted_thumb_blob_id."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "sg_thumb.jpg", content, "image/jpeg")

        # Wait for encryption
        _wait_encryption(user_client, photo_id, admin_client=admin_client)

        gallery = user_client.create_secure_gallery("thumb_test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(3)

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)["items"]
        assert len(items) >= 1
        item = items[0]

        # The item should have an encrypted_thumb_blob_id for the client to fetch
        thumb_field = item.get("encrypted_thumb_blob_id")
        assert thumb_field, (
            "Gallery item should have encrypted_thumb_blob_id for client-side thumbnail rendering"
        )

    def test_gallery_item_thumb_blob_downloadable(self, user_client, admin_client):
        """The encrypted_thumb_blob_id should be downloadable as a blob."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "sg_dl.jpg", content, "image/jpeg")
        _wait_encryption(user_client, photo_id, admin_client=admin_client)

        gallery = user_client.create_secure_gallery("thumb_dl_test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(3)

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)["items"]
        thumb_blob_id = items[0].get("encrypted_thumb_blob_id")
        assert thumb_blob_id, "No encrypted_thumb_blob_id in gallery item"

        r = user_client.download_blob(thumb_blob_id)
        assert r.status_code == 200, f"Thumbnail blob download failed: {r.status_code}"
        assert len(r.content) > 50, "Downloaded thumbnail blob is too small"


# ══════════════════════════════════════════════════════════════════════
# Test: Blob Upload (Encrypted Workflow) — Thumbnail from Encrypted Data
# ══════════════════════════════════════════════════════════════════════

class TestBlobWorkflowThumbnail:
    """Encrypted blob workflow: verify blobs remain downloadable."""

    def test_blob_upload_and_download(self, user_client):
        """Upload a blob and verify it's downloadable."""
        content = generate_test_jpeg(50, 50)
        blob = user_client.upload_blob("photo", content)
        blob_id = blob["blob_id"]

        r = user_client.download_blob(blob_id)
        assert r.status_code == 200
        assert len(r.content) == len(content)

    def test_blob_list_includes_upload(self, user_client):
        """Uploaded blobs appear in blob listing."""
        content = generate_test_jpeg(50, 50)
        blob = user_client.upload_blob("photo", content)
        blob_id = blob["blob_id"]

        blobs = user_client.list_blobs()
        blob_ids = [b["id"] for b in blobs.get("blobs", blobs)]
        assert blob_id in blob_ids


# ══════════════════════════════════════════════════════════════════════
# Test: Multiple Operations Don't Corrupt Cache
# ══════════════════════════════════════════════════════════════════════

class TestCacheIntegrityMultiOp:
    """Rapid operations shouldn't cause stale/wrong thumbnails."""

    def test_upload_crop_thumb_still_valid(self, user_client):
        """Thumbnail still serves after crop metadata is set."""
        content = generate_test_jpeg(200, 150)
        photo_id = _upload_and_settle(user_client, "crop_cache.jpg", content, "image/jpeg", settle=3)
        _wait_for_thumb(user_client, photo_id)

        # Set crop
        meta = {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8, "rotate": 0, "brightness": 0}
        user_client.crop_photo(photo_id, json.dumps(meta))
        time.sleep(1)

        # Thumbnail should still be 200
        r = user_client.get_photo_thumb(photo_id)
        assert r.status_code == 200
        assert len(r.content) > 50

    def test_favorite_doesnt_break_thumb(self, user_client):
        """Toggling favorite should not affect thumbnail availability."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload_and_settle(user_client, "fav_cache.jpg", content, "image/jpeg", settle=3)
        _wait_for_thumb(user_client, photo_id)

        user_client.favorite_photo(photo_id)
        time.sleep(0.5)

        r = user_client.get_photo_thumb(photo_id)
        assert r.status_code == 200

    def test_duplicate_original_thumb_intact(self, user_client):
        """After duplicating a photo, original's thumbnail still works."""
        content = generate_test_jpeg(200, 150)
        photo_id = _upload_and_settle(user_client, "dup_cache.jpg", content, "image/jpeg", settle=3)
        _wait_for_thumb(user_client, photo_id)

        meta = {"x": 0, "y": 0, "width": 1.0, "height": 1.0, "rotate": 90, "brightness": 0}
        user_client.duplicate_photo(photo_id, json.dumps(meta))
        time.sleep(3)

        # Original thumbnail still valid
        r = user_client.get_photo_thumb(photo_id)
        assert r.status_code == 200
        assert len(r.content) > 50
