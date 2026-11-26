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

from helpers import generate_random_bytes, trigger_and_wait


def _short_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:12]


def _register_like_android(client, filename):
    original = generate_random_bytes(4096)
    photo_hash = _short_hash(original)
    thumb = client.upload_blob("thumbnail", generate_random_bytes(512))["blob_id"]
    media = client.upload_blob("photo", generate_random_bytes(8192),
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
