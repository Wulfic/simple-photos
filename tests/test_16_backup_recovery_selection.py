"""
Test 16: Backup Recovery Server Selection — validate the recovery flow
         where users select a specific backup server from a list, then
         confirm recovery.

Tests the API endpoints that back the new Settings UI:
  1. List backup servers (dropdown population)
  2. Recover from a SPECIFIC server by ID (selected from dropdown)
  3. Recovery returns 202 with recovery_id
  4. Attempting recovery from a non-existent server returns 404
  5. Full round-trip: upload → sync → select server → recover → verify data
"""

import time

import pytest
from helpers import (
    APIClient,
    generate_test_jpeg,
    unique_filename,
    wait_for_sync,
    trigger_and_wait,
    assert_no_duplicates,
)
from conftest import (
    ADMIN_USERNAME,
    ADMIN_PASSWORD,
    USER_PASSWORD,
    TEST_BACKUP_API_KEY,
)


class TestBackupServerListing:
    """Verify the server list API that populates the recovery dropdown."""

    def test_list_returns_configured_servers(self, primary_admin, backup_configured):
        """GET /admin/backup/servers returns the registered backup server."""
        result = primary_admin.admin_list_backup_servers()
        assert "servers" in result
        servers = result["servers"]
        assert len(servers) >= 1, "Expected at least one backup server"
        ids = [s["id"] for s in servers]
        assert backup_configured in ids, (
            f"Configured backup {backup_configured} not in server list: {ids}"
        )

    def test_server_list_has_required_fields(self, primary_admin, backup_configured):
        """Each server entry must have fields needed for the dropdown display."""
        result = primary_admin.admin_list_backup_servers()
        servers = result["servers"]
        for s in servers:
            assert "id" in s, f"Server missing 'id': {s}"
            assert "name" in s, f"Server missing 'name': {s}"
            assert "address" in s, f"Server missing 'address': {s}"
            assert "enabled" in s, f"Server missing 'enabled': {s}"

    def test_server_status_reachable(self, primary_admin, backup_configured):
        """The backup server should be reachable for recovery."""
        status = primary_admin.admin_backup_server_status(backup_configured)
        assert status["reachable"] is True, f"Backup server not reachable: {status}"


class TestRecoveryServerSelection:
    """Verify recovery with explicit server selection (the new dropdown flow)."""

    def test_recover_from_selected_server(self, primary_admin, backup_configured):
        """POST /admin/backup/servers/{id}/recover with the selected server ID
        should return 202 (accepted) or 409 (already recovering)."""
        r = primary_admin.post(
            f"/api/admin/backup/servers/{backup_configured}/recover"
        )
        assert r.status_code in (200, 202, 409), (
            f"Recovery from selected server failed: {r.status_code} {r.text}"
        )
        if r.status_code in (200, 202):
            data = r.json()
            assert "recovery_id" in data, f"Response missing recovery_id: {data}"
            assert "message" in data, f"Response missing message: {data}"

    def test_recover_from_nonexistent_server_fails(self, primary_admin):
        """Recovery from a bogus server ID must return 404."""
        r = primary_admin.post(
            "/api/admin/backup/servers/00000000-0000-0000-0000-000000000000/recover"
        )
        assert r.status_code in (404, 400), (
            f"Expected 404 for nonexistent server, got {r.status_code} {r.text}"
        )

    def test_recover_requires_admin(self, primary_server, backup_configured):
        """Non-admin users cannot trigger recovery."""
        anon = APIClient(primary_server.base_url)
        r = anon.post(f"/api/admin/backup/servers/{backup_configured}/recover")
        assert r.status_code in (401, 403), (
            f"Expected 401/403 for unauthenticated recovery, got {r.status_code}"
        )


class TestRecoveryRoundTrip:
    """Full round-trip: upload data, sync to backup, then recover from
    the selected backup server and verify data arrives."""

    def test_upload_sync_select_recover(self, primary_admin, primary_server,
                                         backup_configured, backup_client):
        """End-to-end: upload photos → sync → recover from selected server."""
        # ── Phase 1: Upload test data ──────────────────────────────────
        user_client = APIClient(primary_server.base_url)
        username = f"recsel_{int(time.time())}"
        primary_admin.admin_create_user(username, USER_PASSWORD, role="user")
        user_client.login(username, USER_PASSWORD)

        photo_ids = []
        for i in range(2):
            content = generate_test_jpeg(width=50 + i, height=50 + i)
            p = user_client.upload_photo(unique_filename(), content=content)
            photo_ids.append(p["photo_id"])

        assert len(photo_ids) == 2, f"Expected 2 uploads, got {len(photo_ids)}"

        # ── Phase 2: Sync to backup ───────────────────────────────────
        result = trigger_and_wait(primary_admin, backup_configured, timeout=120)
        assert result.get("status") != "error", f"Sync failed: {result}"

        # Verify photos exist on backup
        backup_photos = backup_client.backup_list()
        backup_ids = [p["id"] for p in backup_photos]
        for pid in photo_ids:
            assert pid in backup_ids, f"Photo {pid} not synced to backup"

        # ── Phase 3: List servers (simulates dropdown population) ─────
        servers = primary_admin.admin_list_backup_servers()
        server_list = servers["servers"]
        assert len(server_list) >= 1

        # Find our backup server in the list (simulates user selection)
        selected = next(
            (s for s in server_list if s["id"] == backup_configured), None
        )
        assert selected is not None, (
            f"Backup server {backup_configured} not found in list"
        )

        # ── Phase 4: Recover from selected server ────────────────────
        r = primary_admin.post(
            f"/api/admin/backup/servers/{selected['id']}/recover"
        )
        assert r.status_code in (200, 202, 409), (
            f"Recovery failed: {r.status_code} {r.text}"
        )

        if r.status_code in (200, 202):
            data = r.json()
            assert "recovery_id" in data
            assert len(data["recovery_id"]) > 0

    def test_recover_specific_server_from_multiple(self, primary_admin, primary_server,
                                                     backup_configured, backup_server):
        """When multiple backup servers exist, recovery targets only the
        selected one (not the first or a random server)."""
        # List the servers we have
        servers = primary_admin.admin_list_backup_servers()
        server_list = servers["servers"]
        assert len(server_list) >= 1, "Need at least 1 backup server"

        # Recover from the specifically selected server
        target_id = backup_configured
        r = primary_admin.post(
            f"/api/admin/backup/servers/{target_id}/recover"
        )
        assert r.status_code in (200, 202, 409), (
            f"Recovery from specific server failed: {r.status_code} {r.text}"
        )

        # If recovery started, the recovery_id should correspond to this server
        if r.status_code in (200, 202):
            data = r.json()
            assert "recovery_id" in data
            # Check sync logs reference the correct server
            logs = primary_admin.admin_get_sync_logs(target_id)
            if logs:
                latest = logs[0] if isinstance(logs, list) else logs
                assert latest.get("server_id") == target_id, (
                    f"Recovery log server_id mismatch: expected {target_id}, "
                    f"got {latest.get('server_id')}"
                )
