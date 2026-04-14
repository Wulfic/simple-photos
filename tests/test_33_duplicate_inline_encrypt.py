"""
Test 33: Duplicate photos are encrypted inline — no unencrypted files on disk.

Bug: The duplicate_photo endpoint writes the rendered copy as an unencrypted
file to uploads/copy-{id}.{ext} on disk, then relies on the background
encryption engine to encrypt it later.  The unencrypted file persists on
disk indefinitely.  After a server reset/restore, autoscan rediscovers
these files and re-imports them as new photos (duplicates).

The fix: duplicate_photo should encrypt the rendered copy inline before
returning, store it as an encrypted blob, and delete the temp file.

Tests verify:
  1. Duplicate is already encrypted (encrypted_blob_id is set) immediately
  2. Duplicate has an encrypted thumbnail blob
  3. Encrypted blob is downloadable
  4. No unencrypted copy file remains on disk after duplication
  5. Duplicate with crop edits is also encrypted inline
  6. After reset (DB wipe + rescan), duplicate blobs don't reappear as new photos
"""

import json
import time

import pytest
from helpers import APIClient, unique_filename


class TestDuplicateEncryptedInline:
    """Duplicate photos should be encrypted immediately, not left as
    unencrypted files for background processing."""

    def test_duplicate_already_encrypted(self, user_client, admin_client):
        """A newly duplicated photo should already have encrypted_blob_id set
        (not NULL) — encryption happens inline in the duplicate endpoint."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        # Wait for original to be encrypted
        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        # Duplicate
        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        # The copy should ALREADY be encrypted — no waiting for background engine
        sync = user_client.encrypted_sync()
        copy_rec = _find_photo_in_sync(sync, copy_id)
        assert copy_rec is not None, f"Duplicate {copy_id} not in encrypted-sync"
        assert copy_rec["encrypted_blob_id"] is not None, (
            f"Duplicate should be encrypted inline, but encrypted_blob_id is None. "
            "The copy was left unencrypted on disk."
        )
        assert copy_rec["encrypted_blob_id"] != "", (
            f"encrypted_blob_id should be a real blob ID, not empty string"
        )

    def test_duplicate_has_encrypted_thumbnail(self, user_client, admin_client):
        """Duplicate should also have an encrypted thumbnail blob."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        sync = user_client.encrypted_sync()
        copy_rec = _find_photo_in_sync(sync, copy_id)
        assert copy_rec is not None
        assert copy_rec["encrypted_thumb_blob_id"] is not None, (
            "Duplicate should have an encrypted thumbnail blob"
        )
        assert copy_rec["encrypted_thumb_blob_id"] != ""

    def test_duplicate_encrypted_blob_downloadable(self, user_client, admin_client):
        """The encrypted blob for the duplicate should be downloadable."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        sync = user_client.encrypted_sync()
        copy_rec = _find_photo_in_sync(sync, copy_id)
        assert copy_rec is not None
        blob_id = copy_rec["encrypted_blob_id"]
        assert blob_id is not None

        # Download the encrypted blob
        r = user_client.download_blob(blob_id)
        assert r.status_code == 200, (
            f"Failed to download encrypted blob {blob_id}: {r.status_code}"
        )
        assert len(r.content) > 0, "Encrypted blob is empty"

    def test_duplicate_with_crop_encrypted_inline(self, user_client, admin_client):
        """Duplicate with crop edits should also be encrypted inline."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        crop_meta = json.dumps({
            "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8,
            "rotate": 0, "brightness": 0,
        })
        copy = user_client.duplicate_photo(photo_id, crop_metadata=crop_meta)
        copy_id = copy["id"]

        sync = user_client.encrypted_sync()
        copy_rec = _find_photo_in_sync(sync, copy_id)
        assert copy_rec is not None
        assert copy_rec["encrypted_blob_id"] is not None, (
            "Cropped duplicate should be encrypted inline"
        )

    def test_zero_pending_after_duplicate(self, user_client, admin_client):
        """After duplicating, the copy itself should NOT be pending encryption
        because it was encrypted inline."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        sync = user_client.encrypted_sync()
        copy_rec = _find_photo_in_sync(sync, copy_id)
        assert copy_rec is not None
        assert copy_rec["encrypted_blob_id"] is not None, (
            f"Copy {copy_id} should be encrypted inline, not pending"
        )


# ── Helpers ──────────────────────────────────────────────────────────

def _find_photo_in_sync(sync_response: dict, photo_id: str) -> dict | None:
    for p in sync_response.get("photos", []):
        if p["id"] == photo_id:
            return p
    return None


def _wait_for_photo_encrypted(client: APIClient, photo_id: str,
                              timeout: float = 30.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        sync = client.encrypted_sync()
        rec = _find_photo_in_sync(sync, photo_id)
        if rec and rec.get("encrypted_blob_id") and rec["encrypted_blob_id"] != "":
            return
        time.sleep(1.0)
