"""
Test 06: Secure Galleries — create, unlock, add items, list items, delete,
         blob isolation, and STRICT gallery-count verification.

Every test that adds items to a secure gallery verifies that the main gallery
blob/photo count goes DOWN (original hidden) and no duplicates appear.
"""

import time

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    unique_filename,
    assert_no_duplicates,
    aes_gcm_decrypt,
    wait_for_encryption,
    trigger_and_wait,
    login_on_server,
    web_gallery_items,
    secure_blob_id_set,
)
from conftest import USER_PASSWORD, TEST_ENCRYPTION_KEY

NONCE_LENGTH = 12  # AES-256-GCM uses 96-bit nonces


# ── Helpers ───────────────────────────────────────────────────────────

def _blob_ids(client):
    """Return all blob IDs from the regular blob listing (duplicates preserved)."""
    blobs = client.list_blobs(limit=500)
    return [b["id"] for b in blobs.get("blobs", [])]


def _photo_ids(client):
    """Return all photo IDs from the regular photo listing."""
    photos = client.list_photos(limit=500)
    return [p["id"] for p in photos.get("photos", [])]


class TestSecureGalleryCRUD:
    """Create, list, and delete secure galleries."""

    def test_create_secure_gallery(self, user_client):
        data = user_client.create_secure_gallery("Secret Gallery")
        assert "gallery_id" in data
        assert data["name"] == "Secret Gallery"

    def test_list_secure_galleries(self, user_client):
        user_client.create_secure_gallery("Listed Gallery")
        data = user_client.list_secure_galleries()
        assert "galleries" in data
        assert any(g["name"] == "Listed Gallery" for g in data["galleries"])

    def test_list_galleries_with_item_count(self, user_client):
        gallery = user_client.create_secure_gallery("Count Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        blob = user_client.upload_blob("photo")
        user_client.add_secure_gallery_item(gallery["gallery_id"], blob["blob_id"], token)

        galleries = user_client.list_secure_galleries()
        g = next(g for g in galleries["galleries"] if g["id"] == gallery["gallery_id"])
        assert g["item_count"] == 1

    def test_delete_secure_gallery(self, user_client):
        gallery = user_client.create_secure_gallery("Deletable Gallery")
        gid = gallery["gallery_id"]

        r = user_client.delete_secure_gallery(gid)
        assert r.status_code == 204

        galleries = user_client.list_secure_galleries()
        assert not any(g["id"] == gid for g in galleries["galleries"])

    def test_gallery_user_isolation(self, user_client, second_user_client):
        g1 = user_client.create_secure_gallery("User1 Gallery")
        g2 = second_user_client.create_secure_gallery("User2 Gallery")

        galleries1 = user_client.list_secure_galleries()
        galleries2 = second_user_client.list_secure_galleries()

        ids1 = [g["id"] for g in galleries1["galleries"]]
        ids2 = [g["id"] for g in galleries2["galleries"]]

        assert g1["gallery_id"] in ids1
        assert g2["gallery_id"] not in ids1
        assert g2["gallery_id"] in ids2
        assert g1["gallery_id"] not in ids2


class TestSecureGalleryUnlock:
    """Gallery unlock flow."""

    def test_unlock_with_correct_password(self, user_client):
        data = user_client.unlock_secure_gallery(USER_PASSWORD)
        assert "gallery_token" in data
        assert "expires_in" in data

    def test_unlock_with_wrong_password(self, user_client):
        r = user_client.post(
            "/api/galleries/secure/unlock",
            json_data={"password": "WrongPassword123!"},
        )
        assert r.status_code in (400, 401, 403)


class TestSecureGalleryItems:
    """Adding, listing, and managing items in secure galleries."""

    def test_add_item_to_gallery(self, user_client):
        gallery = user_client.create_secure_gallery("Items Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        blob = user_client.upload_blob("photo")
        data = user_client.add_secure_gallery_item(
            gallery["gallery_id"], blob["blob_id"], token
        )
        assert "item_id" in data

    def test_list_gallery_items_exact_count(self, user_client):
        gallery = user_client.create_secure_gallery("List Items Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        b1 = user_client.upload_blob("photo", generate_random_bytes(512))
        b2 = user_client.upload_blob("photo", generate_random_bytes(768))
        user_client.add_secure_gallery_item(gallery["gallery_id"], b1["blob_id"], token)
        user_client.add_secure_gallery_item(gallery["gallery_id"], b2["blob_id"], token)

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        assert "items" in items
        assert len(items["items"]) == 2, (
            f"Expected exactly 2 items in gallery, got {len(items['items'])}"
        )

    def test_list_items_without_token_fails(self, user_client):
        gallery = user_client.create_secure_gallery("No Token Gallery")
        r = user_client.get(f"/api/galleries/secure/{gallery['gallery_id']}/items")
        assert r.status_code in (400, 401, 403)


class TestSecureGalleryBlobIsolation:
    """THE CRITICAL TESTS: verify blobs are HIDDEN from the main gallery
    after being added to a secure gallery.  This is exactly the bug the
    user reported — adding a GIF to a secure album should NOT create
    duplicates in the main gallery listing."""

    def test_blob_hidden_from_main_gallery_after_secure_add(self, user_client):
        """Upload a blob, add it to a secure gallery —
        the original blob must DISAPPEAR from GET /api/blobs."""
        # Upload blob
        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        original_id = blob["blob_id"]

        # Verify it's in the main gallery
        before_blobs = _blob_ids(user_client)
        assert original_id in before_blobs, "Blob should be visible before secure add"
        before_count = len(before_blobs)

        # Add to secure gallery
        gallery = user_client.create_secure_gallery("Hide Test Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], original_id, token,
        )
        clone_id = add_result["new_blob_id"]

        # Main gallery: original AND clone must both be hidden
        after_blobs = _blob_ids(user_client)
        assert_no_duplicates(after_blobs, "blobs in main gallery after secure add")
        assert original_id not in after_blobs, (
            f"Original blob {original_id} should be HIDDEN from main gallery after secure add"
        )
        assert clone_id not in after_blobs, (
            f"Clone blob {clone_id} should be HIDDEN from main gallery"
        )
        assert len(after_blobs) == before_count - 1, (
            f"Main gallery should have 1 fewer blob: "
            f"was {before_count}, now {len(after_blobs)}"
        )

    def test_no_duplicate_blobs_after_secure_add(self, user_client):
        """Adding to a secure gallery must NOT increase blob count in main gallery."""
        # Upload multiple blobs
        b1 = user_client.upload_blob("photo", generate_random_bytes(512))
        b2 = user_client.upload_blob("photo", generate_random_bytes(768))
        b3 = user_client.upload_blob("photo", generate_random_bytes(1024))

        before_blobs = _blob_ids(user_client)
        before_count = len(before_blobs)
        assert before_count >= 3

        # Add b2 to secure gallery
        gallery = user_client.create_secure_gallery("No Dupe Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], b2["blob_id"], token)

        after_blobs = _blob_ids(user_client)
        assert_no_duplicates(after_blobs, "blobs")
        assert len(after_blobs) == before_count - 1, (
            f"Expected {before_count - 1} blobs (one hidden), got {len(after_blobs)}"
        )
        assert b1["blob_id"] in after_blobs, "b1 should still be visible"
        assert b2["blob_id"] not in after_blobs, "b2 should be hidden"
        assert b3["blob_id"] in after_blobs, "b3 should still be visible"

    def test_photo_hidden_from_main_gallery_after_secure_add(self, user_client):
        """Server-side photo added to secure gallery must be hidden from
        GET /api/photos."""
        # Upload a photo (goes to photos table)
        fname = unique_filename()
        photo = user_client.upload_photo(fname)
        pid = photo["photo_id"]

        before_photos = _photo_ids(user_client)
        assert pid in before_photos
        before_count = len(before_photos)

        # Add to secure gallery
        gallery = user_client.create_secure_gallery("Photo Hide Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = add_result["new_blob_id"]

        # Photos listing should hide original and clone
        after_photos = _photo_ids(user_client)
        assert_no_duplicates(after_photos, "photos in main gallery")
        assert pid not in after_photos, (
            f"Original photo {pid} should be HIDDEN from gallery"
        )
        assert clone_id not in after_photos, (
            f"Clone {clone_id} should be HIDDEN from gallery"
        )
        assert len(after_photos) == before_count - 1, (
            f"Gallery should shrink by 1: was {before_count}, now {len(after_photos)}"
        )

    def test_secure_add_then_encrypted_sync_hides_item(self, user_client):
        """The encrypted-sync endpoint must also hide secure gallery items."""
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        before = user_client.encrypted_sync()
        before_ids = [p["id"] for p in before.get("photos", [])]
        assert pid in before_ids

        gallery = user_client.create_secure_gallery("EncSync Hide Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        after = user_client.encrypted_sync()
        after_ids = [p["id"] for p in after.get("photos", [])]
        assert_no_duplicates(after_ids, "encrypted-sync photos")
        assert pid not in after_ids, (
            f"Photo {pid} should be hidden from encrypted-sync after secure add"
        )

    def test_secure_blob_ids_endpoint(self, user_client):
        gallery = user_client.create_secure_gallery("Isolation Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        blob = user_client.upload_blob("photo", generate_random_bytes(512))
        user_client.add_secure_gallery_item(gallery["gallery_id"], blob["blob_id"], token)

        data = user_client.get_secure_gallery_blob_ids()
        assert "blob_ids" in data
        assert blob["blob_id"] in data["blob_ids"]

    def test_delete_gallery_restores_blob_to_main_gallery(self, user_client):
        """Deleting a secure gallery should make the original blob visible again
        in the main gallery, or at minimum the blob should still be downloadable."""
        content = generate_random_bytes(512)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        before_count = len(_blob_ids(user_client))

        gallery = user_client.create_secure_gallery("Delete Restore Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], bid, token)

        # Blob hidden
        mid_blobs = _blob_ids(user_client)
        assert bid not in mid_blobs

        # Delete gallery
        user_client.delete_secure_gallery(gallery["gallery_id"])

        # Original blob should be downloadable
        r = user_client.download_blob(bid)
        assert r.status_code == 200
        assert r.content == content

    def test_multiple_blobs_one_secured_counts(self, user_client):
        """Upload 5 blobs, secure 1: main gallery should show exactly 4."""
        blobs = []
        for _ in range(5):
            b = user_client.upload_blob("photo", generate_random_bytes(256))
            blobs.append(b["blob_id"])

        before = _blob_ids(user_client)
        assert_no_duplicates(before, "blobs before")
        for bid in blobs:
            assert bid in before

        gallery = user_client.create_secure_gallery("Count Five Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], blobs[2], token)

        after = _blob_ids(user_client)
        assert_no_duplicates(after, "blobs after")
        assert blobs[2] not in after, "Secured blob should be hidden"
        for i in [0, 1, 3, 4]:
            assert blobs[i] in after, f"Blob {i} should still be visible"


class TestSecureGalleryMultiGallery:
    """Multiple galleries per user."""

    def test_multiple_galleries_independent(self, user_client):
        g1 = user_client.create_secure_gallery("Gallery A")
        g2 = user_client.create_secure_gallery("Gallery B")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        b1 = user_client.upload_blob("photo", generate_random_bytes(512))
        b2 = user_client.upload_blob("photo", generate_random_bytes(768))

        user_client.add_secure_gallery_item(g1["gallery_id"], b1["blob_id"], token)
        user_client.add_secure_gallery_item(g2["gallery_id"], b2["blob_id"], token)

        items1 = user_client.list_secure_gallery_items(g1["gallery_id"], token)
        items2 = user_client.list_secure_gallery_items(g2["gallery_id"], token)

        assert len(items1["items"]) == 1
        assert len(items2["items"]) == 1
        item_ids_1 = {i["id"] for i in items1["items"]}
        item_ids_2 = {i["id"] for i in items2["items"]}
        assert item_ids_1.isdisjoint(item_ids_2)

    def test_multiple_galleries_both_hidden_from_main(self, user_client):
        """Blobs in two different secure galleries should both be hidden."""
        b1 = user_client.upload_blob("photo", generate_random_bytes(512))
        b2 = user_client.upload_blob("photo", generate_random_bytes(768))
        b3 = user_client.upload_blob("photo", generate_random_bytes(1024))

        before_count = len(_blob_ids(user_client))

        g1 = user_client.create_secure_gallery("Multi A")
        g2 = user_client.create_secure_gallery("Multi B")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(g1["gallery_id"], b1["blob_id"], token)
        user_client.add_secure_gallery_item(g2["gallery_id"], b2["blob_id"], token)

        after = _blob_ids(user_client)
        assert_no_duplicates(after, "blobs with multi-gallery")
        assert b1["blob_id"] not in after
        assert b2["blob_id"] not in after
        assert b3["blob_id"] in after
        assert len(after) == before_count - 2


class TestSecureGalleryEncryptedBlobLeak:
    """Verify that when a server-side photo is added to a secure gallery,
    the encrypted blob copies created by server-side migration are ALSO
    hidden from the blob listing.

    This catches the real-world bug: user has server-side photos (autoscanned
    from disk) → server encrypts them into blob copies → user adds photo to
    secure gallery → encrypted blob copies leak into the main gallery, causing
    duplicates and "aes/gcm: invalid ghash tag" decryption errors in the web
    client.

    Root cause: encrypted_gallery_items tracks the photos.id as
    original_blob_id, but the encrypted blob (photos.encrypted_blob_id) has a
    DIFFERENT id that is not tracked, so it passes through the NOT IN filter.
    """

    def _wait_for_migration(self, user_client, max_wait=10):
        """Poll list_blobs until encrypted blobs appear (bug) or timeout (pass).

        The server-side encryption migration runs asynchronously after
        add_gallery_item.  We detect completion by watching for new blobs to
        appear in the listing.  Returns the final blob list.
        """
        start = time.time()
        blobs = []
        while time.time() - start < max_wait:
            blobs = _blob_ids(user_client)
            if blobs:
                return blobs  # Migration created visible blobs (the bug)
            time.sleep(0.5)
        return blobs  # Timeout — no blobs appeared (correct behavior)

    def test_server_photo_encrypted_blobs_hidden_after_secure_add(self, user_client):
        """BUG: Upload a server-side photo, add to secure gallery, wait for
        encryption migration — encrypted blobs must NOT appear in list_blobs.

        The server's add_gallery_item spawns auto_migrate_after_scan which
        encrypts both the original photo and the clone.  The resulting
        encrypted blob IDs are NOT tracked in encrypted_gallery_items, so
        they pass through the filter and appear in the main gallery.
        """
        # Upload a server-side photo (creates photos table entry only)
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # Verify photo appears in listings before securing
        assert pid in _photo_ids(user_client), "Photo should be visible"

        # Create secure gallery and add the photo
        gallery = user_client.create_secure_gallery("EncBlob Leak Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = result["new_blob_id"]

        # Photo should be hidden from photos listing immediately
        assert pid not in _photo_ids(user_client), "Original hidden from photos"
        assert clone_id not in _photo_ids(user_client), "Clone hidden from photos"

        # Wait for background encryption migration to complete.
        # Migration encrypts both original and clone, creating new blobs.
        # These encrypted blobs MUST also be hidden from list_blobs.
        leaked = self._wait_for_migration(user_client)

        assert len(leaked) == 0, (
            f"BUG: {len(leaked)} encrypted blobs leaked into main gallery after "
            f"securing a server-side photo.  The encrypted_blob_id and "
            f"encrypted_thumb_blob_id of photos in secure galleries must be "
            f"hidden from GET /api/blobs.  Leaked IDs: {leaked}"
        )

    def test_encrypted_sync_plus_blobs_no_duplicates(self, user_client):
        """BUG: The web client combines encrypted_sync + list_blobs to build
        the gallery.  After securing a server-side photo, NEITHER endpoint
        should return anything related to that photo.

        This replicates the exact user flow that produces 2 copies of a GIF
        in the main gallery.
        """
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # Verify encrypted_sync shows the photo before securing
        sync_before = user_client.encrypted_sync()
        sync_ids = [p["id"] for p in sync_before.get("photos", [])]
        assert pid in sync_ids, "Photo should appear in encrypted-sync"

        # Add to secure gallery
        gallery = user_client.create_secure_gallery("Web Flow Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = result["new_blob_id"]

        # encrypted_sync: both original and clone must be hidden
        sync_after = user_client.encrypted_sync()
        sync_ids = [p["id"] for p in sync_after.get("photos", [])]
        assert pid not in sync_ids, "Original hidden from encrypted-sync"
        assert clone_id not in sync_ids, "Clone hidden from encrypted-sync"

        # Wait for migration to create encrypted blobs
        leaked = self._wait_for_migration(user_client)

        # Combine: the web client's gallery is encrypted_sync UNION list_blobs.
        # Both must be empty for a user with only one photo that was secured.
        all_visible = sync_ids + leaked
        assert_no_duplicates(all_visible, "combined gallery (sync + blobs)")
        assert len(all_visible) == 0, (
            f"BUG: {len(all_visible)} items visible in main gallery after "
            f"securing the only photo.  sync={sync_ids}, blobs={leaked}"
        )

    def test_secure_blob_ids_covers_encrypted_blobs(self, user_client):
        """The secureBlobIds endpoint should include the encrypted_blob_id
        of secured photos so the web client can do client-side filtering
        as a fallback.
        """
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("SecureIDs Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = result["new_blob_id"]

        # Wait for migration
        leaked = self._wait_for_migration(user_client)

        # secureBlobIds must include any blob that leaked into list_blobs
        secure_data = user_client.get_secure_gallery_blob_ids()
        secure_set = set(secure_data.get("blob_ids", []))

        # Original photo_id and clone_id must be in the secure set
        assert pid in secure_set, "Original photo ID in secureBlobIds"
        assert clone_id in secure_set, "Clone ID in secureBlobIds"

        # Every leaked blob must also be in the secure set (client fallback)
        for bid in leaked:
            assert bid in secure_set, (
                f"BUG: Blob {bid} is visible in list_blobs but NOT in "
                f"secureBlobIds — web client cannot filter it out.  "
                f"secureBlobIds={secure_set}"
            )


class TestSecureGalleryDecryption:
    """Regression tests for decryption errors when viewing secure gallery items.

    Bug 1 (Primary): list_gallery_items returns the UNENCRYPTED clone blob_id
    instead of the encrypted_blob_id.  Client downloads raw photo data and
    AES-GCM decrypt fails with "AES/GCM invalid ghash tag".

    Bug 2 (Backup): The encrypted version of gallery items is never synced to
    the backup.  Client gets a placeholder blob (0 bytes) and decryption fails
    with "aes/gcm: invalid nonce length".
    """

    def test_gallery_item_blob_is_decryptable_primary(self, user_client):
        """PRIMARY BUG: Server-side photo in secure gallery → download the
        blob_id from list_gallery_items → must be valid AES-GCM ciphertext
        decryptable with the test encryption key.

        Regression: list_gallery_items returned the clone's raw blob_id
        (unencrypted file copy), not the encrypted_blob_id created by
        server-side migration.  The client tried to AES-GCM decrypt
        unencrypted JPEG data → "AES/GCM invalid ghash tag".
        """
        # Upload a server-side photo (goes through /api/photos/upload)
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # Create secure gallery and add the photo
        gallery = user_client.create_secure_gallery("Decrypt Test Primary")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = result["new_blob_id"]

        # Wait for server-side encryption migration to complete.
        # After migration, the clone photo gets encrypted_blob_id set.
        time.sleep(3)

        # List gallery items — the blob_id returned here is what the client
        # downloads and tries to decrypt
        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        assert len(items["items"]) == 1, f"Expected 1 item, got {len(items['items'])}"
        item_blob_id = items["items"][0]["blob_id"]

        # The blob_id must NOT be the plaintext clone — it must be the encrypted version
        assert item_blob_id != clone_id, (
            f"list_gallery_items returned the plaintext clone blob_id {clone_id} "
            f"instead of the encrypted_blob_id"
        )

        # Download the blob
        resp = user_client.download_blob(item_blob_id)
        assert resp.status_code == 200, (
            f"Failed to download gallery item blob {item_blob_id}: HTTP {resp.status_code}"
        )
        blob_data = resp.content
        assert len(blob_data) >= NONCE_LENGTH + 16, (
            f"aes/gcm: invalid nonce length — blob data too short "
            f"({len(blob_data)} bytes).  This means the gallery item blob "
            f"is empty or a placeholder, not valid encrypted content."
        )

        # Decrypt — this is exactly what the web client does
        try:
            plaintext = aes_gcm_decrypt(TEST_ENCRYPTION_KEY, blob_data)
        except ValueError as e:
            pytest.fail(
                f"Gallery item blob {item_blob_id} is NOT valid AES-GCM "
                f"ciphertext: {e}.  list_gallery_items likely returned the "
                f"unencrypted clone blob_id instead of encrypted_blob_id."
            )

        # The decrypted payload should be a JSON object with a "v" field
        # (the wire format used by server-side encryption)
        assert len(plaintext) > 0, "Decrypted payload is empty"

    def test_gallery_item_blob_is_not_raw_photo_data(self, user_client):
        """Verify that the blob returned for a gallery item is NOT the raw
        unencrypted photo data.  If it is, the server is serving the wrong blob.
        """
        # Upload a known JPEG
        jpeg_content = generate_test_jpeg()
        photo = user_client.upload_photo(unique_filename(), content=jpeg_content)
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("Raw Check Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        # Wait for migration
        time.sleep(3)

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        item_blob_id = items["items"][0]["blob_id"]

        resp = user_client.download_blob(item_blob_id)
        assert resp.status_code == 200
        blob_data = resp.content

        # JPEG files start with FF D8 FF. If the blob starts with this magic,
        # the server is serving raw unencrypted photo data — the bug!
        is_jpeg = blob_data[:3] == b'\xff\xd8\xff'
        assert not is_jpeg, (
            f"BUG: Gallery item blob {item_blob_id} is raw JPEG data "
            f"(starts with FF D8 FF).  list_gallery_items is returning the "
            f"unencrypted clone blob_id instead of the encrypted_blob_id."
        )

    def test_gallery_item_blob_decryptable_on_backup(
        self, user_client, primary_admin, backup_server, backup_admin,
        backup_configured,
    ):
        """BACKUP BUG: After sync, gallery items on backup must also be
        decryptable.  Currently fails because:
        1. The encrypted blob is excluded from sync_blobs
        2. Placeholder blobs have 0 bytes → "aes/gcm: invalid nonce length"
        """
        # Upload server-side photo and add to secure gallery
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("Decrypt Test Backup")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )

        # Wait for encryption migration
        time.sleep(3)

        # Trigger sync to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(5)  # Allow sync to complete

        # Now verify on backup: create a user client for the backup
        # The user was synced along with photos
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)

        # List gallery items on backup
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        items = backup_user.list_secure_gallery_items(gallery["gallery_id"], backup_token)
        assert len(items["items"]) >= 1, "Gallery items should be synced to backup"
        item_blob_id = items["items"][0]["blob_id"]

        # Download the blob on backup
        resp = backup_user.download_blob(item_blob_id)
        assert resp.status_code == 200, (
            f"Failed to download gallery item blob on backup: HTTP {resp.status_code}.  "
            f"The encrypted blob may not have been synced."
        )
        blob_data = resp.content
        assert len(blob_data) >= NONCE_LENGTH + 16, (
            f"aes/gcm: invalid nonce length — backup blob too short "
            f"({len(blob_data)} bytes).  The blob is likely a placeholder "
            f"rather than actual encrypted content."
        )

        # Decrypt
        try:
            plaintext = aes_gcm_decrypt(TEST_ENCRYPTION_KEY, blob_data)
        except ValueError as e:
            pytest.fail(
                f"Gallery item blob on backup is NOT valid AES-GCM "
                f"ciphertext: {e}"
            )

        assert len(plaintext) > 0, "Decrypted payload is empty on backup"

    # ── Thumbnail availability tests ─────────────────────────────────────

    def test_gallery_items_include_thumb_blob_id(self, user_client):
        """list_gallery_items MUST include encrypted_thumb_blob_id so the web
        client can download and decrypt the thumbnail for display.

        Without this field the client has no way to locate the encrypted
        thumbnail blob and falls back to showing a lock icon (🔐).
        """
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("Thumb Info Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        # Wait for encryption migration to create the encrypted thumbnail
        time.sleep(3)

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        assert len(items["items"]) == 1
        item = items["items"][0]

        # The response must include encrypted_thumb_blob_id
        assert "encrypted_thumb_blob_id" in item, (
            "list_gallery_items response is missing 'encrypted_thumb_blob_id'. "
            "Without it the web client cannot fetch encrypted thumbnails "
            "and shows a lock icon instead of the actual photo thumbnail."
        )
        assert item["encrypted_thumb_blob_id"] is not None, (
            "encrypted_thumb_blob_id is null — encryption migration may not "
            "have generated a thumbnail for this photo."
        )

    def test_gallery_thumbnail_blob_decryptable_primary(self, user_client):
        """The encrypted_thumb_blob_id returned by list_gallery_items must be
        downloadable and AES-GCM decryptable on the primary server.
        """
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("Thumb Decrypt Primary")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        time.sleep(3)

        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        item = items["items"][0]
        thumb_blob_id = item.get("encrypted_thumb_blob_id")
        assert thumb_blob_id, "encrypted_thumb_blob_id missing from response"

        # Download the thumbnail blob directly
        resp = user_client.download_blob(thumb_blob_id)
        assert resp.status_code == 200, (
            f"Failed to download thumbnail blob {thumb_blob_id}: "
            f"HTTP {resp.status_code}"
        )

        blob_data = resp.content
        assert len(blob_data) >= NONCE_LENGTH + 16, (
            f"Thumbnail blob too short ({len(blob_data)} bytes) — "
            f"not valid AES-GCM ciphertext."
        )

        try:
            plaintext = aes_gcm_decrypt(TEST_ENCRYPTION_KEY, blob_data)
        except ValueError as e:
            pytest.fail(
                f"Thumbnail blob {thumb_blob_id} is not valid AES-GCM: {e}"
            )
        assert len(plaintext) > 0, "Decrypted thumbnail payload is empty"

    def test_gallery_thumbnail_blob_downloadable_on_backup(
        self, user_client, primary_admin, backup_server, backup_admin,
        backup_configured,
    ):
        """BACKUP BUG: Thumbnail blob for secure gallery items must be
        available on the backup server.  Currently the backup has no way
        to resolve the thumbnail because:

        1. The clone photo row is excluded from sync_photos
        2. encrypted_thumb_blob_id is not included in the gallery item response
        3. Even if included, the thumbnail blob may not be synced to backup
        """
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        gallery = user_client.create_secure_gallery("Thumb Backup Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        time.sleep(3)

        # Trigger sync
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(5)

        # Verify on backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)[
            "gallery_token"
        ]

        items = backup_user.list_secure_gallery_items(
            gallery["gallery_id"], backup_token,
        )
        assert len(items["items"]) >= 1, "Gallery items not synced to backup"
        item = items["items"][0]

        # Thumbnail blob ID must be in the response
        thumb_blob_id = item.get("encrypted_thumb_blob_id")
        assert thumb_blob_id, (
            "encrypted_thumb_blob_id missing from backup gallery item response. "
            "Without it the client gets 'Download failed http 404' when "
            "trying to view the photo."
        )

        # And the thumbnail blob must actually be downloadable on backup
        resp = backup_user.download_blob(thumb_blob_id)
        assert resp.status_code == 200, (
            f"Thumbnail blob download on backup failed: HTTP {resp.status_code}. "
            f"The encrypted thumbnail blob was not synced to backup."
        )

        # Verify it's valid encrypted content
        blob_data = resp.content
        assert len(blob_data) >= NONCE_LENGTH + 16, (
            f"Backup thumbnail blob too short ({len(blob_data)} bytes)"
        )

        try:
            aes_gcm_decrypt(TEST_ENCRYPTION_KEY, blob_data)
        except ValueError as e:
            pytest.fail(f"Backup thumbnail blob not valid AES-GCM: {e}")

    # ── Client-encrypted blob gallery tests ──────────────────────────

    def test_client_encrypted_blob_in_gallery_downloadable_on_backup(
        self, user_client, primary_admin, backup_server, backup_admin,
        backup_configured,
    ):
        """BACKUP BUG: Client-encrypted blobs added to secure galleries
        must be downloadable on the backup server.

        The web/Android client uploads photos as client-encrypted blobs via
        /api/blobs (not /api/photos/upload).  When such a blob is added to
        a secure gallery:
        - A clone is created in the blobs table
        - No photos table row is created (is_server_side = false)
        - No server-side encryption migration runs

        On the primary this works — the clone blob has the correct user_id.
        On the backup, sync_blobs EXCLUDES the clone (it's in
        encrypted_gallery_items.blob_id) and the backup only has a
        gallery-placeholder with admin user_id.  list_gallery_items COALESCE
        falls through to gi.blob_id → placeholder → user_id mismatch → 404.
        """
        # Upload a client-encrypted blob (the workflow the web client uses)
        blob_data = generate_random_bytes(512)
        blob = user_client.upload_blob("photo", blob_data)
        blob_id = blob["blob_id"]

        # Create secure gallery and add the client-encrypted blob
        gallery = user_client.create_secure_gallery("Client Blob Backup Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], blob_id, token,
        )
        clone_id = result["new_blob_id"]

        # Client-encrypted blobs don't get server-side encryption, but
        # give the server a moment to settle
        time.sleep(1)

        # Verify on primary first: list_gallery_items returns clone blob_id
        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)
        assert len(items["items"]) == 1
        primary_blob_id = items["items"][0]["blob_id"]

        # For client-encrypted blobs, blob_id should be the clone itself
        # (no encrypted version exists)
        assert primary_blob_id == clone_id, (
            f"Expected blob_id={clone_id} (clone), got {primary_blob_id}"
        )

        # Download on primary works
        resp = user_client.download_blob(primary_blob_id)
        assert resp.status_code == 200, (
            f"Primary download failed: HTTP {resp.status_code}"
        )

        # Trigger sync to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(5)

        # Verify on backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)[
            "gallery_token"
        ]

        items = backup_user.list_secure_gallery_items(
            gallery["gallery_id"], backup_token,
        )
        assert len(items["items"]) >= 1, "Gallery items not synced to backup"
        backup_blob_id = items["items"][0]["blob_id"]

        # The backup should return the same blob_id as the primary
        assert backup_blob_id == clone_id, (
            f"Backup blob_id mismatch: expected {clone_id}, got {backup_blob_id}. "
            f"The COALESCE may have fallen through to a placeholder."
        )

        # Download the blob on backup — THIS is where the 404 occurs
        resp = backup_user.download_blob(backup_blob_id)
        assert resp.status_code == 200, (
            f"Client-encrypted gallery blob download on backup failed: "
            f"HTTP {resp.status_code}. The clone blob was likely excluded "
            f"from sync_blobs and only a gallery-placeholder exists on "
            f"the backup (with admin user_id → user mismatch → 404)."
        )

        # Content should match what was uploaded
        assert len(resp.content) == len(blob_data), (
            f"Downloaded blob size mismatch: expected {len(blob_data)}, "
            f"got {len(resp.content)} bytes. The backup may have a "
            f"gallery-placeholder (0 bytes) instead of actual data."
        )

    def test_server_photo_gallery_viewable_on_backup_after_prior_sync(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """BACKUP BUG: Server-side photo synced to backup BEFORE being added
        to a secure gallery becomes un-viewable after a second sync.

        Root cause: the sync_blobs exclusion filter hides the original
        photo's encrypted_blob_id from backup transfer.  When the
        encryption migration reuses the same encrypted blob for the gallery
        clone (content_hash dedup), the gallery item references a blob that
        was never sent to the backup → 404.

        This requires a two-sync scenario:
        1. Photo synced to backup → backup migration encrypts it locally
        2. Photo added to gallery → clone reuses original's encrypted blob
        3. Second sync must send the reused blob to backup
        """
        # 1. Upload a server-side photo on primary
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # 2. Wait for primary encryption migration to process the photo
        time.sleep(4)

        # 3. First sync: photo + its encrypted blob get synced to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(6)

        # 4. Wait for backup encryption migration to process the synced photo.
        time.sleep(5)

        # 5. Add photo to secure gallery on primary (creates clone)
        gallery = user_client.create_secure_gallery("Prior Sync Bug Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)

        # 6. Wait for clone encryption on primary
        time.sleep(4)

        # 7. Second sync
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(6)

        # 8. Verify on backup: list gallery items and download
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)[
            "gallery_token"
        ]

        items = backup_user.list_secure_gallery_items(
            gallery["gallery_id"], backup_token,
        )
        assert len(items["items"]) >= 1, "Gallery items not synced to backup"
        item = items["items"][0]

        blob_id = item["blob_id"]

        # Download the blob — this is what the web client does
        resp = backup_user.download_blob(blob_id)
        assert resp.status_code == 200, (
            f"Gallery blob download on backup failed after prior sync: "
            f"HTTP {resp.status_code}. blob_id={blob_id}. "
            f"The encrypted blob was likely excluded from sync_blobs "
            f"transfer due to aggressive gallery-original filtering."
        )

        # Verify it's valid encrypted content (not a 0-byte placeholder)
        assert len(resp.content) >= NONCE_LENGTH + 16, (
            f"Blob too short ({len(resp.content)} bytes) — likely a "
            f"gallery-placeholder instead of real encrypted data."
        )

        # Also verify thumbnail is downloadable if present
        thumb_blob_id = item.get("encrypted_thumb_blob_id")
        if thumb_blob_id:
            resp = backup_user.download_blob(thumb_blob_id)
            assert resp.status_code == 200, (
                f"Thumbnail blob download on backup failed after prior sync: "
                f"HTTP {resp.status_code}. thumb_blob_id={thumb_blob_id}."
            )
            assert len(resp.content) >= NONCE_LENGTH + 16, (
                f"Thumbnail blob too short ({len(resp.content)} bytes)."
            )

    def test_gallery_encrypted_blobs_hidden_from_backup_blob_listing(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """BACKUP BUG: GIF/photo encrypted blobs of secure gallery items
        appear in the regular blob listing on the backup, causing
        "Queued" items in the web client's main gallery.

        Root cause: backup's list_blobs uses a JOIN on the photos table to
        find encrypted_blob_ids of gallery items, but clone photos are NOT
        synced to the backup's photos table (excluded since Bug 5).  The
        JOIN fails for clones, so their encrypted blobs leak through.

        The secureBlobIds endpoint has the same JOIN-based gap, so the web
        client cannot filter them out client-side either.
        """
        # 1. Upload a server-side photo on primary
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]

        # 2. Wait for primary encryption migration
        time.sleep(4)

        # 3. Add photo to secure gallery on primary (creates clone)
        gallery = user_client.create_secure_gallery("Backup Blob Leak Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = user_client.add_secure_gallery_item(
            gallery["gallery_id"], pid, token,
        )
        clone_id = add_result["new_blob_id"]

        # 4. Wait for clone encryption migration on primary
        time.sleep(4)

        # 5. Verify primary hides everything from blob listing
        primary_blobs = [b["id"] for b in user_client.list_blobs(limit=500).get("blobs", [])]
        assert pid not in primary_blobs, "Original photo hidden from primary list_blobs"
        assert clone_id not in primary_blobs, "Clone hidden from primary list_blobs"

        # 6. Sync to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # 7. Log in as user on backup and check blob listing
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)

        backup_blobs = backup_user.list_blobs(limit=500)
        backup_blob_ids = [b["id"] for b in backup_blobs.get("blobs", [])]

        # 8. Gallery items and their encrypted blobs must NOT appear
        assert pid not in backup_blob_ids, (
            f"BUG: Original photo {pid} visible in backup list_blobs"
        )
        assert clone_id not in backup_blob_ids, (
            f"BUG: Clone blob {clone_id} visible in backup list_blobs"
        )

        # 9. Check that no encrypted blobs leaked through either.
        #    The gallery item's encrypted_blob_id should not be in the listing.
        backup_token = backup_user.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        items = backup_user.list_secure_gallery_items(gallery["gallery_id"], backup_token)
        item_list = items if isinstance(items, list) else items.get("items", [])
        for item in item_list:
            enc_blob = item.get("encrypted_blob_id")
            enc_thumb = item.get("encrypted_thumb_blob_id")
            if enc_blob:
                assert enc_blob not in backup_blob_ids, (
                    f"BUG: Gallery item's encrypted_blob_id {enc_blob} leaked "
                    f"into backup list_blobs — shows as 'Queued' in web client"
                )
            if enc_thumb:
                assert enc_thumb not in backup_blob_ids, (
                    f"BUG: Gallery item's encrypted_thumb_blob_id {enc_thumb} "
                    f"leaked into backup list_blobs"
                )

        # 10. secureBlobIds on backup must cover all gallery encrypted blobs
        secure_data = backup_user.get_secure_gallery_blob_ids()
        secure_set = set(secure_data.get("blob_ids", []))
        assert pid in secure_set, (
            f"secureBlobIds on backup missing original photo ID {pid}"
        )
        assert clone_id in secure_set, (
            f"secureBlobIds on backup missing clone ID {clone_id}"
        )
        for item in item_list:
            enc_blob = item.get("encrypted_blob_id")
            enc_thumb = item.get("encrypted_thumb_blob_id")
            if enc_blob:
                assert enc_blob in secure_set, (
                    f"BUG: secureBlobIds on backup missing encrypted_blob_id "
                    f"{enc_blob} — web client cannot filter it out"
                )
            if enc_thumb:
                assert enc_thumb in secure_set, (
                    f"BUG: secureBlobIds on backup missing encrypted_thumb_blob_id "
                    f"{enc_thumb} — web client cannot filter it out"
                )

    def test_backup_no_duplicate_encrypted_blob_in_gallery(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """BACKUP BUG: After syncing an encrypted photo to backup, the web
        client shows a DUPLICATE — the photo appears BOTH via encrypted-sync
        AND as a separate "Queued" blob in list_blobs.

        Root cause: sync_photos did not include encrypted_blob_id in the
        transfer headers, so the backup's photos.encrypted_blob_id was set
        by the backup's own independent migration (a different blob ID).
        The primary's encrypted blob (sent by sync_blobs) was not connected
        to any photos row on the backup, so list_blobs filter 3 could not
        exclude it.  The web client's dedup logic (syncedBlobIds) used the
        backup's blob ID, leaving the primary's blob as "unsynced."

        Fix: sync_photos and sync_metadata now include encrypted_blob_id
        and encrypted_thumb_blob_id so the backup's photos table references
        the primary's encrypted blobs.
        """
        # Upload a GIF on primary and wait for encryption
        gif_content = (
            b'GIF89a\x01\x00\x01\x00\x80\x00\x00'
            b'\xff\x00\x00\x00\x00\x00'
            b'!\xf9\x04\x00\x00\x00\x00\x00'
            b',\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;'
        )
        fname = unique_filename().replace(".jpg", ".gif")
        photo = user_client.upload_photo(fname, content=gif_content, mime_type="image/gif")
        pid = photo["photo_id"]

        # Trigger migration and wait for encryption
        primary_admin.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
        enc_blob_id = wait_for_encryption(user_client, pid, max_wait=30)
        assert enc_blob_id, f"GIF {pid} not encrypted within timeout"

        # Sync to backup
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Check what the web client would see on backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)

        # encrypted-sync: should show the photo with encrypted_blob_id
        sync_res = backup_user.encrypted_sync(limit=500)
        sync_photos = sync_res.get("photos", [])
        synced_blob_ids = {
            p["encrypted_blob_id"]
            for p in sync_photos
            if p.get("encrypted_blob_id")
        }

        # list_blobs: all media types
        all_blobs = []
        for btype in ("photo", "gif", "video", "audio"):
            res = backup_user.list_blobs(blob_type=btype, limit=200)
            all_blobs.extend(res.get("blobs", []))

        # Unsynced blobs = blobs NOT deduped by synced_blob_ids
        unsynced = [b for b in all_blobs if b["id"] not in synced_blob_ids]

        # The primary's encrypted blob must NOT appear as an unsynced blob
        assert enc_blob_id not in {b["id"] for b in unsynced}, (
            f"BUG: Primary encrypted blob {enc_blob_id} leaked into "
            f"list_blobs as an unsynced blob. synced_blob_ids={synced_blob_ids}. "
            f"This causes a 'duplicate photo' in the web gallery."
        )

        # Combined visible = encrypted-sync photos + unsynced blobs
        # Should be exactly 1
        visible_count = len([p for p in sync_photos if p.get("encrypted_blob_id")]) + len(unsynced)
        assert visible_count == 1, (
            f"Expected 1 visible item on backup, got {visible_count}. "
            f"encrypted-sync={len(sync_photos)}, unsynced_blobs={len(unsynced)}. "
            f"This is the 'extra duplicate photo' the user sees."
        )

    def test_presynced_photo_no_flash_after_gallery_add(
        self, user_client, primary_admin, backup_server,
        backup_admin, backup_configured,
    ):
        """SYNC ORDERING BUG: A pre-synced photo added to a secure gallery
        on the primary briefly appeared in the backup's regular gallery
        during sync, before disappearing.

        Root cause: sync_galleries (which purges pre-synced gallery photos
        from backup and adds egi metadata) ran LAST — after sync_blobs and
        sync_metadata had already delivered the encrypted blob and updated
        encrypted_blob_id.  During this window the backup's encrypted-sync
        returned the photo (it had encrypted_blob_id but no egi filter).

        Fix: sync_galleries now runs BEFORE sync_blobs and sync_metadata,
        so the backup has gallery exclusion data before any encrypted data
        arrives.
        """
        # Upload and encrypt on primary
        photo = user_client.upload_photo(unique_filename())
        pid = photo["photo_id"]
        primary_admin.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
        enc_blob_id = wait_for_encryption(user_client, pid, max_wait=30)
        assert enc_blob_id, f"Photo {pid} not encrypted"

        # First sync: photo goes to backup (pre-synced)
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(8)

        # Verify photo visible on backup
        backup_user = APIClient(backup_server.base_url)
        backup_user.login(user_client.username, USER_PASSWORD)
        sync_res = backup_user.encrypted_sync(limit=500)
        backup_photo_ids = {p["id"] for p in sync_res.get("photos", [])}
        assert pid in backup_photo_ids, "Photo should be on backup before gallery add"

        # Add to secure gallery on primary
        gallery = user_client.create_secure_gallery("Flash Order Test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], pid, token)
        time.sleep(5)  # clone migration

        # Second sync (with fixed ordering: galleries before blobs/metadata)
        primary_admin.admin_trigger_sync(backup_configured)
        time.sleep(10)

        # After sync: photo MUST NOT be in encrypted-sync on backup
        sync_res = backup_user.encrypted_sync(limit=500)
        backup_photo_ids = {p["id"] for p in sync_res.get("photos", [])}
        assert pid not in backup_photo_ids, (
            f"Pre-synced photo {pid} still in backup's encrypted-sync after "
            f"gallery add + sync. The sync_galleries phase should have purged "
            f"it before sync_metadata could update encrypted_blob_id."
        )

        # Also not in list_blobs
        all_blobs = []
        for btype in ("photo", "gif", "video", "audio"):
            res = backup_user.list_blobs(blob_type=btype, limit=200)
            all_blobs.extend(res.get("blobs", []))
        blob_ids = {b["id"] for b in all_blobs}
        assert enc_blob_id not in blob_ids, (
            f"Encrypted blob {enc_blob_id} leaked into backup list_blobs"
        )
