"""
Test 04: Trash — soft delete, restore, permanent delete, empty trash, thumbnails.

In this encrypted system, photos are trashed via the blob trash endpoint
(POST /api/blobs/{id}/trash), not via a DELETE /api/photos/{id} endpoint.
"""

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    unique_filename,
    wait_for_encryption,
)


class TestBlobSoftDelete:
    """Soft-deleting blobs to trash (encrypted mode)."""

    def test_soft_delete_blob_to_trash(self, user_client):
        blob = user_client.upload_blob("photo")
        bid = blob["blob_id"]

        data = user_client.soft_delete_blob(
            bid, filename="blob_trash.jpg", mime_type="image/jpeg",
            size_bytes=1024,
        )
        assert "trash_id" in data
        assert "expires_at" in data

        # Blob should be gone
        r = user_client.download_blob(bid)
        assert r.status_code == 404

    def test_soft_delete_nonexistent_blob(self, user_client):
        """Deleting a nonexistent blob should return 404."""
        r = user_client.post("/api/blobs/nonexistent-blob-id/trash", json_data={
            "filename": "nope.jpg",
            "mime_type": "image/jpeg",
        })
        assert r.status_code in (400, 404)

    def test_soft_delete_other_users_blob(self, user_client, second_user_client):
        blob = user_client.upload_blob("photo")
        r = second_user_client.post(f"/api/blobs/{blob['blob_id']}/trash", json_data={
            "filename": "stolen.jpg",
            "mime_type": "image/jpeg",
        })
        assert r.status_code in (403, 404)

        # Should still be downloadable by owner
        r = user_client.download_blob(blob["blob_id"])
        assert r.status_code == 200


class TestTrashList:
    """Trash listing."""

    def test_list_trash_items(self, user_client):
        blob = user_client.upload_blob("photo")
        user_client.soft_delete_blob(blob["blob_id"], filename="trash_list.jpg")

        trash = user_client.list_trash()
        assert "items" in trash
        assert len(trash["items"]) >= 1

    def test_list_trash_pagination(self, user_client):
        for _ in range(3):
            blob = user_client.upload_blob("photo")
            user_client.soft_delete_blob(blob["blob_id"], filename="paginate.jpg")

        trash = user_client.list_trash(limit=1)
        assert len(trash["items"]) <= 1

    def test_trash_user_isolation(self, user_client, second_user_client):
        """Users should only see their own trash."""
        b1 = user_client.upload_blob("photo")
        b2 = second_user_client.upload_blob("photo")
        t1 = user_client.soft_delete_blob(b1["blob_id"], filename="user1.jpg")
        t2 = second_user_client.soft_delete_blob(b2["blob_id"], filename="user2.jpg")

        trash1 = user_client.list_trash()
        trash2 = second_user_client.list_trash()

        ids1 = {t["id"] for t in trash1["items"]}
        ids2 = {t["id"] for t in trash2["items"]}

        assert t1["trash_id"] in ids1
        assert t1["trash_id"] not in ids2


class TestTrashRestore:
    """Restoring items from trash."""

    def test_restore_blob_from_trash(self, user_client):
        """Restore a blob that was soft-deleted to trash."""
        content = generate_random_bytes(1024)
        blob = user_client.upload_blob("photo", content)
        bid = blob["blob_id"]

        # Soft-delete to trash
        trash_data = user_client.soft_delete_blob(
            bid, filename="restore_me.dat", size_bytes=len(content))
        trash_id = trash_data["trash_id"]

        # Restore
        r = user_client.restore_trash(trash_id)
        assert r.status_code == 204

        # Blob should be accessible again
        r = user_client.download_blob(bid)
        assert r.status_code == 200
        assert r.content == content

    def test_restore_content_integrity(self, user_client):
        """Blob content should be byte-identical after trash+restore."""
        content = generate_random_bytes(4096)
        blob = user_client.upload_blob("photo", content)

        trash_data = user_client.soft_delete_blob(
            blob["blob_id"], filename="integrity.bin", size_bytes=len(content))
        r = user_client.restore_trash(trash_data["trash_id"])
        assert r.status_code == 204

        r = user_client.download_blob(blob["blob_id"])
        assert r.status_code == 200
        assert r.content == content

    def test_restore_nonexistent_trash(self, user_client):
        r = user_client.restore_trash("nonexistent-trash-id")
        assert r.status_code == 404

    def test_restore_removes_from_trash(self, user_client):
        """After a restore, the item should disappear from the trash list."""
        blob = user_client.upload_blob("photo")
        trash_data = user_client.soft_delete_blob(
            blob["blob_id"], filename="remove_check.jpg")
        tid = trash_data["trash_id"]

        user_client.restore_trash(tid)

        trash = user_client.list_trash()
        trash_ids = {t["id"] for t in trash["items"]}
        assert tid not in trash_ids


class TestTrashPermanentDelete:
    """Permanent deletion from trash."""

    def test_permanent_delete(self, user_client):
        blob = user_client.upload_blob("photo")
        trash_data = user_client.soft_delete_blob(
            blob["blob_id"], filename="perm.jpg")
        tid = trash_data["trash_id"]

        r = user_client.permanent_delete_trash(tid)
        assert r.status_code == 204

        # Can't restore anymore
        r = user_client.restore_trash(tid)
        assert r.status_code == 404

    def test_empty_trash(self, user_client):
        # Add some items to trash
        for _ in range(2):
            blob = user_client.upload_blob("photo")
            user_client.soft_delete_blob(blob["blob_id"], filename="empty.jpg")

        data = user_client.empty_trash()
        assert "deleted" in data


class TestTrashScanReadd:
    """Regression (#3): the filesystem autoscan must NOT re-import a photo that
    is sitting in trash.

    Server-side encryption leaves the plaintext original on disk (photos.file_path
    stays set, the file is never removed). The encrypted-blob soft-delete deletes
    the photos row and recorded only the *blob* storage_path, so the scan's
    exclusion set missed the orphaned plaintext original and re-registered it on
    the next scan — the photo reappeared in the gallery while still in trash.
    """

    def test_scan_does_not_readd_trashed_photo(self, admin_client):
        fn = unique_filename("jpg")

        # Upload a photo: writes uploads/<fn>, registers a photos row, and
        # (per upload.rs) spawns server-side encryption.
        up = admin_client.upload_photo(filename=fn)
        photo_id = up["photo_id"]
        file_path = up["file_path"]

        # Wait for encryption so the photo has an encrypted blob to trash.
        blob_id = wait_for_encryption(admin_client, photo_id)
        assert blob_id, "photo was never encrypted — cannot exercise blob trash path"

        def count_with_path() -> int:
            photos = admin_client.list_photos(limit=500).get("photos", [])
            return sum(1 for p in photos if p.get("file_path") == file_path)

        assert count_with_path() == 1, "uploaded photo not registered"

        # Soft-delete the encrypted blob → trash (deletes the photos row).
        trash = admin_client.soft_delete_blob(blob_id, filename=fn)
        trash_id = trash["trash_id"]
        assert count_with_path() == 0, "photos row not removed by soft-delete"

        # Trigger the filesystem autoscan. The plaintext original is still on
        # disk; before the fix this re-registered it synchronously.
        admin_client.admin_trigger_scan()

        # Must NOT come back, and must still be in trash.
        assert count_with_path() == 0, "autoscan re-imported a trashed photo (#3 regression)"
        trash_ids = {t["id"] for t in admin_client.list_trash().get("items", [])}
        assert trash_id in trash_ids, "item disappeared from trash after scan"

    def test_permanent_delete_removes_original_so_scan_skips_it(self, admin_client):
        """After permanent delete, the plaintext original must be gone so a
        later scan cannot re-import it (dropping the trash row removes the
        exclusion, so the file itself must be removed)."""
        fn = unique_filename("jpg")
        up = admin_client.upload_photo(filename=fn)
        file_path = up["file_path"]
        blob_id = wait_for_encryption(admin_client, up["photo_id"])
        assert blob_id

        trash = admin_client.soft_delete_blob(blob_id, filename=fn)
        r = admin_client.permanent_delete_trash(trash["trash_id"])
        assert r.status_code == 204

        admin_client.admin_trigger_scan()
        photos = admin_client.list_photos(limit=500).get("photos", [])
        assert not any(p.get("file_path") == file_path for p in photos), \
            "scan re-imported a permanently-deleted photo (#3 regression)"


class TestTrashThumbnail:
    """Trash thumbnail serving."""

    def test_trash_thumbnail(self, user_client):
        blob = user_client.upload_blob("photo")
        trash_data = user_client.soft_delete_blob(
            blob["blob_id"], filename="thumb_test.jpg")

        r = user_client.get(f"/api/trash/{trash_data['trash_id']}/thumb")
        # 200 = thumbnail found, 404 = no thumb for this blob
        assert r.status_code in (200, 404)
