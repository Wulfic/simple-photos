"""Diagnostic test: Reproduce the exact 'Queued + duplicate' issue.

Replicates what the web client sees by combining encrypted-sync + list_blobs
exactly as useGalleryData.ts does. Checks BOTH primary and backup.
"""

import time
import pytest
from collections import Counter
from helpers import APIClient, generate_test_jpeg, generate_random_bytes, unique_filename
from conftest import USER_PASSWORD, TEST_ENCRYPTION_KEY


def _web_gallery_items(client, label=""):
    """Simulate what the web client's useGalleryData builds.

    Returns (sync_photos, blob_items, combined_visible, synced_blob_ids)
    where combined_visible is the set of IDB-key items that would show
    in the gallery before secureBlobIds filtering.
    """
    # Phase 1: encrypted-sync (all pages)
    sync_photos = []
    cursor = None
    while True:
        params = {"limit": 500}
        if cursor:
            params["after"] = cursor
        res = client.encrypted_sync(**params)
        sync_photos.extend(res.get("photos", []))
        cursor = res.get("next_cursor")
        if not cursor:
            break

    # Phase 2: list_blobs for all media types
    blob_items = []
    for btype in ("photo", "gif", "video", "audio"):
        page_cursor = None
        while True:
            params = {"blob_type": btype, "limit": 200}
            if page_cursor:
                params["after"] = page_cursor
            res = client.list_blobs(**params)
            blobs = res.get("blobs", [])
            blob_items.extend(blobs)
            page_cursor = res.get("next_cursor")
            if not page_cursor:
                break

    # Phase 3+4 merge: synced encrypted_blob_ids used for dedup
    synced_blob_ids = {
        p["encrypted_blob_id"]
        for p in sync_photos
        if p.get("encrypted_blob_id")
    }
    # "unsynced" blobs = those not already represented by a sync record
    unsynced_blobs = [b for b in blob_items if b["id"] not in synced_blob_ids]

    # Combined visible items (before secureBlobIds):
    # Phase 3: one entry per sync photo (keyed by photo.id)
    # Phase 4: one entry per unsynced blob (keyed by blob.id)
    combined = {}
    for p in sync_photos:
        if not p.get("encrypted_blob_id"):
            continue  # skip unencrypted (migration pending)
        combined[p["id"]] = {"source": "encrypted-sync", "type": "photo", "photo": p}
    for b in unsynced_blobs:
        combined[b["id"]] = {"source": "list_blobs", "type": b.get("blob_type", "?"), "blob": b}

    if label:
        print(f"\n[{label}] encrypted-sync: {len(sync_photos)} photos")
        print(f"[{label}] list_blobs: {len(blob_items)} blobs (all types)")
        print(f"[{label}] synced_blob_ids: {synced_blob_ids}")
        print(f"[{label}] unsynced_blobs: {len(unsynced_blobs)}")
        print(f"[{label}] combined visible: {len(combined)}")
        for k, v in combined.items():
            print(f"  {k} via {v['source']} ({v['type']})")

    return sync_photos, blob_items, combined, synced_blob_ids


def _secure_blob_id_set(client):
    """Return the set of blob IDs from secureBlobIds endpoint."""
    data = client.get_secure_gallery_blob_ids()
    return set(data.get("blob_ids", []))


def _trigger_migration(admin_client):
    """Re-store the encryption key to trigger scan + migration for new photos."""
    admin_client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)


def _wait_for_encryption(client, photo_id, max_wait=30):
    """Poll encrypted-sync until the photo has encrypted_blob_id set."""
    start = time.time()
    while time.time() - start < max_wait:
        res = client.encrypted_sync(limit=500)
        for p in res.get("photos", []):
            if p["id"] == photo_id and p.get("encrypted_blob_id"):
                return p["encrypted_blob_id"]
        time.sleep(1)
    return None


class TestDuplicateDiag:
    """Reproduce: add a photo/GIF to secure gallery, check for duplicates
    in the combined gallery view (encrypted-sync + list_blobs)."""

    def test_server_photo_no_duplicate_after_gallery_add(self, user_client, primary_admin):
        """Upload a server-side photo → add to secure gallery → wait for
        migration → combined gallery view should have ZERO items for
        this photo on primary."""
        # Upload photo
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # Trigger migration for the new photo
        _trigger_migration(primary_admin)

        # Wait for encryption migration to complete
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"Photo {pid} was not encrypted within timeout"

        # Baseline: 1 item visible
        _, _, before, _ = _web_gallery_items(user_client, "BEFORE")
        assert pid in before, f"Photo {pid} should be visible before gallery add"
        before_count = len(before)

        # Add to secure gallery
        gallery = user_client.create_secure_gallery("Dup Test Photo")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        # Wait for clone migration (can't poll encrypted-sync since it's excluded)
        time.sleep(5)

        # After: photo should be gone, no duplicates
        _, _, after, _ = _web_gallery_items(user_client, "AFTER")
        assert pid not in after, f"Original photo {pid} still visible after gallery add"

        # Overall count should have decreased by 1
        assert len(after) == before_count - 1, (
            f"Expected {before_count - 1} visible items, got {len(after)}. "
            f"Visible after: {list(after.keys())}"
        )

        # Also verify secureBlobIds covers everything
        secure_ids = _secure_blob_id_set(user_client)
        assert pid in secure_ids, f"secureBlobIds missing original {pid}"

    def test_client_blob_no_duplicate_after_gallery_add(self, user_client):
        """Upload a client-encrypted GIF blob → add to secure gallery →
        combined gallery view should have ZERO items for this GIF on primary."""
        content = generate_random_bytes(512)
        blob = user_client.upload_blob("gif", content)
        bid = blob["blob_id"]

        # Baseline
        _, _, before, _ = _web_gallery_items(user_client, "BEFORE-BLOB")
        assert bid in before, f"Blob {bid} should be visible before gallery add"
        before_count = len(before)

        # Add to secure gallery
        gallery = user_client.create_secure_gallery("Dup Test Blob")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], bid, token)

        # After: blob should be gone, no duplicates
        _, _, after, _ = _web_gallery_items(user_client, "AFTER-BLOB")
        assert bid not in after, f"Original blob {bid} still visible after gallery add"

        assert len(after) == before_count - 1, (
            f"Expected {before_count - 1} visible items, got {len(after)}. "
            f"Visible after: {list(after.keys())}"
        )

        secure_ids = _secure_blob_id_set(user_client)
        assert bid in secure_ids, f"secureBlobIds missing original blob {bid}"

    def test_backup_no_duplicate_after_gallery_sync(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """Upload photo → add to gallery → sync → backup gallery should
        have zero items in regular view, no duplicates."""
        # Upload and encrypt
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        _trigger_migration(primary_admin)
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"Photo {pid} not encrypted"

        # Add to gallery
        gallery = user_client.create_secure_gallery("Backup Dup Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)
        time.sleep(5)  # clone migration

        # Sync
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Check backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)

        sync_photos, blob_items, combined, synced_blob_ids = _web_gallery_items(
            backup_user, "BACKUP"
        )

        # Nothing related to this photo should be visible
        assert pid not in combined, (
            f"Original photo {pid} visible on backup regular gallery"
        )

        # No blob with the encrypted_blob_id should be visible either
        # (check gallery items for the encrypted_blob_id)
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        items = backup_user.list_secure_gallery_items(gallery["gallery_id"], backup_token)
        item_list = items if isinstance(items, list) else items.get("items", [])
        for item in item_list:
            enc_blob = item.get("encrypted_blob_id")
            enc_thumb = item.get("encrypted_thumb_blob_id")
            if enc_blob:
                assert enc_blob not in combined, (
                    f"Gallery encrypted_blob_id {enc_blob} visible on backup as "
                    f"'{combined[enc_blob]['source']}' — this is the 'Queued' duplicate"
                )
            if enc_thumb:
                assert enc_thumb not in combined, (
                    f"Gallery encrypted_thumb_blob_id {enc_thumb} visible on backup"
                )

        # secureBlobIds must cover everything
        secure_ids = _secure_blob_id_set(backup_user)
        for item in item_list:
            enc_blob = item.get("encrypted_blob_id")
            if enc_blob:
                assert enc_blob in secure_ids, (
                    f"secureBlobIds on backup missing encrypted_blob_id {enc_blob}"
                )

    def test_presynced_photo_no_duplicate_on_backup(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """Photo synced BEFORE gallery add → add to gallery → sync →
        no duplicate on backup.

        This is the critical pre-synced scenario: the photo and its
        encrypted blob already exist on backup. After gallery add,
        the retroactive purge should clean up, and the encrypted blob
        must not reappear in list_blobs.
        """
        # Upload and encrypt
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        _trigger_migration(primary_admin)
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"Photo {pid} not encrypted"

        # First sync: photo and encrypted blob go to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Verify photo visible on backup before gallery add
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        _, _, before, _ = _web_gallery_items(backup_user, "BACKUP-BEFORE")
        # Photo should be visible (via encrypted-sync or list_blobs)
        visible_ids = set(before.keys())
        assert pid in visible_ids or any(
            p.get("encrypted_blob_id") and p["encrypted_blob_id"] in visible_ids
            for p in [{"encrypted_blob_id": v.get("photo", {}).get("encrypted_blob_id")}
                       for v in before.values() if v["source"] == "encrypted-sync"]
        ), f"Photo not visible on backup before gallery add"
        before_count = len(before)

        # Now add to gallery on primary
        gallery = user_client.create_secure_gallery("Pre-sync Dup Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)
        time.sleep(5)  # clone migration

        # Second sync
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Check backup: photo must be GONE, no duplicates
        _, _, after, _ = _web_gallery_items(backup_user, "BACKUP-AFTER")
        assert pid not in after, (
            f"Pre-synced photo {pid} still visible on backup after gallery add"
        )

        # The backup's gallery count should have gone DOWN
        assert len(after) == before_count - 1, (
            f"Expected {before_count - 1} visible items on backup, got {len(after)}. "
            f"Visible: {list(after.keys())}. "
            f"Something leaked — likely the encrypted blob of the gallery item."
        )

    def test_pre_encryption_sync_then_gallery_no_orphan(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """CRITICAL: Photo synced BEFORE encryption on primary → backup
        independently encrypts it → primary encrypts + gallery add → sync →
        backup should have NO orphaned encrypted blobs.

        This tests the exact user scenario: auto-sync runs before migration
        completes, creating a backup-side encrypted blob that becomes orphaned
        when the primary's encrypted_blob_id supersedes it.
        """
        # Upload photo but do NOT trigger migration yet
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # Sync BEFORE encryption — photo goes to backup with encrypted_blob_id=NULL
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Check backup: photo arrived unencrypted
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        _, _, mid, _ = _web_gallery_items(backup_user, "BACKUP-MID(unencrypted)")
        # Photo might be invisible in combined (no encrypted_blob_id → skipped)
        # but it SHOULD be in encrypted-sync as an unencrypted photo
        sync_res = backup_user.encrypted_sync(limit=500)
        backup_photo_ids = [p["id"] for p in sync_res.get("photos", [])]
        print(f"\n[CHECK] Backup has photo in encrypted-sync: {pid in backup_photo_ids}")

        # Now trigger migration on PRIMARY
        _trigger_migration(primary_admin)
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"Photo {pid} not encrypted on primary"
        print(f"[CHECK] Primary encrypted_blob_id: {enc_blob_id}")

        # Add to gallery on primary
        gallery = user_client.create_secure_gallery("Orphan Blob Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)
        time.sleep(5)  # clone migration

        # Second sync (sends updated photo + gallery data)
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(10)

        # Check backup: NO items should be visible in regular gallery
        sync_photos, blob_items, after, synced_blob_ids = _web_gallery_items(
            backup_user, "BACKUP-AFTER-ORPHAN"
        )

        assert pid not in after, (
            f"Photo {pid} still visible on backup after gallery sync"
        )

        # THE KEY CHECK: no orphaned encrypted blobs should appear
        # list_blobs should be completely empty (all blobs should be excluded)
        assert len(blob_items) == 0, (
            f"Orphaned blob(s) leaked into list_blobs on backup: "
            f"{[b['id'] for b in blob_items]}. These are likely backup-created "
            f"encrypted blobs that became orphaned when primary's encrypted_blob_id "
            f"superseded them."
        )

        assert len(after) == 0, (
            f"Expected 0 visible items on backup, got {len(after)}. "
            f"Visible items: {list(after.keys())}. "
            f"This is the 'extra duplicate' the user sees."
        )

    def test_gif_no_duplicate_on_backup(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """GIF-specific test: upload GIF → pre-sync → add to gallery → sync →
        check that no GIF blob leaks into list_blobs(gif) on backup.

        This targets the exact user scenario: a GIF in a secure gallery
        showing as 'Queued' in the regular gallery on backup.
        """
        # Minimal valid GIF89a (1x1 pixel, red)
        gif_content = (
            b'GIF89a\x01\x00\x01\x00\x80\x00\x00'
            b'\xff\x00\x00\x00\x00\x00'
            b'!\xf9\x04\x00\x00\x00\x00\x00'
            b',\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;'
        )

        # Upload as GIF
        fname = unique_filename().replace(".jpg", ".gif")
        photo = user_client.upload_photo(fname, content=gif_content, mime_type="image/gif")
        pid = photo["photo_id"]
        print(f"\n[GIF] Uploaded GIF: {pid}")

        # Trigger migration and wait
        _trigger_migration(primary_admin)
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"GIF {pid} not encrypted"
        print(f"[GIF] Primary encrypted_blob_id: {enc_blob_id}")

        # First sync: encrypted GIF goes to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Verify GIF visible on backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        _, _, before_backup, _ = _web_gallery_items(backup_user, "BACKUP-GIF-BEFORE")
        print(f"[GIF] Items visible on backup before gallery: {list(before_backup.keys())}")

        # Add GIF to secure gallery on primary
        gallery = user_client.create_secure_gallery("GIF Dup Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)
        time.sleep(5)  # clone migration

        # Second sync with gallery data
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(10)

        # Check backup: NO GIF should be visible
        _, blob_items, after_backup, _ = _web_gallery_items(backup_user, "BACKUP-GIF-AFTER")

        # Specifically check list_blobs("gif") — this is where the user sees the leak
        gif_blobs = [b for b in blob_items if b.get("blob_type") == "gif"]
        assert len(gif_blobs) == 0, (
            f"GIF blob(s) leaked into list_blobs(gif) on backup: "
            f"{[b['id'] for b in gif_blobs]}. This is the 'Queued GIF' the user sees."
        )

        assert len(after_backup) == 0, (
            f"Expected 0 visible items on backup after gallery sync, got {len(after_backup)}. "
            f"Visible: {list(after_backup.keys())}"
        )

    def test_gif_pre_encryption_sync_no_orphan(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """CRITICAL GIF test: sync GIF to backup BEFORE encryption →
        backup independently encrypts → add to gallery → sync →
        backup's orphan GIF blob must NOT leak.

        This combines the GIF-specific classify_blob_type("image/gif") = "gif"
        with the pre-encryption sync orphan scenario.
        """
        gif_content = (
            b'GIF89a\x01\x00\x01\x00\x80\x00\x00'
            b'\xff\x00\x00\x00\x00\x00'
            b'!\xf9\x04\x00\x00\x00\x00\x00'
            b',\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;'
        )
        fname = unique_filename().replace(".jpg", ".gif")
        photo = user_client.upload_photo(fname, content=gif_content, mime_type="image/gif")
        pid = photo["photo_id"]
        print(f"\n[GIF-ORPHAN] Uploaded GIF: {pid}")

        # Sync BEFORE encryption
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Check backup mid-state
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        _, mid_blobs, mid_combined, _ = _web_gallery_items(backup_user, "BACKUP-GIF-MID")
        mid_gif_blobs = [b for b in mid_blobs if b.get("blob_type") == "gif"]
        print(f"[GIF-ORPHAN] GIF blobs on backup mid-sync: {[b['id'] for b in mid_gif_blobs]}")

        # Now encrypt on primary
        _trigger_migration(primary_admin)
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"GIF {pid} not encrypted on primary"
        print(f"[GIF-ORPHAN] Primary encrypted_blob_id: {enc_blob_id}")

        # Add to gallery
        gallery = user_client.create_secure_gallery("GIF Orphan Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)
        time.sleep(5)

        # Sync again (gallery data + updated blobs)
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(10)

        # Final check: NO GIF should leak
        _, final_blobs, final_combined, _ = _web_gallery_items(backup_user, "BACKUP-GIF-FINAL")
        final_gif_blobs = [b for b in final_blobs if b.get("blob_type") == "gif"]
        assert len(final_gif_blobs) == 0, (
            f"Orphaned GIF blob(s) leaked into list_blobs(gif) on backup: "
            f"{[b['id'] for b in final_gif_blobs]}. "
            f"This is the persistent 'extra duplicate' — the backup's independently-"
            f"encrypted GIF blob was orphaned when primary's encrypted_blob_id arrived."
        )

        all_blobs = [b['id'] for b in final_blobs]
        assert len(final_blobs) == 0, (
            f"Orphaned blob(s) of any type leaked on backup: {all_blobs}"
        )

        assert len(final_combined) == 0, (
            f"Expected 0 visible items, got {len(final_combined)}. "
            f"Visible: {list(final_combined.keys())}"
        )

    def test_synced_photo_no_duplicate_blob_on_backup(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """ROOT CAUSE TEST: After syncing an encrypted photo to backup,
        the backup should show EXACTLY 1 item in the combined web gallery
        (via encrypted-sync), NOT 2.

        The bug: sync_blobs sends PRIMARY_E (the encrypted blob) to backup,
        but sync_photos doesn't include encrypted_blob_id in the headers.
        So on backup:
        - photos.encrypted_blob_id = BACKUP_E (from backup's own migration)
        - Blob PRIMARY_E exists in blobs table (from sync_blobs)
        - list_blobs filter 3 excludes BACKUP_E (it's in photos.encrypted_blob_id)
        - list_blobs does NOT exclude PRIMARY_E (no photos row references it)
        - Web client sees: encrypted-sync photo (1) + unsynced PRIMARY_E blob (2) = DUPLICATE

        Fix: sync_photos must include encrypted_blob_id so backup can connect
        PRIMARY_E to the photo row. Then filter 3 excludes PRIMARY_E.
        """
        # Upload GIF and encrypt on primary
        gif_content = (
            b'GIF89a\x01\x00\x01\x00\x80\x00\x00'
            b'\xff\x00\x00\x00\x00\x00'
            b'!\xf9\x04\x00\x00\x00\x00\x00'
            b',\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;'
        )
        fname = unique_filename().replace(".jpg", ".gif")
        photo = user_client.upload_photo(fname, content=gif_content, mime_type="image/gif")
        pid = photo["photo_id"]

        _trigger_migration(primary_admin)
        enc_blob_id = _wait_for_encryption(user_client, pid)
        assert enc_blob_id, f"GIF {pid} not encrypted"
        print(f"\n[DUP-ROOT] GIF photo: {pid}")
        print(f"[DUP-ROOT] Primary encrypted_blob_id: {enc_blob_id}")

        # Sync to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Check backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        sync_photos, blob_items, combined, synced_blob_ids = _web_gallery_items(
            backup_user, "BACKUP-DUP-ROOT"
        )

        # The photo should appear ONCE — via encrypted-sync
        assert pid in combined, f"Photo {pid} should be visible via encrypted-sync"
        assert combined[pid]["source"] == "encrypted-sync", (
            f"Photo {pid} should come from encrypted-sync, not {combined[pid]['source']}"
        )

        # The primary's encrypted blob should NOT appear as a separate entry
        assert enc_blob_id not in combined, (
            f"PRIMARY encrypted blob {enc_blob_id} leaked into list_blobs on backup! "
            f"It appears as '{combined[enc_blob_id]['source']}' type='{combined[enc_blob_id]['type']}'. "
            f"This is the 'extra duplicate' — sync_photos doesn't send encrypted_blob_id, "
            f"so the backup can't connect the blob to the photo row."
        )

        # Total should be exactly 1
        assert len(combined) == 1, (
            f"Expected exactly 1 visible item on backup, got {len(combined)}. "
            f"Items: {[(k, v['source'], v['type']) for k, v in combined.items()]}"
        )
