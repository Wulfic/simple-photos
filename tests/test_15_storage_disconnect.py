"""
E2E tests for storage disconnect handling and automatic reconnection.

Verifies that when the storage backend (network drive) becomes unreachable:
1. The server returns 503 for storage-dependent operations
2. The /health endpoint reports "degraded" status
3. Non-storage operations (auth, list, health) still work
4. When storage reconnects, the server auto-recovers within ~10s
5. Uploads and downloads resume normally after reconnection
"""

import os
import stat
import time

import pytest
import requests

# ── Fixtures ─────────────────────────────────────────────────────────


@pytest.fixture
def storage_client(primary_server, primary_admin):
    """
    Create a regular user with an uploaded blob, then return the client
    and blob_id for disconnect testing.
    """
    from helpers import APIClient, random_username, generate_random_bytes

    username = random_username("storage_")
    primary_admin.admin_create_user(username, "StorageTest123!", role="user")

    client = APIClient(primary_server.base_url)
    client.login(username, "StorageTest123!")
    client.username = username

    # Upload a blob while storage is healthy
    content = generate_random_bytes(2048)
    blob = client.upload_blob("photo", content)

    client._test_blob_id = blob["blob_id"]
    client._test_content = content
    return client


# ── Helpers ──────────────────────────────────────────────────────────


def _make_storage_unavailable(server):
    """
    Simulate a network drive disconnect by revoking all permissions on the
    storage root directory.  The storage health probe will fail on the
    next 10s tick.
    """
    storage_root = server.storage_root
    # Save original permissions for restoration
    original_mode = os.stat(storage_root).st_mode
    # Remove all permissions — makes read/write/stat fail
    os.chmod(storage_root, 0o000)
    return original_mode


def _make_storage_available(server, original_mode):
    """Restore storage directory permissions (simulate reconnect)."""
    os.chmod(server.storage_root, original_mode)


def _wait_for_storage_status(client, expected_status, timeout=25):
    """
    Poll /health until the storage field matches the expected status.
    The health monitor probes every 10s, so we allow up to 25s.
    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = client.session.get(f"{client.base_url}/health", timeout=3)
            data = r.json()
            if data.get("storage") == expected_status:
                return data
        except Exception:
            pass
        time.sleep(1)
    raise TimeoutError(
        f"Storage status did not become '{expected_status}' within {timeout}s"
    )


# ── Tests ────────────────────────────────────────────────────────────


class TestStorageDisconnect:
    """Tests for graceful storage disconnect handling."""

    def test_health_reports_ok_when_storage_connected(self, storage_client):
        """Health endpoint reports 'ok' and 'connected' under normal conditions."""
        r = storage_client.session.get(f"{storage_client.base_url}/health")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "ok"
        assert data["storage"] == "connected"

    def test_upload_works_when_storage_connected(self, storage_client):
        """Blob upload succeeds when storage is healthy."""
        from helpers import generate_random_bytes

        content = generate_random_bytes(1024)
        blob = storage_client.upload_blob("photo", content)
        assert "blob_id" in blob

    def test_download_works_when_storage_connected(self, storage_client):
        """Blob download succeeds when storage is healthy."""
        r = storage_client.download_blob(storage_client._test_blob_id)
        assert r.status_code == 200
        assert r.content == storage_client._test_content

    def test_disconnect_returns_503_on_upload(
        self, storage_client, primary_server
    ):
        """
        When storage disconnects, blob uploads return 503 Service Unavailable.
        After reconnect, uploads succeed again.
        """
        from helpers import generate_random_bytes

        original_mode = _make_storage_unavailable(primary_server)
        try:
            # Wait for the health monitor to detect the disconnect
            _wait_for_storage_status(storage_client, "disconnected")

            # Upload should now fail with 503
            content = generate_random_bytes(512)
            import hashlib

            client_hash = hashlib.sha256(content).hexdigest()
            h = {
                **storage_client._auth_headers(),
                "x-blob-type": "photo",
                "x-client-hash": client_hash,
                "Content-Type": "application/octet-stream",
            }
            r = storage_client.session.post(
                f"{storage_client.base_url}/api/blobs", data=content, headers=h
            )
            assert r.status_code == 503, (
                f"Expected 503 during storage disconnect, got {r.status_code}: {r.text}"
            )
            data = r.json()
            assert "storage" in data["error"].lower() or "unavailable" in data["error"].lower()
        finally:
            _make_storage_available(primary_server, original_mode)

        # Wait for reconnection
        _wait_for_storage_status(storage_client, "connected")

        # Upload should work again
        content = generate_random_bytes(512)
        blob = storage_client.upload_blob("photo", content)
        assert "blob_id" in blob

    def test_disconnect_returns_503_on_download(
        self, storage_client, primary_server
    ):
        """
        When storage disconnects, blob downloads return 503 Service Unavailable.
        After reconnect, downloads succeed again.
        """
        original_mode = _make_storage_unavailable(primary_server)
        try:
            _wait_for_storage_status(storage_client, "disconnected")

            # Download should fail with 503
            r = storage_client.download_blob(storage_client._test_blob_id)
            assert r.status_code == 503, (
                f"Expected 503 during storage disconnect, got {r.status_code}: {r.text}"
            )
        finally:
            _make_storage_available(primary_server, original_mode)

        # Wait for reconnection
        _wait_for_storage_status(storage_client, "connected")

        # Download should work again
        r = storage_client.download_blob(storage_client._test_blob_id)
        assert r.status_code == 200
        assert r.content == storage_client._test_content

    def test_disconnect_returns_503_on_delete(
        self, storage_client, primary_server
    ):
        """
        When storage disconnects, blob delete returns 503.
        """
        from helpers import generate_random_bytes

        # Upload a blob to delete later
        content = generate_random_bytes(256)
        blob = storage_client.upload_blob("photo", content)
        blob_id = blob["blob_id"]

        original_mode = _make_storage_unavailable(primary_server)
        try:
            _wait_for_storage_status(storage_client, "disconnected")

            # Delete should fail with 503
            r = storage_client.delete_blob(blob_id)
            assert r.status_code == 503
        finally:
            _make_storage_available(primary_server, original_mode)

        # Wait for reconnection then delete should work
        _wait_for_storage_status(storage_client, "connected")
        r = storage_client.delete_blob(blob_id)
        assert r.status_code == 204

    def test_health_reports_degraded_during_disconnect(
        self, storage_client, primary_server
    ):
        """
        /health reports status='degraded' and storage='disconnected'
        when the storage backend is unreachable.
        """
        original_mode = _make_storage_unavailable(primary_server)
        try:
            _wait_for_storage_status(storage_client, "disconnected")

            r = storage_client.session.get(f"{storage_client.base_url}/health")
            assert r.status_code == 200  # Health endpoint always returns 200
            data = r.json()
            assert data["status"] == "degraded"
            assert data["storage"] == "disconnected"
        finally:
            _make_storage_available(primary_server, original_mode)
            _wait_for_storage_status(storage_client, "connected")

    def test_auth_works_during_disconnect(
        self, storage_client, primary_server
    ):
        """
        Authentication (non-storage) operations continue to work during
        a storage disconnect — only storage I/O is affected.
        """
        original_mode = _make_storage_unavailable(primary_server)
        try:
            _wait_for_storage_status(storage_client, "disconnected")

            # Auth refresh should still work (it only touches DB, not storage)
            r = storage_client.post(
                "/api/auth/refresh",
                json_data={"refresh_token": storage_client.refresh_token},
            )
            assert r.status_code == 200
            data = r.json()
            storage_client.access_token = data["access_token"]
            storage_client.refresh_token = data["refresh_token"]

            # Blob listing should still work (it only queries DB)
            r = storage_client.get("/api/blobs")
            assert r.status_code == 200
        finally:
            _make_storage_available(primary_server, original_mode)
            _wait_for_storage_status(storage_client, "connected")

    def test_automatic_reconnection(
        self, storage_client, primary_server
    ):
        """
        After storage disconnects and then reconnects, the server
        automatically detects recovery within ~10 seconds and resumes
        normal operation without manual intervention.
        """
        from helpers import generate_random_bytes

        original_mode = _make_storage_unavailable(primary_server)
        try:
            _wait_for_storage_status(storage_client, "disconnected")

            # Verify we're in degraded state
            r = storage_client.session.get(f"{storage_client.base_url}/health")
            assert r.json()["status"] == "degraded"
        finally:
            _make_storage_available(primary_server, original_mode)

        # Time how long reconnection takes (should be < 15s given 10s interval)
        start = time.time()
        _wait_for_storage_status(storage_client, "connected", timeout=20)
        reconnect_time = time.time() - start
        print(f"\nStorage reconnection detected in {reconnect_time:.1f}s")

        # Health should be fully ok
        r = storage_client.session.get(f"{storage_client.base_url}/health")
        data = r.json()
        assert data["status"] == "ok"
        assert data["storage"] == "connected"

        # Full round-trip: upload then download
        content = generate_random_bytes(1024)
        blob = storage_client.upload_blob("photo", content)
        r = storage_client.download_blob(blob["blob_id"])
        assert r.status_code == 200
        assert r.content == content

    def test_repeated_disconnect_reconnect_cycles(
        self, storage_client, primary_server
    ):
        """
        The server handles multiple disconnect/reconnect cycles gracefully
        without accumulating state or leaking resources.
        """
        from helpers import generate_random_bytes

        for cycle in range(3):
            # Disconnect
            original_mode = _make_storage_unavailable(primary_server)
            try:
                _wait_for_storage_status(storage_client, "disconnected")

                # Verify 503 on upload
                import hashlib

                content = generate_random_bytes(256)
                client_hash = hashlib.sha256(content).hexdigest()
                h = {
                    **storage_client._auth_headers(),
                    "x-blob-type": "photo",
                    "x-client-hash": client_hash,
                    "Content-Type": "application/octet-stream",
                }
                r = storage_client.session.post(
                    f"{storage_client.base_url}/api/blobs",
                    data=content,
                    headers=h,
                )
                assert r.status_code == 503, f"Cycle {cycle}: expected 503, got {r.status_code}"
            finally:
                _make_storage_available(primary_server, original_mode)

            # Reconnect
            _wait_for_storage_status(storage_client, "connected")

            # Verify upload works after reconnect
            content = generate_random_bytes(256)
            blob = storage_client.upload_blob("photo", content)
            assert "blob_id" in blob, f"Cycle {cycle}: upload failed after reconnect"
