"""
Test 03: Blobs — encrypted blob upload, list, download, delete, range requests.
"""

import hashlib
import pytest
from helpers import APIClient, generate_random_bytes


class TestBlobUpload:
    """Blob upload and integrity verification."""

    def test_upload_blob(self, user_client):
        content = generate_random_bytes(2048)
        client_hash = hashlib.sha256(content).hexdigest()
        data = user_client.upload_blob("photo", content, client_hash=client_hash)
        assert "blob_id" in data
        assert data["size"] == 2048

    def test_upload_blob_types(self, user_client):
        """Test all valid blob types."""
        for blob_type in ("photo", "video", "thumbnail", "audio", "gif"):
            content = generate_random_bytes(512)
            data = user_client.upload_blob(blob_type, content)
            assert "blob_id" in data

    def test_upload_blob_with_content_hash(self, user_client):
        content = generate_random_bytes(1024)
        content_hash = hashlib.sha256(b"original_content").hexdigest()[:16]
        data = user_client.upload_blob("photo", content, content_hash=content_hash)
        assert "blob_id" in data

    def test_upload_blob_hash_mismatch(self, user_client):
        """Client hash mismatch should be rejected."""
        content = generate_random_bytes(1024)
        wrong_hash = "0" * 64
        r = user_client.post(
            "/api/blobs",
            data=content,
            headers={
                "x-blob-type": "photo",
                "x-client-hash": wrong_hash,
                "Content-Type": "application/octet-stream",
            },
        )
        # Server should reject mismatched hash
        assert r.status_code in (400, 409, 422)


class TestBlobList:
    """Blob listing and filtering."""

    def test_list_blobs(self, user_client):
        user_client.upload_blob("photo")
        data = user_client.list_blobs()
        assert "blobs" in data
        assert len(data["blobs"]) >= 1

    def test_list_blobs_filter_by_type(self, user_client):
        user_client.upload_blob("thumbnail")
        data = user_client.list_blobs(blob_type="thumbnail")
        assert "blobs" in data
        for blob in data["blobs"]:
            assert blob["blob_type"] == "thumbnail"

    def test_list_blobs_pagination(self, user_client):
        for _ in range(3):
            user_client.upload_blob("photo")
        data = user_client.list_blobs(limit=1)
        assert len(data["blobs"]) <= 1
        if data.get("next_cursor"):
            data2 = user_client.list_blobs(after=data["next_cursor"])
            assert "blobs" in data2

    def test_list_blobs_user_isolation(self, user_client, second_user_client):
        """Each user should only see their own blobs."""
        b1 = user_client.upload_blob("photo")
        b2 = second_user_client.upload_blob("photo")

        list1 = user_client.list_blobs()
        list2 = second_user_client.list_blobs()

        ids1 = [b["id"] for b in list1["blobs"]]
        ids2 = [b["id"] for b in list2["blobs"]]

        assert b1["blob_id"] in ids1
        assert b2["blob_id"] not in ids1
        assert b2["blob_id"] in ids2


class TestBlobDownload:
    """Blob download, range requests, and caching."""

    def test_download_blob(self, user_client):
        content = generate_random_bytes(2048)
        data = user_client.upload_blob("photo", content)
        r = user_client.download_blob(data["blob_id"])
        assert r.status_code == 200
        assert r.content == content

    def test_download_blob_range(self, user_client):
        content = generate_random_bytes(4096)
        data = user_client.upload_blob("photo", content)
        r = user_client.get(
            f"/api/blobs/{data['blob_id']}",
            headers={"Range": "bytes=100-199"},
        )
        assert r.status_code == 206
        assert r.content == content[100:200]

    def test_download_blob_etag(self, user_client):
        content = generate_random_bytes(1024)
        data = user_client.upload_blob("photo", content)
        r1 = user_client.download_blob(data["blob_id"])
        etag = r1.headers.get("ETag")
        if etag:
            r2 = user_client.get(
                f"/api/blobs/{data['blob_id']}",
                headers={"If-None-Match": etag},
            )
            assert r2.status_code == 304

    def test_download_blob_not_found(self, user_client):
        # Must use valid UUID format; non-UUID strings return 400
        import uuid
        r = user_client.download_blob(str(uuid.uuid4()))
        assert r.status_code == 404

    def test_download_blob_invalid_id(self, user_client):
        r = user_client.download_blob("nonexistent-blob-id")
        assert r.status_code == 400

    def test_download_other_users_blob(self, user_client, second_user_client):
        content = generate_random_bytes(512)
        data = user_client.upload_blob("photo", content)
        r = second_user_client.download_blob(data["blob_id"])
        assert r.status_code in (403, 404)


class TestBlobDelete:
    """Blob deletion."""

    def test_delete_blob(self, user_client):
        data = user_client.upload_blob("photo")
        blob_id = data["blob_id"]

        r = user_client.delete_blob(blob_id)
        assert r.status_code == 204

        # Should be gone
        r = user_client.download_blob(blob_id)
        assert r.status_code == 404

    def test_delete_other_users_blob(self, user_client, second_user_client):
        data = user_client.upload_blob("photo")
        r = second_user_client.delete_blob(data["blob_id"])
        assert r.status_code in (403, 404)

        # Original should still exist
        r = user_client.download_blob(data["blob_id"])
        assert r.status_code == 200
