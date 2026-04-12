"""
Test 26: Secure Gallery Grid, Export Packaging, and Secure GIF Autoplay

Regression tests for four issues:
1. Secure gallery API returns item dimensions + media_type so the client
   can render a justified (flex) grid instead of forcing 1:1 squares.
2. Library export must NOT include thumbnails — only original media.
3. Library export must NOT have .bin files — metadata (album_manifest etc.)
   should be in a readable format (JSON) inside a metadata/ subfolder.
4. Secure gallery items include media_type so the client can identify GIFs
   and trigger autoplay.
"""

import io
import json
import os
import time
import zipfile

import pytest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_gif,
    generate_test_jpeg,
    unique_filename,
)

USER_PASSWORD = "E2eUserPass456!"
TEST_ENCRYPTION_KEY = "a" * 64  # 64 hex chars = 32 bytes AES-256


# ── Helpers ────────────────────────────────────────────────────────────────────

def _encrypt_for_server(plaintext: bytes) -> bytes:
    """Encrypt data using the test key (AES-256-GCM).
    Wire format: [12-byte nonce][ciphertext + 16-byte tag]."""
    key_bytes = bytes.fromhex(TEST_ENCRYPTION_KEY)
    nonce = os.urandom(12)
    aesgcm = AESGCM(key_bytes)
    ciphertext = aesgcm.encrypt(nonce, plaintext, None)
    return nonce + ciphertext


def _wait_for_encryption(client, *, timeout=30):
    """Wait for all photos to have encrypted_blob_id set via encrypted-sync."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        sync = client.encrypted_sync()
        photos = sync.get("photos", [])
        if photos and all(p.get("encrypted_blob_id") for p in photos):
            return photos
        time.sleep(1.0)
    return client.encrypted_sync().get("photos", [])


def _trigger_and_wait_for_encryption(user_client, admin_client, *, timeout=30):
    """Re-store the encryption key (idempotent) to trigger the server-side
    encryption migration, then wait for all photos to be encrypted."""
    try:
        admin_client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
    except Exception:
        pass  # may already be stored
    return _wait_for_encryption(user_client, timeout=timeout)


def _poll_export_completion(client, *, timeout=60):
    """Start export, poll until completed, return export status JSON."""
    r = client.post("/api/export", json_data={"size_limit": 10_737_418_240})
    assert r.status_code == 200, f"Start export failed: {r.status_code} {r.text}"
    job_id = r.json()["id"]

    deadline = time.time() + timeout
    while time.time() < deadline:
        r = client.get("/api/export/status")
        assert r.status_code == 200
        data = r.json()
        if data["job"]["status"] in ("completed", "failed"):
            assert data["job"]["status"] == "completed", \
                f"Export failed: {data['job']}"
            return data, job_id
        time.sleep(1)
    raise TimeoutError("Export did not complete in time")


def _download_first_zip(client, export_data):
    """Download the first zip file from an export and return a ZipFile object."""
    files = export_data["files"]
    assert len(files) >= 1, "Export produced no files"
    r = client.get(files[0]["download_url"])
    assert r.status_code == 200
    return zipfile.ZipFile(io.BytesIO(r.content), "r")


# ── Test Class 1: Secure Gallery Grid (server provides dimensions) ─────────

class TestSecureGalleryGridData:
    """Verify the gallery items API returns width, height, and media_type
    so the client can render a proper justified grid layout."""

    def test_gallery_item_has_dimensions(self, user_client):
        """After adding a photo to a secure gallery, the items list must
        include width and height fields from the original photo."""
        jpeg = generate_test_jpeg(width=160, height=90)
        photo = user_client.upload_photo("landscape.jpg", content=jpeg)
        photo_id = photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("Dimension Test")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        time.sleep(2)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) >= 1
        item = items[0]
        assert "width" in item, f"Item missing 'width' field: {item}"
        assert "height" in item, f"Item missing 'height' field: {item}"
        assert item["width"] > 0, f"width should be positive: {item['width']}"
        assert item["height"] > 0, f"height should be positive: {item['height']}"

    def test_gallery_item_has_media_type(self, user_client):
        """Gallery items must include media_type so the client can differentiate
        photos, videos, and GIFs for rendering."""
        jpeg = generate_test_jpeg(width=100, height=100)
        photo = user_client.upload_photo("square.jpg", content=jpeg)
        photo_id = photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("MediaType Test")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        time.sleep(2)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) >= 1
        item = items[0]
        assert "media_type" in item, f"Item missing 'media_type' field: {item}"
        assert item["media_type"] == "photo", \
            f"Expected media_type='photo', got '{item.get('media_type')}'"

    def test_portrait_photo_aspect_ratio(self, user_client):
        """A portrait photo in the secure gallery should report width < height."""
        jpeg = generate_test_jpeg(width=90, height=160)
        photo = user_client.upload_photo("portrait.jpg", content=jpeg)
        photo_id = photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("Portrait Test")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        time.sleep(2)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) >= 1
        item = items[0]
        assert item["width"] < item["height"], \
            f"Portrait should have width < height, got {item['width']}x{item['height']}"

    def test_landscape_photo_aspect_ratio(self, user_client):
        """A landscape photo in the secure gallery should report width > height."""
        jpeg = generate_test_jpeg(width=160, height=90)
        photo = user_client.upload_photo("landscape.jpg", content=jpeg)
        photo_id = photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("Landscape Test")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        time.sleep(2)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) >= 1
        item = items[0]
        assert item["width"] > item["height"], \
            f"Landscape should have width > height, got {item['width']}x{item['height']}"


# ── Test Class 2: Export excludes thumbnails ───────────────────────────────

class TestExportNoThumbnails:
    """Library export zips must NOT contain thumbnail blobs."""

    def test_export_no_thumbnails_folder(self, user_client, admin_client):
        """After uploading photos, the export zip should have no thumbnails/ dir."""
        jpeg = generate_test_jpeg(width=100, height=100)
        user_client.upload_photo("thumb_test.jpg", content=jpeg)
        _trigger_and_wait_for_encryption(user_client, admin_client)
        time.sleep(2)

        data, job_id = _poll_export_completion(user_client)

        with _download_first_zip(user_client, data) as zf:
            names = zf.namelist()
            thumb_entries = [n for n in names if "thumbnail" in n.lower()]
            assert len(thumb_entries) == 0, \
                f"Export should not contain thumbnails, found: {thumb_entries}"

        user_client.delete(f"/api/export/{job_id}")

    def test_export_no_thumbnail_blob_types(self, user_client, admin_client):
        """Upload a photo (which creates thumbnail via migration) and verify
        thumbnails are excluded from the export ZIP."""
        jpeg = generate_test_jpeg(width=50, height=50)
        user_client.upload_photo("thumb_blob_test.jpg", content=jpeg)
        _trigger_and_wait_for_encryption(user_client, admin_client)
        time.sleep(2)

        data, job_id = _poll_export_completion(user_client)

        with _download_first_zip(user_client, data) as zf:
            names = zf.namelist()
            photo_entries = [n for n in names if n.startswith("photos/")]
            thumb_entries = [n for n in names
                            if "thumbnail" in n.lower() and n != "manifest.json"]
            assert len(photo_entries) >= 1, \
                f"Export should contain at least 1 photo, entries: {names}"
            assert len(thumb_entries) == 0, \
                f"Export should not contain any thumbnail blobs, found: {thumb_entries}"

        user_client.delete(f"/api/export/{job_id}")

    def test_export_only_media_in_photos_folder(self, user_client, admin_client):
        """Photos folder should only contain original media files."""
        user_client.upload_photo("media_only.jpg",
                                content=generate_test_jpeg(width=30, height=30))
        _trigger_and_wait_for_encryption(user_client, admin_client)
        time.sleep(2)

        data, job_id = _poll_export_completion(user_client)

        with _download_first_zip(user_client, data) as zf:
            names = zf.namelist()
            photo_files = [n for n in names if n.startswith("photos/")]
            assert len(photo_files) == 1, \
                f"Expected exactly 1 file in photos/, got {len(photo_files)}: {photo_files}"

        user_client.delete(f"/api/export/{job_id}")


# ── Test Class 3: Export metadata readable (no .bin) ───────────────────────

class TestExportMetadataReadable:
    """Metadata blobs (album_manifest etc.) should be packaged as readable
    JSON in a metadata/ subfolder, never as opaque .bin files."""

    def test_export_no_bin_files(self, user_client, admin_client):
        """Export must not contain .bin files for known blob types."""
        user_client.upload_photo("nobin_test.jpg",
                                content=generate_test_jpeg(width=30, height=30))
        _trigger_and_wait_for_encryption(user_client, admin_client)

        manifest_content = json.dumps({
            "album_name": "Test Album",
            "photos": ["photo1", "photo2"],
        }).encode()
        user_client.upload_blob("album_manifest",
                               _encrypt_for_server(manifest_content))

        time.sleep(2)

        data, job_id = _poll_export_completion(user_client)

        with _download_first_zip(user_client, data) as zf:
            names = zf.namelist()
            bin_files = [n for n in names if n.endswith(".bin")]
            assert len(bin_files) == 0, \
                f"Export should not contain .bin files, found: {bin_files}"

        user_client.delete(f"/api/export/{job_id}")

    def test_export_album_manifest_in_metadata_folder(self, user_client, admin_client):
        """Album manifest blobs should appear under metadata/ subfolder."""
        manifest_content = json.dumps({
            "album_name": "My Album",
            "photos": ["a", "b"],
        }).encode()
        user_client.upload_blob("album_manifest",
                               _encrypt_for_server(manifest_content))

        # Also upload a photo so the export has real content
        user_client.upload_photo("meta_test.jpg",
                                content=generate_test_jpeg(width=30, height=30))
        _trigger_and_wait_for_encryption(user_client, admin_client)
        time.sleep(2)

        data, job_id = _poll_export_completion(user_client)

        with _download_first_zip(user_client, data) as zf:
            names = zf.namelist()
            metadata_files = [n for n in names if n.startswith("metadata/")]
            assert len(metadata_files) >= 1, \
                f"Expected at least 1 file in metadata/, got: {names}"

            for mf in metadata_files:
                content = zf.read(mf)
                parsed = json.loads(content)
                assert isinstance(parsed, dict), \
                    f"Metadata file {mf} should be a JSON object"

        user_client.delete(f"/api/export/{job_id}")

    def test_export_manifest_preserves_content(self, user_client, admin_client):
        """The album manifest content should be preserved accurately."""
        original = {
            "album_name": "Content Check",
            "photos": ["p1", "p2", "p3"],
            "created_at": "2026-01-01T00:00:00Z",
        }
        manifest_content = json.dumps(original).encode()
        user_client.upload_blob("album_manifest",
                               _encrypt_for_server(manifest_content))

        # Also upload a photo so the export has real content
        user_client.upload_photo("preserve_test.jpg",
                                content=generate_test_jpeg(width=30, height=30))
        _trigger_and_wait_for_encryption(user_client, admin_client)
        time.sleep(2)

        data, job_id = _poll_export_completion(user_client)

        with _download_first_zip(user_client, data) as zf:
            names = zf.namelist()
            metadata_files = [n for n in names if n.startswith("metadata/")]
            assert len(metadata_files) >= 1

            content = json.loads(zf.read(metadata_files[0]))
            assert content["album_name"] == original["album_name"]
            assert content["photos"] == original["photos"]

        user_client.delete(f"/api/export/{job_id}")


# ── Test Class 4: Secure Gallery GIF media_type ───────────────────────────

class TestSecureGalleryGifSupport:
    """Secure gallery items must expose media_type for GIF autoplay support."""

    def test_gif_media_type_in_gallery_items(self, user_client):
        """When a GIF is added to a secure gallery, the item's media_type
        should be 'gif' so the client can trigger autoplay."""
        gif_data = generate_test_gif(width=20, height=20, frames=3)
        photo = user_client.upload_photo(
            "animated.gif", content=gif_data, mime_type="image/gif"
        )
        photo_id = photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("GIF Test")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        time.sleep(2)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) >= 1
        item = items[0]
        assert "media_type" in item, f"Item missing 'media_type': {item}"
        assert item["media_type"] == "gif", \
            f"Expected media_type='gif', got '{item.get('media_type')}'"

    def test_gif_dimensions_in_gallery_items(self, user_client):
        """GIF items should also have width and height for grid layout."""
        gif_data = generate_test_gif(width=40, height=20, frames=2)
        photo = user_client.upload_photo(
            "wide_gif.gif", content=gif_data, mime_type="image/gif"
        )
        photo_id = photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("GIF Dims")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        time.sleep(2)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) >= 1
        item = items[0]
        assert item.get("width", 0) > 0
        assert item.get("height", 0) > 0
        assert item["width"] > item["height"], \
            f"Wide GIF should have width > height: {item['width']}x{item['height']}"

    def test_mixed_media_types_in_gallery(self, user_client):
        """Gallery with both photos and GIFs should report correct media_type
        for each item."""
        jpeg = generate_test_jpeg(width=100, height=100)
        photo = user_client.upload_photo("regular.jpg", content=jpeg)
        photo_id = photo["photo_id"]

        gif_data = generate_test_gif(width=30, height=30, frames=2)
        gif_photo = user_client.upload_photo(
            "anim.gif", content=gif_data, mime_type="image/gif"
        )
        gif_photo_id = gif_photo["photo_id"]

        _wait_for_encryption(user_client)

        gal = user_client.create_secure_gallery("Mixed Media")
        gallery_id = gal["gallery_id"]
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]

        user_client.add_secure_gallery_item(gallery_id, photo_id, token)
        user_client.add_secure_gallery_item(gallery_id, gif_photo_id, token)
        time.sleep(3)
        _wait_for_encryption(user_client)

        items = user_client.list_secure_gallery_items(gallery_id, token)["items"]
        assert len(items) == 2

        media_types = {item["media_type"] for item in items}
        assert "photo" in media_types, f"Expected 'photo' in media_types: {media_types}"
        assert "gif" in media_types, f"Expected 'gif' in media_types: {media_types}"
