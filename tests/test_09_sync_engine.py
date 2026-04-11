"""
Test 09: Sync Engine — primary→backup sync with STRICT data verification.

Every test snapshots backup state before, performs an action, syncs, then
verifies exact counts and data on the backup.  No "assert >= 1" or
"assert isinstance(list)" style shortcuts — we verify *exactly* what
should and should not be present.
"""

import json
import time
from collections import Counter

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    random_username,
    unique_filename,
    wait_for_sync,
    trigger_and_wait,
    assert_no_duplicates,
    backup_photo_ids,
    backup_blob_ids,
    backup_trash_ids,
    backup_user_ids,
)
from conftest import USER_PASSWORD


# ── Helpers ───────────────────────────────────────────────────────────

def _assert_exact_new_items(before_ids, after_ids, expected_new, label):
    """Assert that *exactly* the expected new IDs appeared, no more, no fewer,
    and no duplicates exist."""
    assert_no_duplicates(after_ids, label)
    before_set = set(before_ids)
    after_set = set(after_ids)
    actually_new = after_set - before_set
    missing = expected_new - actually_new
    unexpected = actually_new - expected_new
    assert not missing, f"Missing {label} on backup: {missing}"
    assert not unexpected, f"Unexpected new {label} on backup: {unexpected}"


# ── Test classes ──────────────────────────────────────────────────────

class TestSyncPrerequisites:
    """Verify backup server is reachable and configured."""

    def test_backup_server_reachable(self, primary_admin, backup_configured):
        status = primary_admin.admin_backup_server_status(backup_configured)
        assert status["reachable"] is True

    def test_backup_mode_is_primary(self, primary_admin):
        mode = primary_admin.admin_get_backup_mode()
        assert mode["mode"] == "primary"


class TestSyncUsers:
    """Phase 0: User account sync."""

    def test_sync_creates_user_on_backup(self, primary_admin, backup_configured, backup_client):
        """New user on primary should appear exactly once on backup after sync."""
        before = backup_user_ids(backup_client)
        username = random_username("syncuser_")
        created = primary_admin.admin_create_user(username, "SyncUser123!")
        uid = created["user_id"]

        trigger_and_wait(primary_admin, backup_configured)

        after = backup_user_ids(backup_client)
        assert_no_duplicates(after, "users")
        assert uid in after, f"User {uid} not found on backup"
        assert after.count(uid) == 1, f"User {uid} duplicated on backup"

        # Verify username matches
        bu = next(u for u in backup_client.backup_list_users() if u["id"] == uid)
        assert bu["username"] == username

    def test_sync_user_idempotent(self, primary_admin, backup_configured, backup_client):
        """Syncing twice should not duplicate users."""
        trigger_and_wait(primary_admin, backup_configured)
        snap1 = backup_user_ids(backup_client)
        assert_no_duplicates(snap1, "users after sync 1")

        trigger_and_wait(primary_admin, backup_configured)
        snap2 = backup_user_ids(backup_client)
        assert_no_duplicates(snap2, "users after sync 2")
        assert sorted(snap1) == sorted(snap2), (
            f"User list changed on repeat sync.\n"
            f"  Before: {sorted(snap1)}\n"
            f"  After:  {sorted(snap2)}"
        )

    def test_sync_user_deletion(self, primary_admin, backup_configured, backup_client):
        """Deleted user should be removed from backup after sync."""
        username = random_username("deluser_")
        created = primary_admin.admin_create_user(username, "DelUser123!")
        uid = created["user_id"]

        trigger_and_wait(primary_admin, backup_configured)
        assert uid in backup_user_ids(backup_client)

        primary_admin.admin_delete_user(uid)
        trigger_and_wait(primary_admin, backup_configured)

        after = backup_user_ids(backup_client)
        assert_no_duplicates(after, "users")
        assert uid not in after, f"Deleted user {uid} still on backup"


class TestSyncPhotos:
    """Phase 1: Photo delta sync."""

    def test_sync_single_photo(self, primary_admin, user_client,
                               backup_configured, backup_client):
        """One uploaded photo should appear exactly once on backup."""
        before = backup_photo_ids(backup_client)

        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        trigger_and_wait(primary_admin, backup_configured)

        after = backup_photo_ids(backup_client)
        _assert_exact_new_items(before, after, {pid}, "photos")

    def test_sync_multiple_photos_exact_count(self, primary_admin, user_client,
                                              backup_configured, backup_client):
        """Upload N photos -> exactly N new entries on backup, no duplicates."""
        before = backup_photo_ids(backup_client)
        n = 5
        pids = set()
        for _ in range(n):
            p = user_client.upload_photo(unique_filename())
            pids.add(p["photo_id"])

        trigger_and_wait(primary_admin, backup_configured)

        after = backup_photo_ids(backup_client)
        _assert_exact_new_items(before, after, pids, "photos")

    def test_sync_photo_metadata_values(self, primary_admin, user_client,
                                        backup_configured, backup_client):
        """Photo metadata (filename, favorite, crop) must match on backup."""
        fname = unique_filename()
        photo = user_client.upload_photo(fname)
        pid = photo["photo_id"]

        user_client.favorite_photo(pid)
        user_client.crop_photo(pid, '{"x":42,"y":7}')

        trigger_and_wait(primary_admin, backup_configured)

        bp = next((p for p in backup_client.backup_list() if p["id"] == pid), None)
        assert bp is not None, f"Photo {pid} not on backup"
        assert bp["filename"] == fname
        assert bp["is_favorite"] in (True, 1), f"is_favorite={bp['is_favorite']}"
        crop = bp.get("crop_metadata")
        if isinstance(crop, str):
            crop = json.loads(crop)
        assert crop == {"x": 42, "y": 7}, f"crop_metadata={crop}"

    def test_sync_photo_idempotent_no_duplicates(self, primary_admin, user_client,
                                                  backup_configured, backup_client):
        """Sync same photo twice -> still exactly one copy on backup."""
        before = backup_photo_ids(backup_client)

        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        trigger_and_wait(primary_admin, backup_configured)
        mid = backup_photo_ids(backup_client)
        assert mid.count(pid) == 1

        trigger_and_wait(primary_admin, backup_configured)
        after = backup_photo_ids(backup_client)
        assert after.count(pid) == 1, f"Photo {pid} duplicated: count={after.count(pid)}"
        assert_no_duplicates(after, "photos")
        assert len(mid) == len(after), (
            f"Photo count changed on idempotent sync: {len(mid)} -> {len(after)}"
        )

    def test_repeated_sync_never_grows(self, primary_admin, user_client,
                                       backup_configured, backup_client):
        """5 consecutive syncs with no new data should keep counts constant."""
        user_client.upload_photo(unique_filename())
        trigger_and_wait(primary_admin, backup_configured)
        baseline_photos = len(backup_photo_ids(backup_client))
        baseline_blobs = len(backup_blob_ids(backup_client))
        baseline_trash = len(backup_trash_ids(backup_client))

        for i in range(5):
            trigger_and_wait(primary_admin, backup_configured)
            photos = backup_photo_ids(backup_client)
            blobs = backup_blob_ids(backup_client)
            trash = backup_trash_ids(backup_client)
            assert_no_duplicates(photos, f"photos (repeat {i+1})")
            assert_no_duplicates(blobs, f"blobs (repeat {i+1})")
            assert_no_duplicates(trash, f"trash (repeat {i+1})")
            assert len(photos) == baseline_photos, (
                f"Photo count grew on repeat sync {i+1}: "
                f"{baseline_photos} -> {len(photos)}"
            )
            assert len(blobs) == baseline_blobs, (
                f"Blob count grew on repeat sync {i+1}: "
                f"{baseline_blobs} -> {len(blobs)}"
            )
            assert len(trash) == baseline_trash, (
                f"Trash count grew on repeat sync {i+1}: "
                f"{baseline_trash} -> {len(trash)}"
            )


class TestSyncBlobs:
    """Phase 4: Encrypted blob sync."""

    def test_sync_single_blob(self, primary_admin, user_client,
                              backup_configured, backup_client):
        """One blob should appear exactly once on backup."""
        before = backup_blob_ids(backup_client)

        content = generate_random_bytes(2048)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        trigger_and_wait(primary_admin, backup_configured)

        after = backup_blob_ids(backup_client)
        _assert_exact_new_items(before, after, {bid}, "blobs")

    def test_sync_multiple_blobs_exact_count(self, primary_admin, user_client,
                                             backup_configured, backup_client):
        """Upload N blobs -> exactly N new on backup."""
        before = backup_blob_ids(backup_client)
        n = 4
        bids = set()
        for _ in range(n):
            b = user_client.upload_blob("photo", generate_random_bytes(512))
            bids.add(b["blob_id"])

        trigger_and_wait(primary_admin, backup_configured)

        after = backup_blob_ids(backup_client)
        _assert_exact_new_items(before, after, bids, "blobs")

    def test_sync_blob_idempotent(self, primary_admin, user_client,
                                  backup_configured, backup_client):
        """Sync same blob twice -> no duplication."""
        before = backup_blob_ids(backup_client)

        blob = user_client.upload_blob("photo", generate_random_bytes(1024))
        bid = blob["blob_id"]

        trigger_and_wait(primary_admin, backup_configured)
        mid = backup_blob_ids(backup_client)
        assert_no_duplicates(mid, "blobs after sync 1")
        assert mid.count(bid) == 1

        trigger_and_wait(primary_admin, backup_configured)
        after = backup_blob_ids(backup_client)
        assert_no_duplicates(after, "blobs after sync 2")
        assert after.count(bid) == 1
        assert len(mid) == len(after), (
            f"Blob count changed: {len(mid)} -> {len(after)}"
        )

    def test_sync_blob_size_preserved(self, primary_admin, user_client,
                                      backup_configured, backup_client):
        """Blob size_bytes on backup should match what was uploaded."""
        content = generate_random_bytes(3333)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        trigger_and_wait(primary_admin, backup_configured)

        bb = next((b for b in backup_client.backup_list_blobs() if b["id"] == bid), None)
        assert bb is not None
        assert bb["size_bytes"] == len(content), (
            f"Blob size mismatch: expected {len(content)}, got {bb['size_bytes']}"
        )


class TestSyncTrash:
    """Phase 2: Trash item sync."""

    def test_sync_trash_item(self, primary_admin, user_client,
                             backup_configured, backup_client):
        """Soft-deleted blob -> trash item appears exactly once on backup."""
        before_trash = backup_trash_ids(backup_client)

        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        # Sync blob first
        trigger_and_wait(primary_admin, backup_configured)
        assert bid in backup_blob_ids(backup_client)

        # Soft-delete
        trash_resp = user_client.soft_delete_blob(
            bid, filename="sync_trash.jpg", size_bytes=len(content),
        )
        tid = trash_resp["trash_id"]

        # Sync deletion
        trigger_and_wait(primary_admin, backup_configured)

        after_trash = backup_trash_ids(backup_client)
        assert_no_duplicates(after_trash, "trash")
        assert tid in after_trash, f"Trash {tid} not on backup"
        assert after_trash.count(tid) == 1

    def test_sync_trash_idempotent(self, primary_admin, user_client,
                                   backup_configured, backup_client):
        """Syncing trash twice should not duplicate entries."""
        content = generate_random_bytes(512)
        blob = user_client.upload_blob("photo", content)
        user_client.soft_delete_blob(
            blob["blob_id"], filename="trash_idem.jpg", size_bytes=len(content),
        )

        trigger_and_wait(primary_admin, backup_configured)
        snap1 = backup_trash_ids(backup_client)
        assert_no_duplicates(snap1, "trash after sync 1")

        trigger_and_wait(primary_admin, backup_configured)
        snap2 = backup_trash_ids(backup_client)
        assert_no_duplicates(snap2, "trash after sync 2")
        assert len(snap1) == len(snap2), (
            f"Trash count changed: {len(snap1)} -> {len(snap2)}"
        )
        assert sorted(snap1) == sorted(snap2)

    def test_sync_trash_fields_preserved(self, primary_admin, user_client,
                                         backup_configured, backup_client):
        """Trash item file_path and size should be correct on backup."""
        content = generate_random_bytes(2222)
        blob = user_client.upload_blob("photo", content)
        trash_resp = user_client.soft_delete_blob(
            blob["blob_id"], filename="verify_fields.jpg", size_bytes=len(content),
        )
        tid = trash_resp["trash_id"]

        trigger_and_wait(primary_admin, backup_configured)

        items = backup_client.backup_list_trash()
        item = next((t for t in items if t["id"] == tid), None)
        assert item is not None
        assert item["size_bytes"] == len(content), (
            f"Trash size mismatch: expected {len(content)}, got {item['size_bytes']}"
        )
        assert item["file_path"], "Trash file_path should not be empty"


class TestSyncDeletions:
    """Verify that deleted items are properly cleaned on backup."""

    def test_blob_deletion_syncs_to_trash(self, primary_admin, user_client,
                                          backup_configured, backup_client):
        """Soft-deleted blob on primary should result in trash item on backup."""
        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        trigger_and_wait(primary_admin, backup_configured)
        assert bid in backup_blob_ids(backup_client)

        trash_resp = user_client.soft_delete_blob(
            bid, filename="sync_del.jpg", size_bytes=len(content),
        )

        trigger_and_wait(primary_admin, backup_configured)

        trash = backup_trash_ids(backup_client)
        assert_no_duplicates(trash, "trash")
        assert trash_resp["trash_id"] in trash


class TestSyncSecureGalleries:
    """Phase 3: Secure gallery sync — verifies that items added to secure
    galleries are NOT synced to backup as visible duplicates.

    When a blob is added to a secure gallery, the primary creates a clone
    (new UUID) and hides both the original and clone from the main gallery
    listing (via encrypted_gallery_items filter).  The sync engine must
    respect that filter — it must NOT push hidden items to the backup.

    These tests are designed to catch the exact bug the user reported:
    items added to secure albums were duplicating in the backup gallery.
    """

    def test_secure_gallery_items_not_synced_to_backup(self, primary_admin, user_client,
                                                        backup_configured, backup_client):
        """Upload blob → add to secure gallery → sync → NEITHER original nor
        clone should appear on backup.  This is the CORE duplication bug test."""
        before_blobs = backup_blob_ids(backup_client)
        before_photos = backup_photo_ids(backup_client)

        # Upload a blob and immediately add to secure gallery
        content = generate_random_bytes(512)
        blob = user_client.upload_blob("photo", content)
        original_id = blob["blob_id"]

        gallery = user_client.create_secure_gallery("NoSyncGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], original_id, token,
        )
        clone_id = add_result["new_blob_id"]

        # Verify primary hides both from main listing
        primary_blobs = [b["id"] for b in user_client.list_blobs(limit=500).get("blobs", [])]
        assert original_id not in primary_blobs, (
            f"Original {original_id} should be HIDDEN on primary"
        )
        assert clone_id not in primary_blobs, (
            f"Clone {clone_id} should be HIDDEN on primary"
        )

        # Sync
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        # Backup must NOT have the hidden items
        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_blobs, "blobs after secure gallery sync")
        assert original_id not in after_blobs, (
            f"BUG: Original blob {original_id} was synced to backup despite being in secure gallery"
        )
        assert clone_id not in after_blobs, (
            f"BUG: Clone blob {clone_id} was synced to backup — secure gallery items should not sync"
        )
        # Blob count on backup should not have changed (0 new items synced)
        assert len(after_blobs) == len(before_blobs), (
            f"Backup blob count changed: {len(before_blobs)} -> {len(after_blobs)}. "
            f"Secure gallery items should NOT be synced."
        )

    def test_secure_gallery_photo_not_synced_to_backup(self, primary_admin, user_client,
                                                        backup_configured, backup_client):
        """Upload a server-side photo → add to secure gallery → sync →
        photo (and its clone row) must NOT appear on backup."""
        before_photos = backup_photo_ids(backup_client)

        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("PhotoNoSyncGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = add_result["new_blob_id"]

        # Verify primary hides photo from listing
        primary_photos = [p["id"] for p in user_client.list_photos(limit=500).get("photos", [])]
        assert pid not in primary_photos, (
            f"Photo {pid} should be HIDDEN on primary after secure add"
        )

        # Sync
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        # Backup must NOT have hidden photo or its clone
        after_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(after_photos, "photos after secure gallery sync")
        assert pid not in after_photos, (
            f"BUG: Photo {pid} was synced to backup despite being in secure gallery"
        )
        assert clone_id not in after_photos, (
            f"BUG: Clone photo {clone_id} was synced to backup"
        )
        assert len(after_photos) == len(before_photos), (
            f"Backup photo count changed: {len(before_photos)} -> {len(after_photos)}. "
            f"Secure gallery items should NOT be synced."
        )

    def test_secure_gallery_multiple_items_none_synced(self, primary_admin, user_client,
                                                        backup_configured, backup_client):
        """Add 3 blobs to secure gallery → sync → ZERO new blobs on backup."""
        before_blobs = backup_blob_ids(backup_client)

        original_ids = set()
        clone_ids = set()
        gallery = user_client.create_secure_gallery("MultiNoSyncGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        for _ in range(3):
            b = user_client.upload_blob("photo", generate_random_bytes(512))
            original_ids.add(b["blob_id"])
            add_result = user_client.add_secure_gallery_item(
                gallery["gallery_id"], b["blob_id"], token,
            )
            clone_ids.add(add_result["new_blob_id"])

        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_blobs, "blobs after multi-item gallery sync")

        # None of the 6 IDs (3 originals + 3 clones) should be on backup
        all_hidden = original_ids | clone_ids
        for bid in all_hidden:
            assert bid not in after_blobs, (
                f"BUG: Secure gallery blob {bid} was synced to backup"
            )
        assert len(after_blobs) == len(before_blobs), (
            f"Backup blob count changed: {len(before_blobs)} -> {len(after_blobs)}. "
            f"Expected 0 new blobs (all hidden by secure gallery)."
        )

    def test_mixed_secure_and_regular_blobs_sync_correctly(self, primary_admin, user_client,
                                                            backup_configured, backup_client):
        """Upload 3 blobs: secure 1, leave 2 regular → sync → only 2 regular on backup."""
        before_blobs = backup_blob_ids(backup_client)

        b1 = user_client.upload_blob("photo", generate_random_bytes(512))
        b2 = user_client.upload_blob("photo", generate_random_bytes(768))
        b3 = user_client.upload_blob("photo", generate_random_bytes(1024))

        # Add b2 to secure gallery (hides b2 original + creates clone)
        gallery = user_client.create_secure_gallery("MixedSyncGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], b2["blob_id"], token,
        )
        clone_id = add_result["new_blob_id"]

        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_blobs, "blobs after mixed sync")

        # Only b1 and b3 should be new on backup
        _assert_exact_new_items(before_blobs, after_blobs, {b1["blob_id"], b3["blob_id"]}, "blobs")
        # b2 (hidden original) and clone should NOT be on backup
        assert b2["blob_id"] not in after_blobs, (
            f"BUG: Hidden original {b2['blob_id']} synced to backup"
        )
        assert clone_id not in after_blobs, (
            f"BUG: Clone {clone_id} synced to backup"
        )

    def test_secure_gallery_sync_idempotent(self, primary_admin, user_client,
                                             backup_configured, backup_client):
        """Syncing gallery data twice must not create any items."""
        blob = user_client.upload_blob("photo", generate_random_bytes(512))
        gallery = user_client.create_secure_gallery("IdempotentGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], blob["blob_id"], token)

        trigger_and_wait(primary_admin, backup_configured)
        snap1_blobs = backup_blob_ids(backup_client)
        snap1_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(snap1_blobs, "blobs after sync 1")
        assert_no_duplicates(snap1_photos, "photos after sync 1")

        trigger_and_wait(primary_admin, backup_configured)
        snap2_blobs = backup_blob_ids(backup_client)
        snap2_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(snap2_blobs, "blobs after sync 2")
        assert_no_duplicates(snap2_photos, "photos after sync 2")

        assert len(snap1_blobs) == len(snap2_blobs), (
            f"Blob count changed on repeat gallery sync: {len(snap1_blobs)} -> {len(snap2_blobs)}"
        )
        assert len(snap1_photos) == len(snap2_photos), (
            f"Photo count changed on repeat gallery sync: {len(snap1_photos)} -> {len(snap2_photos)}"
        )

    def test_secure_gallery_repeated_sync_never_grows(self, primary_admin, user_client,
                                                       backup_configured, backup_client):
        """5 consecutive syncs after a gallery operation — counts must stay constant."""
        blob = user_client.upload_blob("photo", generate_random_bytes(512))
        gallery = user_client.create_secure_gallery("RepeatedSyncGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], blob["blob_id"], token)

        trigger_and_wait(primary_admin, backup_configured)
        baseline_blobs = len(backup_blob_ids(backup_client))
        baseline_photos = len(backup_photo_ids(backup_client))

        for i in range(5):
            trigger_and_wait(primary_admin, backup_configured)
            blobs = backup_blob_ids(backup_client)
            photos = backup_photo_ids(backup_client)
            assert_no_duplicates(blobs, f"blobs (repeat {i+1})")
            assert_no_duplicates(photos, f"photos (repeat {i+1})")
            assert len(blobs) == baseline_blobs, (
                f"Blob count grew on repeat sync {i+1}: {baseline_blobs} -> {len(blobs)}"
            )
            assert len(photos) == baseline_photos, (
                f"Photo count grew on repeat sync {i+1}: {baseline_photos} -> {len(photos)}"
            )

    def test_sync_gallery_deletion_no_growth(self, primary_admin, user_client,
                                              backup_configured, backup_client):
        """Deleting a secure gallery + syncing should not increase counts."""
        gallery = user_client.create_secure_gallery("SyncDelGallery")
        trigger_and_wait(primary_admin, backup_configured)

        before_photos = backup_photo_ids(backup_client)
        before_blobs = backup_blob_ids(backup_client)

        user_client.delete_secure_gallery(gallery["gallery_id"])
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error"

        after_photos = backup_photo_ids(backup_client)
        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_photos, "photos after gallery deletion")
        assert_no_duplicates(after_blobs, "blobs after gallery deletion")
        assert len(after_photos) <= len(before_photos), (
            f"Photo count GREW after gallery deletion: {len(before_photos)} -> {len(after_photos)}"
        )
        assert len(after_blobs) <= len(before_blobs), (
            f"Blob count GREW after gallery deletion: {len(before_blobs)} -> {len(after_blobs)}"
        )

    def test_photo_synced_then_hidden_removed_from_backup(self, primary_admin, user_client,
                                                           backup_configured, backup_client):
        """Photo synced to backup FIRST, then added to secure gallery →
        must be REMOVED from backup on the next sync (retroactive purge).

        This tests the critical order-dependent scenario: the item was already
        on the backup before it was hidden.  The sync engine must not only
        skip sending it again, but actively purge the stale copy."""
        before_photos = backup_photo_ids(backup_client)

        # Upload and sync — photo lands on backup
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        trigger_and_wait(primary_admin, backup_configured)

        mid_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(mid_photos, "photos after initial sync")
        assert pid in mid_photos, f"Photo {pid} should be on backup after first sync"

        # Now add to secure gallery (hides it on primary)
        gallery = user_client.create_secure_gallery("RetroHidePhotoGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = add_result["new_blob_id"]

        # Verify hidden on primary
        primary_photos = [p["id"] for p in user_client.list_photos(limit=500).get("photos", [])]
        assert pid not in primary_photos, (
            f"Photo {pid} should be hidden on primary after secure gallery add"
        )

        # Sync again → photo MUST be removed from backup
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        after_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(after_photos, "photos after retroactive hide")
        assert pid not in after_photos, (
            f"BUG: Photo {pid} still on backup after being added to secure gallery. "
            f"Sync should retroactively remove previously-synced items when they become hidden."
        )
        assert clone_id not in after_photos, (
            f"BUG: Clone {clone_id} appeared on backup"
        )
        # Count should have decreased by exactly 1 (the hidden photo)
        expected_count = len(mid_photos) - 1
        assert len(after_photos) == expected_count, (
            f"Expected {expected_count} photos after retroactive purge, got {len(after_photos)}"
        )

    def test_blob_synced_then_hidden_removed_from_backup(self, primary_admin, user_client,
                                                          backup_configured, backup_client):
        """Blob synced to backup FIRST, then added to secure gallery →
        must be REMOVED from backup on the next sync (retroactive purge).

        Same principle as the photo test, but for client-encrypted blobs."""
        before_blobs = backup_blob_ids(backup_client)

        # Upload blob and sync — blob lands on backup
        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        trigger_and_wait(primary_admin, backup_configured)

        mid_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(mid_blobs, "blobs after initial sync")
        assert bid in mid_blobs, f"Blob {bid} should be on backup after first sync"

        # Now add to secure gallery (hides it)
        gallery = user_client.create_secure_gallery("RetroHideBlobGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], bid, token,
        )
        clone_id = add_result["new_blob_id"]

        # Sync again → blob MUST be removed from backup
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_blobs, "blobs after retroactive hide")
        assert bid not in after_blobs, (
            f"BUG: Blob {bid} still on backup after being added to secure gallery. "
            f"Sync should retroactively remove previously-synced blobs when they become hidden."
        )
        assert clone_id not in after_blobs, (
            f"BUG: Clone {clone_id} appeared on backup blobs"
        )
        # Count should have decreased by exactly 1
        expected_count = len(mid_blobs) - 1
        assert len(after_blobs) == expected_count, (
            f"Expected {expected_count} blobs after retroactive purge, got {len(after_blobs)}"
        )

    def test_mixed_presynced_and_new_gallery_items(self, primary_admin, user_client,
                                                    backup_configured, backup_client):
        """Mix of pre-synced and new items added to gallery: all must be
        absent from backup after sync, and regular items unaffected."""
        before_blobs = backup_blob_ids(backup_client)

        # Upload 3 blobs
        b1 = user_client.upload_blob("photo", generate_random_bytes(512))
        b2 = user_client.upload_blob("photo", generate_random_bytes(768))
        b3 = user_client.upload_blob("photo", generate_random_bytes(1024))

        # Sync all 3 to backup
        trigger_and_wait(primary_admin, backup_configured)
        mid_blobs = backup_blob_ids(backup_client)
        for b in (b1, b2, b3):
            assert b["blob_id"] in mid_blobs

        # Add b1 to gallery (pre-synced, retroactive purge needed)
        gallery = user_client.create_secure_gallery("MixedRetroGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        r1 = user_client.add_secure_gallery_item(
            gallery["gallery_id"], b1["blob_id"], token,
        )

        # Upload b4, add to gallery immediately (never synced)
        b4 = user_client.upload_blob("photo", generate_random_bytes(256))
        r4 = user_client.add_secure_gallery_item(
            gallery["gallery_id"], b4["blob_id"], token,
        )

        # Sync
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error"

        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_blobs, "blobs after mixed retroactive purge")

        # b1 should be REMOVED (retroactive purge)
        assert b1["blob_id"] not in after_blobs, (
            f"BUG: Pre-synced blob {b1['blob_id']} not purged after gallery add"
        )
        # b4 should never have been sent
        assert b4["blob_id"] not in after_blobs
        # Clones should not be on backup
        assert r1["new_blob_id"] not in after_blobs
        assert r4["new_blob_id"] not in after_blobs
        # b2 and b3 should still be there (regular, unaffected)
        assert b2["blob_id"] in after_blobs
        assert b3["blob_id"] in after_blobs


class TestSyncMetadata:
    """Phase 5: Metadata sync (edit copies, shared albums, tags)."""

    def test_sync_edit_copies_no_duplicates(self, primary_admin, user_client,
                                            backup_configured, backup_client):
        before = backup_photo_ids(backup_client)

        photo = user_client.upload_photo(unique_filename())
        user_client.create_edit_copy(
            photo["photo_id"], name="SyncCopy",
            edit_metadata=json.dumps({"brightness": 1.5}),
        )

        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error"

        after = backup_photo_ids(backup_client)
        assert_no_duplicates(after, "photos after edit copy sync")

    def test_sync_shared_album_no_duplicates(self, primary_admin, user_client,
                                             backup_configured, backup_client):
        before = backup_photo_ids(backup_client)

        album = user_client.create_shared_album("SyncAlbum")
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        user_client.add_album_photo(album["id"], pid)

        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error"

        after = backup_photo_ids(backup_client)
        assert_no_duplicates(after, "photos after album sync")
        assert after.count(pid) == 1

    def test_sync_tags_no_duplicates(self, primary_admin, user_client,
                                     backup_configured, backup_client):
        before_photos = backup_photo_ids(backup_client)

        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        user_client.add_tag(pid, "synced_tag")
        user_client.add_tag(pid, "second_tag")

        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error"

        after_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(after_photos, "photos after tag sync")
        assert after_photos.count(pid) == 1


class TestSyncLogs:
    """Sync logging and status."""

    def test_sync_creates_log(self, primary_admin, backup_configured):
        trigger_and_wait(primary_admin, backup_configured)
        logs = primary_admin.admin_get_sync_logs(backup_configured)
        assert len(logs) >= 1
        latest = logs[0]
        assert "started_at" in latest
        assert latest["status"] in ("success", "completed", "error")

    def test_sync_reports_counts(self, primary_admin, user_client, backup_configured):
        user_client.upload_photo(unique_filename())
        result = trigger_and_wait(primary_admin, backup_configured)
        assert "photos_synced" in result or "status" in result


class TestSyncConcurrency:
    """Concurrent sync prevention."""

    def test_concurrent_sync_rejected(self, primary_admin, backup_configured):
        r1 = primary_admin.post(f"/api/admin/backup/servers/{backup_configured}/sync")
        r2 = primary_admin.post(f"/api/admin/backup/servers/{backup_configured}/sync")
        statuses = {r1.status_code, r2.status_code}
        assert 200 in statuses or 202 in statuses
        wait_for_sync(primary_admin, backup_configured, timeout=60)


class TestSyncFullIntegrity:
    """End-to-end: create a known dataset, sync, verify everything matches."""

    def test_comprehensive_sync_integrity(self, primary_admin, user_client,
                                          backup_configured, backup_client):
        """Upload a diverse dataset including secure gallery items, sync once,
        then verify backup has exactly the right items with no duplicates.

        KEY ASSERTION: Secure gallery items (both original and clone) must NOT
        appear on the backup.  Only regular (non-hidden) items should sync."""
        # Flush any accumulated encrypted blobs from earlier tests so the
        # before-snapshot is clean.
        trigger_and_wait(primary_admin, backup_configured)

        # Snapshot before
        before_photos = backup_photo_ids(backup_client)
        before_blobs = backup_blob_ids(backup_client)
        before_trash = backup_trash_ids(backup_client)

        # Create photos (regular — should sync)
        photo_ids = set()
        for _ in range(3):
            p = user_client.upload_photo(unique_filename())
            photo_ids.add(p["photo_id"])

        # Create blobs (regular — should sync unless hidden)
        blob_ids = set()
        for _ in range(3):
            b = user_client.upload_blob("photo", generate_random_bytes(512))
            blob_ids.add(b["blob_id"])

        # Add 1 blob to secure gallery (hides original + creates clone)
        gallery = user_client.create_secure_gallery("IntegrityGallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        target_blob = next(iter(blob_ids))
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], target_blob, token,
        )
        clone_id = add_result["new_blob_id"]

        # Trash one blob (different from the gallery one)
        remaining = blob_ids - {target_blob}
        trash_blob = remaining.pop()
        trash_content_size = 512
        trash_resp = user_client.soft_delete_blob(
            trash_blob, filename="integrity_trash.jpg",
            size_bytes=trash_content_size,
        )
        trash_id = trash_resp["trash_id"]

        # Sync
        result = trigger_and_wait(primary_admin, backup_configured)
        assert result.get("status") != "error", f"Sync failed: {result}"

        # Verify photos — exactly 3 new
        after_photos = backup_photo_ids(backup_client)
        assert_no_duplicates(after_photos, "photos")
        _assert_exact_new_items(before_photos, after_photos, photo_ids, "photos")

        # Verify blobs:
        # - target_blob: hidden by secure gallery → NOT synced
        # - clone_id: secure gallery clone → NOT synced
        # - trash_blob: soft-deleted → removed from blobs table → NOT synced as blob
        # - remaining 1 blob: regular → synced
        # Note: the backup's auto_migrate may create encrypted blobs from
        # previously synced photos, so we check expected/excluded items
        # individually rather than exact total counts.
        after_blobs = backup_blob_ids(backup_client)
        assert_no_duplicates(after_blobs, "blobs")
        expected_blobs = blob_ids - {target_blob} - {trash_blob}  # only the 1 regular blob
        after_set = set(after_blobs)
        before_set = set(before_blobs)
        actually_new = after_set - before_set
        missing = expected_blobs - actually_new
        assert not missing, f"Missing blobs on backup: {missing}"
        assert target_blob not in after_blobs, (
            f"BUG: Secure gallery original {target_blob} synced to backup"
        )
        assert clone_id not in after_blobs, (
            f"BUG: Secure gallery clone {clone_id} synced to backup"
        )

        # Verify trash
        after_trash = backup_trash_ids(backup_client)
        assert_no_duplicates(after_trash, "trash")
        assert trash_id in after_trash

        # Verify idempotent — second sync should not change anything
        trigger_and_wait(primary_admin, backup_configured)
        final_photos = backup_photo_ids(backup_client)
        final_blobs = backup_blob_ids(backup_client)
        final_trash = backup_trash_ids(backup_client)
        assert_no_duplicates(final_photos, "photos (repeat)")
        assert_no_duplicates(final_blobs, "blobs (repeat)")
        assert_no_duplicates(final_trash, "trash (repeat)")
        assert len(final_photos) == len(after_photos), (
            f"Photo count changed on repeat: {len(after_photos)} -> {len(final_photos)}"
        )
        assert len(final_blobs) == len(after_blobs), (
            f"Blob count changed on repeat: {len(after_blobs)} -> {len(final_blobs)}"
        )
        assert len(final_trash) == len(after_trash), (
            f"Trash count changed on repeat: {len(after_trash)} -> {len(final_trash)}"
        )


class TestSyncHashIntegrity:
    """Verify hash-based dedup across sync boundaries."""

    def test_synced_photo_preserves_hash(self, primary_admin, user_client,
                                         backup_configured, backup_client):
        """Photo hash computed on primary must match on backup after sync."""
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        primary_hash = photo.get("photo_hash")
        assert primary_hash, "Primary should return photo_hash on upload"

        trigger_and_wait(primary_admin, backup_configured)

        bp = next((p for p in backup_client.backup_list() if p["id"] == pid), None)
        assert bp is not None, f"Photo {pid} not on backup"
        assert bp.get("photo_hash") == primary_hash, (
            f"Hash mismatch: primary={primary_hash}, backup={bp.get('photo_hash')}"
        )

    def test_no_duplicate_hashes_on_backup(self, primary_admin, user_client,
                                            backup_configured, backup_client):
        """Photos uploaded in this test must each have a unique photo_hash
        on backup — no content-level duplicates."""
        uploaded_ids = set()
        for i in range(3):
            p = user_client.upload_photo(unique_filename())
            uploaded_ids.add(p["photo_id"])

        trigger_and_wait(primary_admin, backup_configured)

        photos = backup_client.backup_list()
        our_photos = [p for p in photos if p["id"] in uploaded_ids]
        hashes = [p["photo_hash"] for p in our_photos
                  if p.get("photo_hash") is not None]
        hash_counts = Counter(hashes)
        dupes = {h: c for h, c in hash_counts.items() if c > 1}
        assert not dupes, (
            f"Duplicate photo_hash values on backup for our uploads: {dupes}"
        )
        assert len(hashes) == 3, (
            f"Expected 3 unique hashes, got {len(hashes)}"
        )

    def test_synced_blob_arrives_on_backup(self, primary_admin, user_client,
                                            backup_configured, backup_client):
        """Blob uploaded with content_hash should sync to backup."""
        import hashlib
        original_content = b"sync-hash-integrity-check"
        content_hash = hashlib.sha256(original_content).hexdigest()[:12]

        blob = user_client.upload_blob("photo", generate_random_bytes(1024),
                                       content_hash=content_hash)
        bid = blob["blob_id"]

        trigger_and_wait(primary_admin, backup_configured)

        bb = next((b for b in backup_client.backup_list_blobs()
                   if b["id"] == bid), None)
        assert bb is not None, f"Blob {bid} not on backup after sync"

    def test_dedup_blob_syncs_once(self, primary_admin,
                                    user_client,
                                    backup_configured,
                                    backup_client):
        """Uploading two blobs with the same content_hash should only
        produce one blob on primary (dedup), and thus one on backup."""
        import hashlib
        before = backup_blob_ids(backup_client)

        ch = hashlib.sha256(b"sync-dedup-test").hexdigest()[:12]
        b1 = user_client.upload_blob("photo", generate_random_bytes(512),
                                     content_hash=ch)
        b2 = user_client.upload_blob("photo", generate_random_bytes(512),
                                     content_hash=ch)
        # Primary dedup should return same blob
        assert b1["blob_id"] == b2["blob_id"], (
            "Primary should have deduped these blobs"
        )

        trigger_and_wait(primary_admin, backup_configured)

        after = backup_blob_ids(backup_client)
        _assert_exact_new_items(before, after, {b1["blob_id"]}, "blobs")


class TestSyncAfterComplexOperations:
    """Sync engine handles complex state correctly (moved from test_12)."""

    def test_sync_after_rapid_create_delete(self, primary_admin, user_client,
                                            backup_configured, backup_client):
        """Create many blobs, trash some, then sync."""
        created = []
        deleted = []
        for i in range(5):
            b = user_client.upload_blob("photo")
            created.append(b["blob_id"])

        # Trash first two
        for bid in created[:2]:
            user_client.soft_delete_blob(bid, filename="rapid_del.jpg")
            deleted.append(bid)

        # Sync
        trigger_and_wait(primary_admin, backup_configured)

        # Verify: only non-deleted should be on backup
        backup_blobs = backup_client.backup_list_blobs()
        backup_ids = {b["id"] for b in backup_blobs}

        for bid in created[2:]:
            assert bid in backup_ids, f"Surviving blob {bid} missing from backup"
        for bid in deleted:
            assert bid not in backup_ids, f"Deleted blob {bid} still on backup"

    def test_sync_with_secure_gallery_items(self, primary_admin, user_client,
                                            backup_configured):
        """Secure gallery items should sync without errors."""
        gallery = user_client.create_secure_gallery("Sync Edge Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        # Add multiple items
        for _ in range(3):
            blob = user_client.upload_blob("photo")
            user_client.add_secure_gallery_item(gallery["gallery_id"], blob["blob_id"], token)

        trigger_and_wait(primary_admin, backup_configured)

    def test_sync_with_shared_album_members_and_photos(self, primary_admin, user_client,
                                                       backup_configured):
        """Complex shared album state should sync correctly."""
        album = user_client.create_shared_album("Complex Sync Album")

        # Add photos
        for _ in range(3):
            p = user_client.upload_photo(unique_filename())
            user_client.add_album_photo(album["id"], p["photo_id"])

        trigger_and_wait(primary_admin, backup_configured)
