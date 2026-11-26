"""
Test 100: rows with no plaintext file reach the backup, and sync reaches `success`.

Why this exists (TODO.md, F9 — found 2026-11-25 by the full E2E run)
--------------------------------------------------------------------
`register-encrypted` (every Android upload) and the encrypted Save Copy store a
`photos` row with `file_path = ''`: the bytes live only in the encrypted blob.
Trashing such a photo copies that empty path into `trash_items`. The sync engine's
photo and trash phases sent every row as a FILE (`storage_root.join(file_path)`), so
each of these rows failed every cycle and the run was `partial` forever. The
backup received the blobs (phase 3) but never the rows that index them, so a
disaster recovery would come back with encrypted bytes and no library.

These tests replay the phone's call order (thumb blob, media blob, register),
then sync, and check the backup holds the ROW with its blob ids.
"""

import hashlib
import os
import time

import pytest
import requests

from conftest import (
    ADMIN_PASSWORD,
    ADMIN_USERNAME,
    TEST_ENCRYPTION_KEY,
    USER_PASSWORD,
    ServerInstance,
    _find_free_port,
)
from helpers import APIClient, generate_random_bytes, trigger_and_wait


def _short_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:12]


def _register_like_android(client, filename):
    original = generate_random_bytes(4096)
    photo_hash = _short_hash(original)
    thumb_bytes = generate_random_bytes(512)
    media_bytes = generate_random_bytes(8192)
    thumb = client.upload_blob("thumbnail", thumb_bytes)["blob_id"]
    media = client.upload_blob("photo", media_bytes,
                               content_hash=photo_hash)["blob_id"]
    r = client.post("/api/photos/register-encrypted", json_data={
        "filename": filename,
        "mime_type": "image/jpeg",
        "media_type": "photo",
        "width": 4032,
        "height": 3024,
        "taken_at": "2026-04-18T02:16:41.872Z",
        "encrypted_blob_id": media,
        "encrypted_thumb_blob_id": thumb,
        "photo_hash": photo_hash,
    })
    assert r.status_code == 201, r.text
    return {"photo_id": r.json()["photo_id"], "blob": media, "thumb": thumb,
            "media_bytes": media_bytes, "thumb_bytes": thumb_bytes,
            "photo_hash": photo_hash, "filename": filename}


def _sync_ok(primary_admin, server_id):
    log = trigger_and_wait(primary_admin, server_id)
    assert log["status"] == "success", (
        f"sync ended {log['status']!r}: {log.get('error')}"
    )
    return log


class TestEncryptedOnlyRowsReachBackup:

    def test_registered_photo_row_reaches_backup_with_its_blob_ids(
            self, primary_admin, user_client, backup_configured, backup_client):
        reg = _register_like_android(user_client, "PXL_F9_row.jpg")

        _sync_ok(primary_admin, backup_configured)

        rows = {p["id"]: p for p in backup_client.backup_list()}
        assert reg["photo_id"] in rows, "encrypted-only photo row never reached the backup"
        row = rows[reg["photo_id"]]
        assert row["filename"] == reg["filename"]
        assert row["photo_hash"] == reg["photo_hash"]
        assert row["file_path"] == "", "a row-only transfer must not invent a file path"
        assert row["encrypted_blob_id"] == reg["blob"]
        assert row["encrypted_thumb_blob_id"] == reg["thumb"]
        assert row["taken_at"].startswith("2026-04-18T02:16:41")

    def test_resync_is_idempotent(
            self, primary_admin, user_client, backup_configured, backup_client):
        reg = _register_like_android(user_client, "PXL_F9_twice.jpg")
        _sync_ok(primary_admin, backup_configured)
        _sync_ok(primary_admin, backup_configured)

        ids = [p["id"] for p in backup_client.backup_list()]
        assert ids.count(reg["photo_id"]) == 1

    def test_trashed_registered_photo_reaches_backup_trash(
            self, primary_admin, user_client, backup_configured, backup_client):
        reg = _register_like_android(user_client, "PXL_F9_trash.jpg")
        _sync_ok(primary_admin, backup_configured)
        assert reg["photo_id"] in {p["id"] for p in backup_client.backup_list()}

        r = user_client.delete(f"/api/photos/{reg['photo_id']}")
        assert r.status_code in (200, 204), r.text

        _sync_ok(primary_admin, backup_configured)

        assert reg["photo_id"] not in {p["id"] for p in backup_client.backup_list()}, \
            "trashed photo still in the backup gallery"
        trash = user_client.get("/api/trash").json()
        items = trash.get("items", trash) if isinstance(trash, dict) else trash
        trash_ids = {t["id"] for t in items if t.get("photo_id") == reg["photo_id"]}
        assert len(trash_ids) == 1, f"expected one primary trash row, got {items}"
        assert trash_ids <= {t["id"] for t in backup_client.backup_list_trash()}

    def test_empty_path_rows_never_delete_other_backup_rows(
            self, primary_admin, user_client, backup_configured, backup_client):
        """The trap the design avoided: in `/backup/receive`, an empty `file_path`
        matches every encrypted-only row. Trashing ONE such photo must leave the
        other encrypted-only rows on the backup alone."""
        keep = [_register_like_android(user_client, f"PXL_F9_keep{i}.jpg") for i in range(3)]
        gone = _register_like_android(user_client, "PXL_F9_gone.jpg")
        _sync_ok(primary_admin, backup_configured)

        r = user_client.delete(f"/api/photos/{gone['photo_id']}")
        assert r.status_code in (200, 204), r.text
        _sync_ok(primary_admin, backup_configured)

        ids = {p["id"] for p in backup_client.backup_list()}
        assert gone["photo_id"] not in ids
        for k in keep:
            assert k["photo_id"] in ids, f"{k['photo_id']} was deleted by an unrelated trash row"


def _recover_fresh_primary(server_binary, session_tmpdir, backup_server, label):
    """Start an empty primary, point it at `backup_server`, run a recovery and
    wait for it to finish. Returns the running ServerInstance (caller stops it).

    Recovery replaces the admin's user row with the backup's, which invalidates
    the session it started with, so the admin re-logs in while polling.
    """
    port = _find_free_port()
    tmpdir = os.path.join(session_tmpdir, f"{label}_{int(time.time())}")
    server = ServerInstance(label, port, tmpdir)
    server.start(server_binary)
    try:
        admin = APIClient(server.base_url)
        admin.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        admin.login(ADMIN_USERNAME, ADMIN_PASSWORD)
        admin.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
        sid = admin.admin_add_backup_server(
            name=f"{label}-backup",
            address=backup_server.base_url.replace("http://", ""),
            api_key=backup_server.backup_api_key,
        )["id"]

        r = admin.post(f"/api/admin/backup/servers/{sid}/recover")
        assert r.status_code in (200, 202), f"recover refused: {r.status_code} {r.text}"

        deadline = time.time() + 120
        last = None
        while time.time() < deadline:
            time.sleep(2)
            try:
                logs = admin.admin_get_sync_logs(sid)
            except requests.HTTPError as e:
                if e.response is None or e.response.status_code != 401:
                    raise
                # Recovery swapped the admin's user row: the old session is gone.
                print(f"[{label}] session invalidated by recovery; logging in again")
                admin.login(ADMIN_USERNAME, ADMIN_PASSWORD)
                continue
            last = logs[0] if isinstance(logs, list) and logs else None
            if last and last.get("status") in ("success", "completed"):
                return server
            if last and last.get("status") == "error":
                server.dump_logs()
                pytest.fail(f"recovery failed: {last.get('error')}")
        server.dump_logs()
        pytest.fail(f"recovery did not finish in 120 s; last log: {last}")
    except BaseException:
        server.stop()
        raise


class TestEncryptedOnlyRowsSurviveRecovery:
    """F10 (2026-11-25). The suspicion was "recovery can't restore `file_path = ''`
    (Android) rows". It was filed against `run_recovery`, which turned out to be
    dead code; the LIVE recovery is the backup pushing to the new primary through
    the sync engine, which since F9 sends these rows through receive-photo-rows.
    Nothing tested that end to end: a backup that holds every Android photo is
    worthless if a recovery can't put them back."""

    def test_registered_photo_comes_back_with_its_bytes(
            self, server_binary, session_tmpdir, backup_server, primary_admin,
            user_client, backup_configured):
        if server_binary is None:
            pytest.skip("External servers: can't spin up a fresh instance")

        reg = _register_like_android(user_client, "PXL_F10_recover.jpg")
        _sync_ok(primary_admin, backup_configured)

        server = _recover_fresh_primary(server_binary, session_tmpdir, backup_server,
                                        "recovery-f10")
        try:
            user = APIClient(server.base_url)
            user.login(user_client.username, USER_PASSWORD)

            rows = {p["id"]: p for p in user.encrypted_sync_all()}
            assert reg["photo_id"] in rows, (
                "an Android (register-encrypted) photo did not come back from recovery"
            )
            row = rows[reg["photo_id"]]
            assert row["encrypted_blob_id"] == reg["blob"]
            assert row["encrypted_thumb_blob_id"] == reg["thumb"]

            media = user.download_blob(reg["blob"])
            assert media.status_code == 200, f"media blob missing: {media.status_code}"
            assert media.content == reg["media_bytes"], "recovered media bytes differ"
            thumb = user.download_blob(reg["thumb"])
            assert thumb.status_code == 200, f"thumb blob missing: {thumb.status_code}"
            assert thumb.content == reg["thumb_bytes"], "recovered thumbnail bytes differ"
        finally:
            server.stop()
