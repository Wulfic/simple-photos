"""
Test 06: Secure Galleries — create, unlock, add items, list items, delete,
         blob isolation, and STRICT gallery-count verification.

Every test that adds items to a secure gallery verifies that the main gallery
blob/photo count goes DOWN (original hidden) and no duplicates appear.
"""

import time
from collections import Counter

import pytest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from helpers import APIClient, generate_random_bytes, generate_test_jpeg, unique_filename

# Password to use for secure gallery unlock (must match user's account password)
from conftest import USER_PASSWORD, TEST_ENCRYPTION_KEY


# ── Helpers ───────────────────────────────────────────────────────────

def _blob_ids(client) -> list[str]:
    """Return all blob IDs from the regular blob listing (duplicates preserved)."""
    blobs = client.list_blobs(limit=500)
    return [b["id"] for b in blobs.get("blobs", [])]


def _photo_ids(client) -> list[str]:
    """Return all photo IDs from the regular photo listing."""
    photos = client.list_photos(limit=500)
    return [p["id"] for p in photos.get("photos", [])]


def _assert_no_duplicates(ids, label):
    counts = Counter(ids)
    dupes = {k: v for k, v in counts.items() if v > 1}
    assert not dupes, f"DUPLICATE {label}: {dupes}"


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
        _assert_no_duplicates(after_blobs, "blobs in main gallery after secure add")
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
        _assert_no_duplicates(after_blobs, "blobs")
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
        _assert_no_duplicates(after_photos, "photos in main gallery")
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
        _assert_no_duplicates(after_ids, "encrypted-sync photos")
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
        _assert_no_duplicates(before, "blobs before")
        for bid in blobs:
            assert bid in before

        gallery = user_client.create_secure_gallery("Count Five Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], blobs[2], token)

        after = _blob_ids(user_client)
        _assert_no_duplicates(after, "blobs after")
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
        _assert_no_duplicates(after, "blobs with multi-gallery")
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
        _assert_no_duplicates(all_visible, "combined gallery (sync + blobs)")
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


# ── AES-GCM decryption helper ────────────────────────────────────────

NONCE_LENGTH = 12  # AES-256-GCM uses 96-bit nonces

def _aes_gcm_decrypt(key_hex: str, data: bytes) -> bytes:
    """Decrypt AES-256-GCM data in the server wire format:
    [12-byte nonce][ciphertext + 16-byte auth tag]."""
    key = bytes.fromhex(key_hex)
    assert len(key) == 32, f"Key must be 32 bytes, got {len(key)}"
    if len(data) < NONCE_LENGTH + 16:
        raise ValueError(
            f"aes/gcm: invalid nonce length — data too short "
            f"({len(data)} bytes, minimum {NONCE_LENGTH + 16})"
        )
    nonce = data[:NONCE_LENGTH]
    ciphertext = data[NONCE_LENGTH:]
    try:
        return AESGCM(key).decrypt(nonce, ciphertext, None)
    except Exception as e:
        raise ValueError(f"AES/GCM invalid ghash tag — decryption failed: {e}")


class TestSecureGalleryDecryption:
    """Regression tests for decryption errors when viewing secure gallery items.

    Bug 1 (Primary): list_gallery_items returns the UNENCRYPTED clone blob_id
    instead of the encrypted_blob_id.  Client downloads raw photo data and
    AES-GCM decrypt fails with "AES/GCM invalid ghash tag".

    Bug 2 (Backup): The encrypted version of gallery items is never synced to
    the backup.  Client gets a placeholder blob (0 bytes) and decryption fails
    with "aes/gcm: invalid nonce length".
    """

    def _wait_for_encryption_migration(self, user_client, photo_id, max_wait=15):
        """Poll encrypted-sync until the photo gets an encrypted_blob_id."""
        start = time.time()
        while time.time() - start < max_wait:
            sync = user_client.encrypted_sync()
            for p in sync.get("photos", []):
                if p.get("id") == photo_id and p.get("encrypted_blob_id"):
                    return p["encrypted_blob_id"]
            time.sleep(0.5)
        return None

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
            plaintext = _aes_gcm_decrypt(TEST_ENCRYPTION_KEY, blob_data)
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
            plaintext = _aes_gcm_decrypt(TEST_ENCRYPTION_KEY, blob_data)
        except ValueError as e:
            pytest.fail(
                f"Gallery item blob on backup is NOT valid AES-GCM "
                f"ciphertext: {e}"
            )

        assert len(plaintext) > 0, "Decrypted payload is empty on backup"
