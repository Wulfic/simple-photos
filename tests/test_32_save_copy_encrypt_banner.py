"""
Test 32: Save Copy Encryption — duplicate photos are encrypted inline.

History: The original bug was that duplicate_photo INSERT set
encrypted_blob_id = '' (empty string), causing a stuck banner.  That was
fixed to use NULL.  Now the endpoint encrypts inline, so the copy's
encrypted_blob_id is immediately set to a real blob ID.

These tests verify:
  1. Duplicated photo's encrypted_blob_id is a real blob ID (not NULL or '')
  2. Duplicated photo's encrypted_thumb_blob_id is set
  3. Duplicate is NOT pending encryption (already done inline)
  4. Duplicate is already encrypted, no engine run needed
  5. Multiple duplicates are all encrypted inline
  6. Duplicate with crop_metadata is also encrypted inline
  7. Banner shows zero pending after inline encryption
"""

import json
import time

import pytest
from helpers import APIClient, unique_filename


class TestDuplicateEncryptedBlobId:
    """Duplicated photos are encrypted inline — no stuck banner, no
    unencrypted files left on disk."""

    def test_duplicate_encrypted_blob_id_is_set(self, user_client, admin_client):
        """After duplicating a photo the copy's encrypted_blob_id must be
        a real blob ID (not NULL or empty string) — encrypted inline."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        # Wait for the original to be encrypted first
        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        # Duplicate the photo
        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        # Fetch the copy via encrypted-sync
        sync = user_client.encrypted_sync()
        copy_record = _find_photo_in_sync(sync, copy_id)
        assert copy_record is not None, f"Duplicate {copy_id} not found in encrypted-sync"

        # Inline encryption: encrypted_blob_id should be a real blob ID
        assert copy_record["encrypted_blob_id"] is not None, (
            f"Expected encrypted_blob_id to be set (inline encryption), got None"
        )
        assert copy_record["encrypted_blob_id"] != "", (
            f"Expected encrypted_blob_id to be a real blob ID, got empty string"
        )

    def test_duplicate_encrypted_thumb_blob_id_is_set(self, user_client, admin_client):
        """Duplicate's encrypted_thumb_blob_id should be set (inline)."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        sync = user_client.encrypted_sync()
        copy_record = _find_photo_in_sync(sync, copy_id)
        assert copy_record is not None

        assert copy_record["encrypted_thumb_blob_id"] is not None, (
            f"Expected encrypted_thumb_blob_id to be set (inline encryption), got None"
        )
        assert copy_record["encrypted_thumb_blob_id"] != "", (
            f"Expected encrypted_thumb_blob_id to be a real blob ID, got empty string"
        )

    def test_duplicate_not_in_encryption_pending(self, user_client, admin_client):
        """A newly duplicated photo should NOT appear as 'needs encryption'
        because it was encrypted inline."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        # The copy should not be pending
        sync = user_client.encrypted_sync()
        pending = [
            p for p in sync["photos"]
            if p["encrypted_blob_id"] is None
        ]
        copy_ids = [p["id"] for p in pending]
        assert copy_id not in copy_ids, (
            f"Duplicate {copy_id} should NOT be in pending encryption list — "
            f"it was encrypted inline."
        )

    def test_duplicate_already_encrypted_no_engine_needed(self, user_client, admin_client):
        """The duplicate should already have a real encrypted_blob_id
        immediately, no need to trigger the encryption engine."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        # Already encrypted — no waiting or engine trigger needed
        sync = user_client.encrypted_sync()
        copy_record = _find_photo_in_sync(sync, copy_id)
        assert copy_record is not None
        assert copy_record["encrypted_blob_id"] is not None, (
            "Duplicate should be encrypted immediately (inline), not require engine"
        )
        assert copy_record["encrypted_blob_id"] != "", (
            "encrypted_blob_id should be a real blob ID"
        )

    def test_multiple_duplicates_all_encrypted_inline(self, user_client, admin_client):
        """All duplicates should have encrypted_blob_id set immediately."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy_ids = []
        for _ in range(3):
            copy = user_client.duplicate_photo(photo_id)
            copy_ids.append(copy["id"])

        sync = user_client.encrypted_sync()
        for cid in copy_ids:
            rec = _find_photo_in_sync(sync, cid)
            assert rec is not None, f"Copy {cid} not found in sync"
            assert rec["encrypted_blob_id"] is not None, (
                f"Copy {cid}: expected encrypted_blob_id set (inline), got None"
            )
            assert rec["encrypted_blob_id"] != "", (
                f"Copy {cid}: expected real blob ID, got empty string"
            )

    def test_duplicate_with_crop_encrypted_inline(self, user_client, admin_client):
        """Duplicate with crop_metadata should also be encrypted inline."""
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
        rec = _find_photo_in_sync(sync, copy_id)
        assert rec is not None
        assert rec["encrypted_blob_id"] is not None, (
            f"Cropped copy: expected encrypted_blob_id set (inline), got None"
        )

    def test_no_pending_after_duplicate(self, user_client, admin_client):
        """After duplicating an already-encrypted photo, no new pending
        items should appear — the banner would not trigger."""
        photo = user_client.upload_photo(unique_filename())
        photo_id = photo["photo_id"]

        _wait_for_photo_encrypted(user_client, photo_id, timeout=30)

        copy = user_client.duplicate_photo(photo_id)
        copy_id = copy["id"]

        # The copy should already be encrypted
        sync = user_client.encrypted_sync()
        copy_record = _find_photo_in_sync(sync, copy_id)
        assert copy_record is not None
        assert copy_record["encrypted_blob_id"] is not None, (
            "Copy should be encrypted inline, not pending"
        )


# ── Helpers ──────────────────────────────────────────────────────────

def _find_photo_in_sync(sync_response: dict, photo_id: str) -> dict | None:
    """Find a photo record in the encrypted-sync response by ID."""
    for p in sync_response.get("photos", []):
        if p["id"] == photo_id:
            return p
    return None


def _wait_for_photo_encrypted(client: APIClient, photo_id: str,
                              timeout: float = 30.0):
    """Poll encrypted-sync until the given photo has a non-empty encrypted_blob_id."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        sync = client.encrypted_sync()
        rec = _find_photo_in_sync(sync, photo_id)
        if rec and rec.get("encrypted_blob_id") and rec["encrypted_blob_id"] != "":
            return
        time.sleep(1.0)
    # Don't fail here — the caller's assertions will catch it
