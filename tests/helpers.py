"""
Shared helpers for E2E tests: API client wrapper, test data generators, assertions.
"""

import hashlib
import io
import json
import os
import random
import string
import time
from typing import Optional

import requests


_client_counter = 0


class APIClient:
    """Thin wrapper around requests for the Simple Photos API."""

    def __init__(self, base_url: str, api_key: Optional[str] = None):
        global _client_counter
        _client_counter += 1
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        self.access_token: Optional[str] = None
        self.refresh_token: Optional[str] = None
        self.api_key = api_key
        # Each client gets a unique fake IP so that rate-limiting (which keys
        # on the X-Forwarded-For header when trust_proxy=true) doesn't lump
        # every test together under 127.0.0.1.
        self._fake_ip = f"10.0.{(_client_counter >> 8) & 0xFF}.{_client_counter & 0xFF}"

    def _url(self, path: str) -> str:
        if not path.startswith("/"):
            path = "/" + path
        return f"{self.base_url}{path}"

    def _auth_headers(self) -> dict:
        h = {"X-Forwarded-For": self._fake_ip}
        if self.access_token:
            h["Authorization"] = f"Bearer {self.access_token}"
        if self.api_key:
            h["X-API-Key"] = self.api_key
        return h

    # ── Generic HTTP verbs ───────────────────────────────────────────

    def get(self, path: str, params=None, headers=None, **kwargs) -> requests.Response:
        h = {**self._auth_headers(), **(headers or {})}
        return self.session.get(self._url(path), params=params, headers=h, **kwargs)

    def post(self, path: str, json_data=None, data=None, headers=None, **kwargs) -> requests.Response:
        h = {**self._auth_headers(), **(headers or {})}
        return self.session.post(self._url(path), json=json_data, data=data, headers=h, **kwargs)

    def put(self, path: str, json_data=None, headers=None, **kwargs) -> requests.Response:
        h = {**self._auth_headers(), **(headers or {})}
        return self.session.put(self._url(path), json=json_data, headers=h, **kwargs)

    def delete(self, path: str, json_data=None, headers=None, **kwargs) -> requests.Response:
        h = {**self._auth_headers(), **(headers or {})}
        return self.session.delete(self._url(path), json=json_data, headers=h, **kwargs)

    # ── Auth helpers ─────────────────────────────────────────────────

    def setup_init(self, username: str, password: str) -> dict:
        """Initialize a fresh server with the first admin user."""
        r = self.post("/api/setup/init", json_data={"username": username, "password": password})
        r.raise_for_status()
        return r.json()

    def register(self, username: str, password: str) -> dict:
        r = self.post("/api/auth/register", json_data={"username": username, "password": password})
        r.raise_for_status()
        return r.json()

    def login(self, username: str, password: str) -> dict:
        r = self.post("/api/auth/login", json_data={"username": username, "password": password})
        r.raise_for_status()
        data = r.json()
        if "access_token" in data:
            self.access_token = data["access_token"]
            self.refresh_token = data.get("refresh_token")
        return data

    def refresh(self) -> dict:
        r = self.post("/api/auth/refresh", json_data={"refresh_token": self.refresh_token})
        r.raise_for_status()
        data = r.json()
        self.access_token = data["access_token"]
        self.refresh_token = data["refresh_token"]
        return data

    def logout(self) -> None:
        self.post("/api/auth/logout", json_data={"refresh_token": self.refresh_token})
        self.access_token = None
        self.refresh_token = None

    def change_password(self, current: str, new: str) -> requests.Response:
        return self.put("/api/auth/password", json_data={"current_password": current, "new_password": new})

    # ── Photo helpers ────────────────────────────────────────────────

    def upload_photo(self, filename: str = "test.jpg", content: bytes = None,
                     mime_type: str = "image/jpeg") -> dict:
        """Upload a photo file via /api/photos/upload."""
        if content is None:
            content = generate_test_jpeg()
        h = {
            **self._auth_headers(),
            "X-Filename": filename,
            "X-Mime-Type": mime_type,
            "Content-Type": "application/octet-stream",
        }
        r = self.session.post(self._url("/api/photos/upload"), data=content, headers=h)
        r.raise_for_status()
        return r.json()

    def list_photos(self, **params) -> dict:
        r = self.get("/api/photos", params=params)
        r.raise_for_status()
        return r.json()

    def get_photo_file(self, photo_id: str) -> requests.Response:
        return self.get(f"/api/photos/{photo_id}/file")

    def get_photo_thumb(self, photo_id: str) -> requests.Response:
        return self.get(f"/api/photos/{photo_id}/thumb")

    def favorite_photo(self, photo_id: str) -> dict:
        r = self.put(f"/api/photos/{photo_id}/favorite")
        r.raise_for_status()
        return r.json()

    def crop_photo(self, photo_id: str, crop_metadata: str) -> dict:
        r = self.put(f"/api/photos/{photo_id}/crop", json_data={"crop_metadata": crop_metadata})
        r.raise_for_status()
        return r.json()

    def duplicate_photo(self, photo_id: str, crop_metadata: str = None) -> dict:
        body = {}
        if crop_metadata:
            body["crop_metadata"] = crop_metadata
        r = self.post(f"/api/photos/{photo_id}/duplicate", json_data=body)
        r.raise_for_status()
        return r.json()

    def delete_photo(self, blob_id: str, filename: str = "test.jpg",
                     mime_type: str = "image/jpeg") -> requests.Response:
        """Soft-delete a photo/blob to trash via the encrypted blob trash endpoint.

        In this encrypted system, there is no DELETE /api/photos/{id}.
        Photos are deleted by trashing the underlying blob via
        POST /api/blobs/{id}/trash.  ``blob_id`` is the id returned by
        ``upload_blob()`` or the ``blob_id`` field from ``upload_blob_photo()``.
        """
        return self.post(f"/api/blobs/{blob_id}/trash", json_data={
            "filename": filename,
            "mime_type": mime_type,
        })

    def upload_blob_photo(self, filename: str = "test.jpg",
                          content: bytes = None,
                          blob_type: str = "photo") -> dict:
        """Upload content as a blob (the encrypted-client workflow).

        Returns a dict with ``blob_id`` (for trash/restore) and
        ``filename`` for convenience.  This is the workflow the web
        client uses — photos are uploaded as encrypted blobs.
        """
        if content is None:
            content = generate_test_jpeg()
        blob = self.upload_blob(blob_type, content)
        return {
            "blob_id": blob["blob_id"],
            "filename": filename,
            **blob,
        }

    def encrypted_sync(self, **params) -> dict:
        r = self.get("/api/photos/encrypted-sync", params=params)
        r.raise_for_status()
        return r.json()

    # ── Edit copies ──────────────────────────────────────────────────

    def create_edit_copy(self, photo_id: str, name: str = None, edit_metadata: str = "{}") -> dict:
        body = {"edit_metadata": edit_metadata}
        if name:
            body["name"] = name
        r = self.post(f"/api/photos/{photo_id}/copies", json_data=body)
        r.raise_for_status()
        return r.json()

    def list_edit_copies(self, photo_id: str) -> dict:
        r = self.get(f"/api/photos/{photo_id}/copies")
        r.raise_for_status()
        return r.json()

    def delete_edit_copy(self, photo_id: str, copy_id: str) -> requests.Response:
        return self.delete(f"/api/photos/{photo_id}/copies/{copy_id}")

    # ── Blob helpers ─────────────────────────────────────────────────

    def upload_blob(self, blob_type: str = "photo", content: bytes = None,
                    client_hash: str = None, content_hash: str = None) -> dict:
        if content is None:
            content = generate_random_bytes(1024)
        if client_hash is None:
            client_hash = hashlib.sha256(content).hexdigest()
        h = {
            **self._auth_headers(),
            "x-blob-type": blob_type,
            "x-client-hash": client_hash,
            "Content-Type": "application/octet-stream",
        }
        if content_hash:
            h["x-content-hash"] = content_hash
        r = self.session.post(self._url("/api/blobs"), data=content, headers=h)
        r.raise_for_status()
        return r.json()

    def list_blobs(self, **params) -> dict:
        r = self.get("/api/blobs", params=params)
        r.raise_for_status()
        return r.json()

    def download_blob(self, blob_id: str) -> requests.Response:
        return self.get(f"/api/blobs/{blob_id}")

    def delete_blob(self, blob_id: str) -> requests.Response:
        return self.delete(f"/api/blobs/{blob_id}")

    # ── Trash helpers ────────────────────────────────────────────────

    def list_trash(self, **params) -> dict:
        r = self.get("/api/trash", params=params)
        r.raise_for_status()
        return r.json()

    def restore_trash(self, trash_id: str) -> requests.Response:
        return self.post(f"/api/trash/{trash_id}/restore")

    def permanent_delete_trash(self, trash_id: str) -> requests.Response:
        return self.delete(f"/api/trash/{trash_id}")

    def empty_trash(self) -> dict:
        r = self.delete("/api/trash")
        r.raise_for_status()
        return r.json()

    def soft_delete_blob(self, blob_id: str, filename: str = "test.jpg",
                         mime_type: str = "image/jpeg", size_bytes: int = None,
                         **kwargs) -> dict:
        body = {"filename": filename, "mime_type": mime_type, **kwargs}
        if size_bytes is not None:
            body["size_bytes"] = size_bytes
        r = self.post(f"/api/blobs/{blob_id}/trash", json_data=body)
        r.raise_for_status()
        return r.json()

    # ── Secure gallery helpers ───────────────────────────────────────

    def list_secure_galleries(self) -> dict:
        r = self.get("/api/galleries/secure")
        r.raise_for_status()
        return r.json()

    def create_secure_gallery(self, name: str) -> dict:
        r = self.post("/api/galleries/secure", json_data={"name": name})
        r.raise_for_status()
        return r.json()

    def unlock_secure_gallery(self, password: str) -> dict:
        r = self.post("/api/galleries/secure/unlock", json_data={"password": password})
        r.raise_for_status()
        return r.json()

    def get_secure_gallery_blob_ids(self) -> dict:
        r = self.get("/api/galleries/secure/blob-ids")
        r.raise_for_status()
        return r.json()

    def add_secure_gallery_item(self, gallery_id: str, blob_id: str,
                                gallery_token: str) -> dict:
        r = self.post(
            f"/api/galleries/secure/{gallery_id}/items",
            json_data={"blob_id": blob_id},
            headers={"x-gallery-token": gallery_token},
        )
        r.raise_for_status()
        return r.json()

    def list_secure_gallery_items(self, gallery_id: str, gallery_token: str) -> dict:
        r = self.get(
            f"/api/galleries/secure/{gallery_id}/items",
            headers={"x-gallery-token": gallery_token},
        )
        r.raise_for_status()
        return r.json()

    def delete_secure_gallery(self, gallery_id: str) -> requests.Response:
        return self.delete(f"/api/galleries/secure/{gallery_id}")

    # ── Shared album helpers ─────────────────────────────────────────

    def list_shared_albums(self) -> list:
        r = self.get("/api/sharing/albums")
        r.raise_for_status()
        return r.json()

    def create_shared_album(self, name: str) -> dict:
        r = self.post("/api/sharing/albums", json_data={"name": name})
        r.raise_for_status()
        return r.json()

    def delete_shared_album(self, album_id: str) -> requests.Response:
        return self.delete(f"/api/sharing/albums/{album_id}")

    def list_album_members(self, album_id: str) -> list:
        r = self.get(f"/api/sharing/albums/{album_id}/members")
        r.raise_for_status()
        return r.json()

    def add_album_member(self, album_id: str, user_id: str) -> dict:
        r = self.post(f"/api/sharing/albums/{album_id}/members", json_data={"user_id": user_id})
        r.raise_for_status()
        return r.json()

    def remove_album_member(self, album_id: str, user_id: str) -> requests.Response:
        return self.delete(f"/api/sharing/albums/{album_id}/members/{user_id}")

    def list_album_photos(self, album_id: str) -> list:
        r = self.get(f"/api/sharing/albums/{album_id}/photos")
        r.raise_for_status()
        return r.json()

    def add_album_photo(self, album_id: str, photo_ref: str,
                        ref_type: str = "photo") -> dict:
        r = self.post(
            f"/api/sharing/albums/{album_id}/photos",
            json_data={"photo_ref": photo_ref, "ref_type": ref_type},
        )
        r.raise_for_status()
        return r.json()

    def remove_album_photo(self, album_id: str, photo_id: str) -> requests.Response:
        return self.delete(f"/api/sharing/albums/{album_id}/photos/{photo_id}")

    def list_sharing_users(self) -> list:
        r = self.get("/api/sharing/users")
        r.raise_for_status()
        return r.json()

    # ── Tags ─────────────────────────────────────────────────────────

    def list_tags(self) -> dict:
        r = self.get("/api/tags")
        r.raise_for_status()
        return r.json()

    def add_tag(self, photo_id: str, tag: str) -> requests.Response:
        return self.post(f"/api/photos/{photo_id}/tags", json_data={"tag": tag})

    def remove_tag(self, photo_id: str, tag: str) -> requests.Response:
        return self.delete(f"/api/photos/{photo_id}/tags", json_data={"tag": tag})

    def get_photo_tags(self, photo_id: str) -> dict:
        r = self.get(f"/api/photos/{photo_id}/tags")
        r.raise_for_status()
        return r.json()

    def search(self, query: str, limit: int = 50) -> dict:
        r = self.get("/api/search", params={"q": query, "limit": limit})
        r.raise_for_status()
        return r.json()

    # ── Admin helpers ────────────────────────────────────────────────

    def admin_create_user(self, username: str, password: str, role: str = "user") -> dict:
        r = self.post("/api/admin/users", json_data={"username": username, "password": password, "role": role})
        r.raise_for_status()
        return r.json()

    def admin_list_users(self) -> list:
        r = self.get("/api/admin/users")
        r.raise_for_status()
        return r.json()

    def admin_delete_user(self, user_id: str) -> requests.Response:
        return self.delete(f"/api/admin/users/{user_id}")

    def admin_store_encryption_key(self, key_hex: str) -> dict:
        r = self.post("/api/admin/encryption/store-key", json_data={"key_hex": key_hex})
        r.raise_for_status()
        return r.json()

    # ── Backup admin helpers ─────────────────────────────────────────

    def admin_list_backup_servers(self) -> dict:
        r = self.get("/api/admin/backup/servers")
        r.raise_for_status()
        return r.json()

    def admin_add_backup_server(self, name: str, address: str,
                                api_key: str = None, sync_hours: int = 1) -> dict:
        body = {"name": name, "address": address, "sync_frequency_hours": sync_hours}
        if api_key:
            body["api_key"] = api_key
        r = self.post("/api/admin/backup/servers", json_data=body)
        r.raise_for_status()
        return r.json()

    def admin_trigger_sync(self, server_id: str) -> dict:
        r = self.post(f"/api/admin/backup/servers/{server_id}/sync")
        r.raise_for_status()
        return r.json()

    def admin_get_sync_logs(self, server_id: str) -> list:
        r = self.get(f"/api/admin/backup/servers/{server_id}/logs")
        r.raise_for_status()
        return r.json()

    def admin_backup_server_status(self, server_id: str) -> dict:
        r = self.get(f"/api/admin/backup/servers/{server_id}/status")
        r.raise_for_status()
        return r.json()

    def admin_recover_from_backup(self, server_id: str) -> dict:
        r = self.post(f"/api/admin/backup/servers/{server_id}/recover")
        r.raise_for_status()
        return r.json()

    def admin_get_backup_mode(self) -> dict:
        r = self.get("/api/admin/backup/mode")
        r.raise_for_status()
        return r.json()

    def admin_set_backup_mode(self, mode: str) -> dict:
        r = self.post("/api/admin/backup/mode", json_data={"mode": mode})
        r.raise_for_status()
        return r.json()

    def admin_get_backup_photos(self, server_id: str) -> list:
        r = self.get(f"/api/admin/backup/servers/{server_id}/photos")
        r.raise_for_status()
        return r.json()

    def admin_force_sync(self) -> dict:
        r = self.post("/api/admin/backup/force-sync")
        r.raise_for_status()
        return r.json()

    def admin_trigger_autoscan(self) -> dict:
        r = self.post("/api/admin/photos/auto-scan")
        r.raise_for_status()
        return r.json()

    def admin_trigger_scan(self) -> dict:
        """Trigger the user-scoped filesystem scan (POST /api/admin/photos/scan)."""
        r = self.post("/api/admin/photos/scan")
        r.raise_for_status()
        return r.json()

    # ── Backup serve endpoints (X-API-Key auth) ─────────────────────

    def backup_list(self) -> list:
        r = self.get("/api/backup/list", headers={"X-API-Key": self.api_key})
        r.raise_for_status()
        return r.json()

    def backup_list_trash(self) -> list:
        r = self.get("/api/backup/list-trash", headers={"X-API-Key": self.api_key})
        r.raise_for_status()
        return r.json()

    def backup_list_users(self) -> list:
        r = self.get("/api/backup/list-users", headers={"X-API-Key": self.api_key})
        r.raise_for_status()
        return r.json()

    def backup_list_blobs(self) -> list:
        r = self.get("/api/backup/list-blobs", headers={"X-API-Key": self.api_key})
        r.raise_for_status()
        return r.json()

    # ── Settings / diagnostics ───────────────────────────────────────

    def storage_stats(self) -> dict:
        r = self.get("/api/settings/storage-stats")
        r.raise_for_status()
        return r.json()

    def health(self) -> dict:
        r = self.session.get(self._url("/health"))
        r.raise_for_status()
        return r.json()

    def setup_status(self) -> dict:
        r = self.get("/api/setup/status")
        r.raise_for_status()
        return r.json()


# ── Test data generators ─────────────────────────────────────────────

def generate_test_jpeg(width: int = 2, height: int = 2) -> bytes:
    """Generate a minimal valid JPEG file for upload tests."""
    # Minimal JPEG: SOI + APP0 + minimal scan data + EOI
    # This is a valid JPEG that image parsers can read.
    return bytes([
        0xFF, 0xD8,  # SOI
        0xFF, 0xE0,  # APP0 marker
        0x00, 0x10,  # Length 16
        0x4A, 0x46, 0x49, 0x46, 0x00,  # "JFIF\0"
        0x01, 0x01,  # Version 1.1
        0x00,        # Aspect ratio units: none
        0x00, 0x01,  # X density: 1
        0x00, 0x01,  # Y density: 1
        0x00, 0x00,  # No thumbnail
        0xFF, 0xDB,  # DQT marker
        0x00, 0x43,  # Length 67
        0x00,        # Table 0, 8-bit precision
    ] + [0x01] * 64 + [  # 64 quantization values
        0xFF, 0xC0,  # SOF0 marker
        0x00, 0x0B,  # Length 11
        0x08,        # 8-bit precision
        0x00, height & 0xFF,  # Height
        0x00, width & 0xFF,   # Width
        0x01,        # 1 component
        0x01,        # Component ID
        0x11,        # Sampling factors
        0x00,        # Quant table 0
        0xFF, 0xC4,  # DHT marker
        0x00, 0x1F,  # Length 31
        0x00,        # DC table 0
        0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        0x08, 0x09, 0x0A, 0x0B,
        0xFF, 0xDA,  # SOS marker
        0x00, 0x08,  # Length
        0x01,        # 1 component
        0x01,        # Component ID
        0x00,        # DC/AC table selectors
        0x00, 0x3F, 0x00,  # Spectral selection
        0x7B, 0x40,  # Compressed data (minimal)
        0xFF, 0xD9,  # EOI
    ])


def generate_test_png() -> bytes:
    """Generate a minimal valid 1x1 red PNG."""
    import struct
    import zlib

    def chunk(chunk_type: bytes, data: bytes) -> bytes:
        c = chunk_type + data
        crc = struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)
        return struct.pack(">I", len(data)) + c + crc

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
    raw = zlib.compress(b"\x00\xff\x00\x00")  # filter byte + RGB
    idat = chunk(b"IDAT", raw)
    iend = chunk(b"IEND", b"")
    return sig + ihdr + idat + iend


def generate_random_bytes(size: int = 1024) -> bytes:
    """Generate random bytes for blob uploads."""
    return os.urandom(size)


def random_username(prefix: str = "testuser") -> str:
    suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))
    return f"{prefix}{suffix}"


def random_password() -> str:
    """Generate a password that meets complexity requirements (8+ chars)."""
    return "Test" + "".join(random.choices(string.ascii_letters + string.digits, k=12)) + "!1"


def unique_filename(ext: str = "jpg") -> str:
    return f"test_{int(time.time() * 1000)}_{random.randint(1000, 9999)}.{ext}"


# ── Wait helpers ─────────────────────────────────────────────────────

def wait_for_server(base_url: str, timeout: float = 30.0, interval: float = 0.5):
    """Block until the server's /health endpoint responds 200."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = requests.get(f"{base_url}/health", timeout=2)
            if r.status_code == 200:
                return
        except requests.ConnectionError:
            pass
        time.sleep(interval)
    raise TimeoutError(f"Server at {base_url} did not become ready within {timeout}s")


def wait_for_sync(client: APIClient, server_id: str, timeout: float = 60.0,
                  interval: float = 2.0):
    """Wait for a sync operation to complete (status != 'running')."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        logs = client.admin_get_sync_logs(server_id)
        if logs:
            latest = logs[0] if isinstance(logs, list) else logs
            status = latest.get("status", "")
            if status in ("success", "error", "completed"):
                return latest
        time.sleep(interval)
    raise TimeoutError(f"Sync did not complete within {timeout}s")


def assert_photo_in_list(photos: list, photo_id: str, msg: str = ""):
    """Assert that a photo ID exists in a list of photo records."""
    ids = [p["id"] for p in photos]
    assert photo_id in ids, f"Photo {photo_id} not found in list. {msg}\nIDs: {ids}"


def assert_photo_not_in_list(photos: list, photo_id: str, msg: str = ""):
    """Assert that a photo ID does NOT exist in a list of photo records."""
    ids = [p["id"] for p in photos]
    assert photo_id not in ids, f"Photo {photo_id} unexpectedly found in list. {msg}"
