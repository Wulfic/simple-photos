"""
Test 12: Edge Cases & Error Handling — boundary conditions, invalid inputs,
         path traversal, cross-user attacks, rate limiting, storage stats.
"""

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    unique_filename,
    random_username,
)
from conftest import USER_PASSWORD


class TestInvalidInputs:
    """Invalid, malformed, and boundary inputs."""

    def test_upload_empty_filename(self, user_client):
        content = generate_test_jpeg()
        r = user_client.post(
            "/api/photos/upload",
            data=content,
            headers={"X-Filename": "", "X-Mime-Type": "image/jpeg",
                     "Content-Type": "application/octet-stream"},
        )
        assert r.status_code in (400, 422)

    def test_upload_path_traversal_filename(self, user_client):
        """Path traversal in filename should be sanitized or rejected."""
        content = generate_test_jpeg()
        r = user_client.post(
            "/api/photos/upload",
            data=content,
            headers={"X-Filename": "../../../etc/passwd",
                     "X-Mime-Type": "image/jpeg",
                     "Content-Type": "application/octet-stream"},
        )
        if r.status_code in (200, 201):
            # If accepted, the filename should be sanitized
            data = r.json()
            assert ".." not in data.get("filename", "")
            assert "/" not in data.get("file_path", "").split("/")[0] if data.get("file_path") else True
        else:
            assert r.status_code == 400

    def test_upload_null_bytes_in_filename(self, user_client):
        content = generate_test_jpeg()
        r = user_client.post(
            "/api/photos/upload",
            data=content,
            headers={"X-Filename": "test\x00evil.jpg",
                     "X-Mime-Type": "image/jpeg",
                     "Content-Type": "application/octet-stream"},
        )
        if r.status_code in (200, 201):
            data = r.json()
            assert "\x00" not in data.get("filename", "")
        else:
            assert r.status_code == 400

    def test_upload_very_long_filename(self, user_client):
        content = generate_test_jpeg()
        long_name = "a" * 500 + ".jpg"
        r = user_client.post(
            "/api/photos/upload",
            data=content,
            headers={"X-Filename": long_name,
                     "X-Mime-Type": "image/jpeg",
                     "Content-Type": "application/octet-stream"},
        )
        # Should either accept with truncated name or reject
        assert r.status_code in (200, 201, 400, 422)

    def test_upload_unicode_filename(self, user_client):
        from urllib.parse import quote
        content = generate_test_jpeg()
        # URL-encode unicode filename for HTTP header (latin-1 safe)
        filename = quote("日本語テスト.jpg")
        data = user_client.upload_photo(filename, content)
        assert "photo_id" in data

    def test_upload_special_chars_filename(self, user_client):
        """Filenames with special characters should be handled."""
        content = generate_test_jpeg()
        for name in ["photo (1).jpg", "photo [copy].jpg", "photo's.jpg"]:
            data = user_client.upload_photo(name, content)
            assert "photo_id" in data

    def test_nonexistent_photo_operations(self, user_client):
        """Operations on nonexistent photos should return 404."""
        fake_id = "00000000-0000-0000-0000-000000000000"
        assert user_client.get_photo_file(fake_id).status_code == 404
        assert user_client.get_photo_thumb(fake_id).status_code in (404, 202)

    def test_nonexistent_blob_trash(self, user_client):
        """Trashing a nonexistent blob should return 400/404."""
        fake_id = "00000000-0000-0000-0000-000000000000"
        r = user_client.post(f"/api/blobs/{fake_id}/trash", json_data={
            "filename": "nope.jpg", "mime_type": "image/jpeg",
        })
        assert r.status_code in (400, 404)

    def test_invalid_blob_type(self, user_client):
        content = generate_random_bytes(512)
        r = user_client.post(
            "/api/blobs",
            data=content,
            headers={
                "x-blob-type": "invalid_type",
                "x-client-hash": "a" * 64,
                "Content-Type": "application/octet-stream",
            },
        )
        assert r.status_code == 400

    def test_create_album_empty_name(self, user_client):
        r = user_client.post("/api/sharing/albums", json_data={"name": ""})
        assert r.status_code in (400, 422)

    def test_create_gallery_empty_name(self, user_client):
        r = user_client.post("/api/galleries/secure", json_data={"name": ""})
        assert r.status_code in (400, 422)


class TestAuthorizationBoundaries:
    """Cross-user access attempts and privilege escalation."""

    def test_user_cannot_access_admin_endpoints(self, user_client):
        endpoints = [
            ("GET", "/api/admin/users"),
            ("GET", "/api/admin/storage"),
            ("GET", "/api/admin/diagnostics"),
            ("GET", "/api/admin/backup/servers"),
            ("GET", "/api/admin/backup/mode"),
        ]
        for method, path in endpoints:
            if method == "GET":
                r = user_client.get(path)
            else:
                r = user_client.post(path)
            assert r.status_code == 403, f"{method} {path} returned {r.status_code}"

    def test_user_cannot_trash_other_users_blobs(self, user_client, second_user_client):
        blob = user_client.upload_blob("photo")
        r = second_user_client.post(f"/api/blobs/{blob['blob_id']}/trash", json_data={
            "filename": "stolen.jpg", "mime_type": "image/jpeg",
        })
        assert r.status_code in (403, 404)

    def test_user_cannot_access_other_users_gallery(self, user_client, second_user_client):
        gallery = user_client.create_secure_gallery("Other User Gallery")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        # Second user tries to list items
        r = second_user_client.get(
            f"/api/galleries/secure/{gallery['gallery_id']}/items",
            headers={"x-gallery-token": token},
        )
        assert r.status_code in (403, 404)

    def test_unauthenticated_access_rejected(self, primary_server):
        client = APIClient(primary_server.base_url)
        protected = [
            "/api/photos",
            "/api/blobs",
            "/api/trash",
            "/api/tags",
            "/api/galleries/secure",
            "/api/sharing/albums",
            "/api/settings/storage-stats",
        ]
        for path in protected:
            r = client.get(path)
            assert r.status_code == 401, f"{path} returned {r.status_code} without auth"

    def test_expired_token_rejected(self, primary_server):
        """A completely fabricated token should be rejected."""
        client = APIClient(primary_server.base_url)
        client.access_token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.fake"
        r = client.get("/api/photos")
        assert r.status_code == 401


class TestBackupApiKeyAuth:
    """Backup API key authentication."""

    def test_backup_endpoints_require_api_key(self, backup_server):
        client = APIClient(backup_server.base_url)
        endpoints = [
            "/api/backup/list",
            "/api/backup/list-trash",
            "/api/backup/list-users",
            "/api/backup/list-blobs",
        ]
        for path in endpoints:
            r = client.get(path)
            assert r.status_code in (401, 403), f"{path} returned {r.status_code} without API key"

    def test_backup_wrong_api_key_rejected(self, backup_server):
        client = APIClient(backup_server.base_url, api_key="wrong-key-12345")
        r = client.get("/api/backup/list", headers={"X-API-Key": "wrong-key-12345"})
        assert r.status_code in (401, 403)

    def test_backup_correct_api_key_accepted(self, backup_client):
        data = backup_client.backup_list()
        assert isinstance(data, list)


class TestStorageStats:
    """Storage statistics endpoint."""

    def test_storage_stats(self, user_client):
        # Storage stats count from blobs table, not photos table
        user_client.upload_blob("photo", generate_random_bytes(512))
        stats = user_client.storage_stats()
        assert "photo_count" in stats
        assert stats["photo_count"] >= 1
        assert "photo_bytes" in stats


class TestBlobDeleteRestore:
    """Complete delete-restore lifecycle for blobs."""

    def test_full_blob_lifecycle(self, user_client):
        """Upload blob → trash → restore → verify accessible."""
        content = generate_random_bytes(2048)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        # Trash — pass size_bytes so restore preserves Content-Length
        trash_data = user_client.soft_delete_blob(bid, filename="lifecycle.jpg",
                                                   size_bytes=len(content))

        # Verify gone
        r = user_client.download_blob(bid)
        assert r.status_code == 404

        # Restore
        r = user_client.restore_trash(trash_data["trash_id"])
        assert r.status_code == 204

        # Verify back
        r = user_client.download_blob(bid)
        assert r.status_code == 200
        assert r.content == content

    def test_permanent_delete_then_reupload(self, user_client):
        """Permanently delete a blob, then upload new content."""
        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        # Trash then permanent delete
        trash_data = user_client.soft_delete_blob(bid, filename="perm.jpg")
        user_client.permanent_delete_trash(trash_data["trash_id"])

        # Upload new content (should get new blob_id)
        new_blob = user_client.upload_blob("photo", generate_random_bytes(1024))
        assert new_blob["blob_id"] != bid


class TestBlobTrashRoundtrip:
    """Blob → trash → restore → verify content integrity."""

    def test_blob_content_integrity_after_restore(self, user_client):
        """Blob content should be identical after trash + restore."""
        content = generate_random_bytes(4096)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        # Soft delete — pass size_bytes so restore preserves Content-Length
        trash_data = user_client.soft_delete_blob(bid, filename="integrity.dat",
                                                   size_bytes=len(content))

        # Restore
        user_client.restore_trash(trash_data["trash_id"])

        # Download and verify
        r = user_client.download_blob(bid)
        assert r.status_code == 200
        assert r.content == content, "Blob content changed after restore!"


class TestSyncAfterComplexOperations:
    """Sync engine handles complex state correctly."""

    def test_sync_after_rapid_create_delete(self, primary_admin, user_client,
                                            backup_configured, backup_client):
        """Create many blobs, trash some, then sync."""
        from helpers import wait_for_sync

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
        primary_admin.admin_trigger_sync(backup_configured)
        wait_for_sync(primary_admin, backup_configured)

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

        from helpers import wait_for_sync
        primary_admin.admin_trigger_sync(backup_configured)
        result = wait_for_sync(primary_admin, backup_configured)
        assert result.get("status") != "error"

    def test_sync_with_shared_album_members_and_photos(self, primary_admin, user_client,
                                                       backup_configured):
        """Complex shared album state should sync correctly."""
        album = user_client.create_shared_album("Complex Sync Album")

        # Add photos
        for _ in range(3):
            p = user_client.upload_photo(unique_filename())
            user_client.add_album_photo(album["id"], p["photo_id"])

        from helpers import wait_for_sync
        primary_admin.admin_trigger_sync(backup_configured)
        result = wait_for_sync(primary_admin, backup_configured)
        assert result.get("status") != "error"


class TestBackupViewReadOnly:
    """Verify backup photos listed via the admin proxy cannot be
    mutated through the primary server's normal endpoints.

    When a user browses photos via the Active Server dropdown, the
    frontend lists photos from the backup proxy
    (GET /api/admin/backup/servers/:id/photos). Those photo IDs belong
    to the *backup* database — they should NOT be mutable through the
    primary server's user-facing endpoints.
    """

    def test_proxy_photo_ids_not_favoritable_on_primary(
        self, primary_admin, user_client, backup_configured, backup_client
    ):
        """Attempting to favorite a backup-only photo ID on the primary must fail."""
        from helpers import wait_for_sync

        # Upload a photo and sync so at least one photo exists on backup
        user_client.upload_photo(unique_filename())
        primary_admin.admin_trigger_sync(backup_configured)
        wait_for_sync(primary_admin, backup_configured)

        photos = primary_admin.admin_get_backup_photos(backup_configured)
        assert len(photos) > 0, "Need at least one backup photo"

        backup_photo_id = photos[0]["id"]

        # Try to favorite the backup photo via primary's user endpoint
        r = user_client.put(f"/api/photos/{backup_photo_id}/favorite")
        # Should fail (404 or 403) because the photo ID doesn't exist on primary
        assert r.status_code in (400, 403, 404, 500), (
            f"Primary server allowed favoriting backup-only photo ID "
            f"{backup_photo_id}: HTTP {r.status_code}"
        )

    def test_proxy_photo_ids_not_deletable_on_primary(
        self, primary_admin, user_client, backup_configured, backup_client
    ):
        """Attempting to delete a backup-only photo ID on the primary must fail."""
        from helpers import wait_for_sync

        user_client.upload_photo(unique_filename())
        primary_admin.admin_trigger_sync(backup_configured)
        wait_for_sync(primary_admin, backup_configured)

        photos = primary_admin.admin_get_backup_photos(backup_configured)
        assert len(photos) > 0

        backup_photo_id = photos[0]["id"]

        # Try to trash the backup photo blob via primary
        r = user_client.post(f"/api/blobs/{backup_photo_id}/trash", json_data={
            "filename": "test.jpg",
            "mime_type": "image/jpeg",
        })
        assert r.status_code in (400, 403, 404, 500), (
            f"Primary server allowed trashing backup-only photo ID "
            f"{backup_photo_id}: HTTP {r.status_code}"
        )

    def test_proxy_photo_ids_not_editable_on_primary(
        self, primary_admin, user_client, backup_configured, backup_client
    ):
        """Attempting to crop a backup-only photo ID on the primary must fail."""
        from helpers import wait_for_sync

        user_client.upload_photo(unique_filename())
        primary_admin.admin_trigger_sync(backup_configured)
        wait_for_sync(primary_admin, backup_configured)

        photos = primary_admin.admin_get_backup_photos(backup_configured)
        assert len(photos) > 0

        backup_photo_id = photos[0]["id"]

        # Try to set crop on the backup photo via primary
        r = user_client.put(
            f"/api/photos/{backup_photo_id}/crop",
            json_data={"crop_metadata": '{"x":0,"y":0}'},
        )
        assert r.status_code in (400, 403, 404, 500), (
            f"Primary server allowed cropping backup-only photo ID "
            f"{backup_photo_id}: HTTP {r.status_code}"
        )

    def test_backup_proxy_thumb_accessible(
        self, primary_admin, backup_configured, backup_client
    ):
        """Backup thumbnails must be accessible through the proxy endpoint."""
        photos = primary_admin.admin_get_backup_photos(backup_configured)
        if not photos:
            pytest.skip("No backup photos to test thumbnail proxy")

        photo_id = photos[0]["id"]
        r = primary_admin.get(
            f"/api/admin/backup/servers/{backup_configured}/photos/{photo_id}/thumb"
        )
        assert r.status_code == 200, (
            f"Expected thumbnail, got HTTP {r.status_code}"
        )
        assert len(r.content) > 0, "Thumbnail response was empty"
