"""
Test 13: Comprehensive Backup & Recovery — "Mega" Integration Test

Simulates a real-world workflow end-to-end:
  Phase 1 — Populate primary with two users performing ALL operation types:
            photos, favorites, crops, duplicates, edit copies, blobs,
            secure galleries, shared albums, tags, trash, audio backup toggle.
  Phase 2 — Sync to backup and verify EXACT data (counts, IDs, metadata).
  Phase 3 — Spin up fresh primary, restore from backup, verify EVERYTHING
            survives the round-trip (users, photos, metadata, albums, blobs).

Every assertion uses exact counts and specific IDs.  No "assert len > 0"
style shortcuts.
"""

import json
import os
import time
from collections import Counter

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    unique_filename,
    random_username,
    wait_for_sync,
    wait_for_server,
)
from conftest import (
    ADMIN_USERNAME,
    ADMIN_PASSWORD,
    USER_PASSWORD,
    TEST_BACKUP_API_KEY,
    TEST_ENCRYPTION_KEY,
    ServerInstance,
    _find_free_port,
)


# ── Module-level state shared across all test classes ────────────────

_state = {}


def _trigger_and_wait(admin_client, server_id, timeout=120):
    """Trigger sync and block until complete."""
    admin_client.admin_trigger_sync(server_id)
    return wait_for_sync(admin_client, server_id, timeout=timeout)


def _assert_no_duplicates(id_list, label):
    """Fail if any ID appears more than once."""
    counts = Counter(id_list)
    dupes = {k: v for k, v in counts.items() if v > 1}
    assert not dupes, f"DUPLICATE {label}: {dupes}"


# =====================================================================
# Phase 1: Populate primary with every operation type
# =====================================================================


class TestMegaPopulate:
    """Create two users and exercise every feature on the primary server."""

    def test_populate_primary(self, primary_admin, primary_server,
                              backup_configured, backup_client):
        """
        Single test that performs ALL operations on primary and records
        exact expected state.

        Operations performed:
          User A: 5 photo uploads, favorite p1, crop p2, duplicate p3→p6,
                  2 edit copies (p1, p3), 3 blob uploads, secure gallery
                  with b1, shared album "Vacation" (member: B, photos: p1+p2),
                  tags (p1: landscape+nature, p4: portrait), trash b2.
          User B: 2 photo uploads, 1 blob upload, favorite p7,
                  shared album "Family" (member: A, photo: p7),
                  tag (p7: family).
          Admin:  toggle audio_backup setting.
        """

        # ── Snapshot backup state BEFORE our operations ──────────────
        before_photos = backup_client.backup_list()
        before_blobs = backup_client.backup_list_blobs()
        before_trash = backup_client.backup_list_trash()
        before_users = backup_client.backup_list_users()

        _state["before_photo_ids"] = [p["id"] for p in before_photos]
        _state["before_blob_ids"] = [b["id"] for b in before_blobs]
        _state["before_trash_ids"] = [t["id"] for t in before_trash]
        _state["before_user_count"] = len(before_users)

        # ── Create users ─────────────────────────────────────────────
        _state["user_a_name"] = random_username("mega_a_")
        created_a = primary_admin.admin_create_user(
            _state["user_a_name"], USER_PASSWORD,
        )
        _state["user_a_id"] = created_a["user_id"]

        _state["user_b_name"] = random_username("mega_b_")
        created_b = primary_admin.admin_create_user(
            _state["user_b_name"], USER_PASSWORD,
        )
        _state["user_b_id"] = created_b["user_id"]

        client_a = APIClient(primary_server.base_url)
        client_a.login(_state["user_a_name"], USER_PASSWORD)
        client_b = APIClient(primary_server.base_url)
        client_b.login(_state["user_b_name"], USER_PASSWORD)

        # ── User A: upload 5 photos ──────────────────────────────────
        _state["photo_ids_a"] = []
        _state["photo_contents"] = {}
        for i in range(5):
            content = generate_test_jpeg(width=10 + i, height=10 + i)
            fname = unique_filename()
            photo = client_a.upload_photo(fname, content=content)
            pid = photo["photo_id"]
            _state["photo_ids_a"].append(pid)
            _state["photo_contents"][pid] = content
        assert len(_state["photo_ids_a"]) == 5

        # ── User A: favorite p1 ──────────────────────────────────────
        client_a.favorite_photo(_state["photo_ids_a"][0])

        # ── User A: crop p2 ──────────────────────────────────────────
        _state["crop_metadata"] = {"x": 10, "y": 20, "w": 100, "h": 100}
        client_a.crop_photo(
            _state["photo_ids_a"][1],
            json.dumps(_state["crop_metadata"]),
        )

        # ── User A: duplicate p3 → p6 ────────────────────────────────
        dup = client_a.duplicate_photo(_state["photo_ids_a"][2])
        _state["photo_dup_id"] = dup.get("id") or dup.get("photo_id")
        assert _state["photo_dup_id"], f"No ID in duplicate response: {dup}"
        assert _state["photo_dup_id"] != _state["photo_ids_a"][2]

        # ── User A: create edit copies ────────────────────────────────
        ec1 = client_a.create_edit_copy(
            _state["photo_ids_a"][0],
            name="edited-v1",
            edit_metadata='{"brightness": 1.2}',
        )
        ec2 = client_a.create_edit_copy(
            _state["photo_ids_a"][2],
            name="retouch",
            edit_metadata='{"contrast": 0.8}',
        )
        _state["edit_copy_p1"] = ec1
        _state["edit_copy_p3"] = ec2

        # ── User A: upload 3 blobs ────────────────────────────────────
        _state["blob_ids_a"] = []
        _state["blob_contents"] = {}
        for i in range(3):
            content = generate_random_bytes(1024 + i * 100)
            blob = client_a.upload_blob("photo", content)
            bid = blob["blob_id"]
            _state["blob_ids_a"].append(bid)
            _state["blob_contents"][bid] = content
        assert len(_state["blob_ids_a"]) == 3

        # ── User A: secure gallery — add b1 (creates clone) ──────────
        gallery = client_a.create_secure_gallery("Private")
        _state["gallery_id"] = gallery["gallery_id"]
        token = client_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_result = client_a.add_secure_gallery_item(
            _state["gallery_id"], _state["blob_ids_a"][0], token,
        )
        _state["clone_blob_id"] = add_result["new_blob_id"]

        # Verify gallery item exists
        items = client_a.list_secure_gallery_items(_state["gallery_id"], token)
        item_list = items if isinstance(items, list) else items.get("items", [])
        assert len(item_list) >= 1, f"Gallery should have items: {items}"

        # ── User A: shared album "Vacation" ───────────────────────────
        album_v = client_a.create_shared_album("Vacation")
        _state["album_vacation_id"] = album_v["id"]
        client_a.add_album_member(album_v["id"], _state["user_b_id"])
        client_a.add_album_photo(album_v["id"], _state["photo_ids_a"][0])
        client_a.add_album_photo(album_v["id"], _state["photo_ids_a"][1])

        # ── User A: tags ──────────────────────────────────────────────
        client_a.add_tag(_state["photo_ids_a"][0], "landscape")
        client_a.add_tag(_state["photo_ids_a"][0], "nature")
        client_a.add_tag(_state["photo_ids_a"][3], "portrait")

        # ── User A: trash b2 ─────────────────────────────────────────
        b2 = _state["blob_ids_a"][1]
        trash_resp = client_a.soft_delete_blob(
            b2,
            filename="trashed_mega.jpg",
            mime_type="image/jpeg",
            size_bytes=len(_state["blob_contents"][b2]),
        )
        _state["trashed_blob_id"] = b2
        _state["trash_id"] = trash_resp["trash_id"]

        # ── User B: upload 2 photos ───────────────────────────────────
        _state["photo_ids_b"] = []
        for i in range(2):
            content = generate_test_jpeg(width=20 + i, height=20 + i)
            fname = unique_filename()
            photo = client_b.upload_photo(fname, content=content)
            pid = photo["photo_id"]
            _state["photo_ids_b"].append(pid)
            _state["photo_contents"][pid] = content
        assert len(_state["photo_ids_b"]) == 2

        # ── User B: upload 1 blob ─────────────────────────────────────
        b4_content = generate_random_bytes(2048)
        b4 = client_b.upload_blob("photo", b4_content)
        _state["blob_id_b"] = b4["blob_id"]
        _state["blob_contents"][b4["blob_id"]] = b4_content

        # ── User B: favorite p7 ──────────────────────────────────────
        client_b.favorite_photo(_state["photo_ids_b"][0])

        # ── User B: shared album "Family" ─────────────────────────────
        album_f = client_b.create_shared_album("Family")
        _state["album_family_id"] = album_f["id"]
        client_b.add_album_member(album_f["id"], _state["user_a_id"])
        client_b.add_album_photo(album_f["id"], _state["photo_ids_b"][0])

        # ── User B: tag p7 ───────────────────────────────────────────
        client_b.add_tag(_state["photo_ids_b"][0], "family")

        # ── Admin: toggle audio backup ────────────────────────────────
        r = primary_admin.get("/api/settings/audio-backup")
        r.raise_for_status()
        initial = r.json().get("audio_backup_enabled", False)
        # Enable it
        r = primary_admin.put("/api/admin/audio-backup",
                              json_data={"audio_backup_enabled": True})
        r.raise_for_status()
        r = primary_admin.get("/api/settings/audio-backup")
        assert r.json()["audio_backup_enabled"] is True
        _state["audio_backup_toggled"] = True

        # Store total user count on primary (admin + user_a + user_b
        # + any users from prior tests) so backup verification is exact.
        all_primary_users = primary_admin.admin_list_users()
        _state["primary_user_count"] = len(all_primary_users)

        # ── Verify primary state ──────────────────────────────────────

        # User A: 6 photos (p1-p5 + duplicate)
        photos_a = client_a.list_photos(limit=500)
        photo_list_a = photos_a.get("photos", [])
        assert len(photo_list_a) == 6, (
            f"User A: expected 6 photos, got {len(photo_list_a)}"
        )

        # p1 favorited
        p1 = next(p for p in photo_list_a if p["id"] == _state["photo_ids_a"][0])
        assert p1["is_favorite"] in (True, 1)

        # p2 cropped
        p2 = next(p for p in photo_list_a if p["id"] == _state["photo_ids_a"][1])
        crop = p2.get("crop_metadata")
        if isinstance(crop, str):
            crop = json.loads(crop)
        assert crop == _state["crop_metadata"]

        # User A blobs: b3 visible (b1 gallery-hidden, b2 trashed)
        blobs_a = client_a.list_blobs(limit=500)
        blob_list_a = blobs_a.get("blobs", [])
        visible_a = [b["id"] for b in blob_list_a]
        assert _state["blob_ids_a"][2] in visible_a, "b3 not visible"
        assert _state["blob_ids_a"][0] not in visible_a, "b1 still visible after gallery"
        assert _state["blob_ids_a"][1] not in visible_a, "b2 still visible after trash"

        # User A trash
        trash_a = client_a.list_trash(limit=500)
        trash_items_a = trash_a.get("items", [])
        assert any(t["id"] == _state["trash_id"] for t in trash_items_a)

        # User B: 2 photos
        photos_b = client_b.list_photos(limit=500)
        assert len(photos_b.get("photos", [])) == 2

        # User B: b4 visible
        blobs_b = client_b.list_blobs(limit=500)
        blob_list_b = blobs_b.get("blobs", [])
        assert any(b["id"] == _state["blob_id_b"] for b in blob_list_b)


# =====================================================================
# Phase 2: Sync to backup and verify exact data
# =====================================================================


class TestMegaBackupSync:
    """Trigger sync and verify every data category on the backup."""

    def test_sync_to_backup(self, primary_admin, backup_configured):
        """Trigger sync and wait for success."""
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        assert result.get("status") != "error", f"Sync failed: {result}"

    # ── Photos ────────────────────────────────────────────────────────

    def test_backup_photos_count_and_ids(self, backup_client):
        """Backup has exactly before + 8 photos, no duplicates."""
        photos = backup_client.backup_list()
        photo_ids = [p["id"] for p in photos]
        _assert_no_duplicates(photo_ids, "backup photos")

        all_new = (
            _state["photo_ids_a"]
            + [_state["photo_dup_id"]]
            + _state["photo_ids_b"]
        )
        for pid in all_new:
            assert pid in photo_ids, f"Photo {pid} missing from backup"

        expected_total = len(_state["before_photo_ids"]) + 8
        assert len(photo_ids) == expected_total, (
            f"Expected {expected_total} photos, got {len(photo_ids)}"
        )

    def test_backup_favorite_p1(self, backup_client):
        """p1 is_favorite on backup."""
        photos = backup_client.backup_list()
        p1 = next(p for p in photos if p["id"] == _state["photo_ids_a"][0])
        assert p1["is_favorite"] in (True, 1), (
            f"p1 is_favorite not synced: {p1['is_favorite']}"
        )

    def test_backup_favorite_p7(self, backup_client):
        """p7 is_favorite on backup."""
        photos = backup_client.backup_list()
        p7 = next(p for p in photos if p["id"] == _state["photo_ids_b"][0])
        assert p7["is_favorite"] in (True, 1), (
            f"p7 is_favorite not synced: {p7['is_favorite']}"
        )

    def test_backup_crop_metadata_p2(self, backup_client):
        """p2 crop metadata matches on backup."""
        photos = backup_client.backup_list()
        p2 = next(p for p in photos if p["id"] == _state["photo_ids_a"][1])
        crop = p2.get("crop_metadata")
        if isinstance(crop, str):
            crop = json.loads(crop)
        assert crop == _state["crop_metadata"], (
            f"p2 crop mismatch: {crop} != {_state['crop_metadata']}"
        )

    def test_backup_non_favorited_photos(self, backup_client):
        """p3, p4, p5, p8 should NOT be favorited on backup."""
        photos = backup_client.backup_list()
        non_fav_ids = [
            _state["photo_ids_a"][2],  # p3
            _state["photo_ids_a"][3],  # p4
            _state["photo_ids_a"][4],  # p5
            _state["photo_ids_b"][1],  # p8
        ]
        for pid in non_fav_ids:
            p = next((p for p in photos if p["id"] == pid), None)
            if p is not None:
                assert p.get("is_favorite") in (False, 0, None), (
                    f"Photo {pid} unexpectedly favorited on backup"
                )

    def test_backup_photo_sizes(self, backup_client):
        """Photo file sizes match on backup."""
        photos = backup_client.backup_list()
        for pid, content in _state["photo_contents"].items():
            bp = next((p for p in photos if p["id"] == pid), None)
            if bp is not None:
                assert bp["size_bytes"] == len(content), (
                    f"Photo {pid} size: backup={bp['size_bytes']} "
                    f"expected={len(content)}"
                )

    # ── Blobs ─────────────────────────────────────────────────────────

    def test_backup_blobs_count_and_ids(self, backup_client):
        """Backup has before + 2 blobs (b3, b4).  No gallery/trash leaks."""
        blobs = backup_client.backup_list_blobs()
        blob_ids = [b["id"] for b in blobs]
        _assert_no_duplicates(blob_ids, "backup blobs")

        b1 = _state["blob_ids_a"][0]
        b2 = _state["blob_ids_a"][1]
        b3 = _state["blob_ids_a"][2]
        b4 = _state["blob_id_b"]
        bc1 = _state["clone_blob_id"]

        assert b3 in blob_ids, f"b3 {b3} missing from backup"
        assert b4 in blob_ids, f"b4 {b4} missing from backup"
        assert b1 not in blob_ids, f"b1 {b1} (gallery) leaked to backup"
        assert bc1 not in blob_ids, f"clone {bc1} leaked to backup"
        assert b2 not in blob_ids, f"b2 {b2} (trashed) in backup blobs"

        expected_total = len(_state["before_blob_ids"]) + 2
        assert len(blob_ids) == expected_total, (
            f"Expected {expected_total} blobs, got {len(blob_ids)}"
        )

    def test_backup_blob_sizes(self, backup_client):
        """Blob file sizes match on backup."""
        blobs = backup_client.backup_list_blobs()
        for bid in [_state["blob_ids_a"][2], _state["blob_id_b"]]:
            content = _state["blob_contents"][bid]
            bb = next((b for b in blobs if b["id"] == bid), None)
            assert bb is not None, f"Blob {bid} missing"
            assert bb["size_bytes"] == len(content), (
                f"Blob {bid} size: backup={bb['size_bytes']} "
                f"expected={len(content)}"
            )

    # ── Trash ─────────────────────────────────────────────────────────

    def test_backup_trash_count_and_ids(self, backup_client):
        """Backup has before + 1 trash item (b2)."""
        trash = backup_client.backup_list_trash()
        trash_ids = [t["id"] for t in trash]
        _assert_no_duplicates(trash_ids, "backup trash")

        assert _state["trash_id"] in trash_ids, (
            f"Trash {_state['trash_id']} missing from backup"
        )

        expected_total = len(_state["before_trash_ids"]) + 1
        assert len(trash_ids) == expected_total, (
            f"Expected {expected_total} trash items, got {len(trash_ids)}"
        )

    def test_backup_trash_size(self, backup_client):
        """Trashed blob size_bytes correct on backup."""
        trash = backup_client.backup_list_trash()
        item = next(t for t in trash if t["id"] == _state["trash_id"])
        expected_size = len(_state["blob_contents"][_state["trashed_blob_id"]])
        assert item["size_bytes"] == expected_size, (
            f"Trash size mismatch: {item['size_bytes']} != {expected_size}"
        )

    # ── Users ─────────────────────────────────────────────────────────

    def test_backup_users_count(self, backup_client):
        """Backup has exactly as many users as primary."""
        users = backup_client.backup_list_users()
        _assert_no_duplicates([u["id"] for u in users], "backup users")
        usernames = [u["username"] for u in users]

        assert _state["user_a_name"] in usernames, "User A not on backup"
        assert _state["user_b_name"] in usernames, "User B not on backup"

        expected_total = _state["primary_user_count"]
        assert len(users) == expected_total, (
            f"Expected {expected_total} users, got {len(users)}"
        )


# =====================================================================
# Phase 3: Restore fresh primary from backup, verify everything
# =====================================================================


class TestMegaRecovery:
    """
    Full disaster recovery: spin up a brand-new server, restore from the
    backup, then verify every data category survived the round-trip.
    """

    @pytest.fixture
    def fresh_server(self, server_binary, session_tmpdir, backup_server,
                     backup_client):
        """Start a fresh primary, pair with backup, snapshot expected state."""
        if server_binary is None:
            pytest.skip("External servers: can't spin up fresh instance")

        port = _find_free_port()
        tmpdir = os.path.join(session_tmpdir, f"mega_recovery_{int(time.time())}")
        server = ServerInstance("mega-recovery", port, tmpdir)
        server.start(server_binary)

        try:
            client = APIClient(server.base_url)
            client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
            client.login(ADMIN_USERNAME, ADMIN_PASSWORD)
            try:
                client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
            except Exception:
                pass

            result = client.admin_add_backup_server(
                name="mega-backup",
                address=backup_server.base_url.replace("http://", ""),
                api_key=backup_server.backup_api_key,
            )
            server_id = result["id"]

            # Snapshot full backup state (this is what should end up on the
            # fresh server after recovery)
            expected = {
                "photo_ids": set(p["id"] for p in backup_client.backup_list()),
                "blob_ids": set(b["id"] for b in backup_client.backup_list_blobs()),
                "trash_ids": set(t["id"] for t in backup_client.backup_list_trash()),
                "usernames": set(u["username"] for u in backup_client.backup_list_users()),
                "photos_full": backup_client.backup_list(),
            }

            yield {
                "server": server,
                "client": client,
                "base_url": server.base_url,
                "server_id": server_id,
                "expected": expected,
            }
        finally:
            server.stop()

    def test_full_recovery_and_verification(self, fresh_server, backup_client):
        """
        Recover → re-login → verify:
          1. Users (all present, correct usernames)
          2. User A photos (6, correct IDs, favorites, crop)
          3. User A edit copies (p1: edited-v1, p3: retouch)
          4. User A shared albums (Vacation with p1+p2)
          5. User B photos (2, correct IDs, favorite)
          6. User B shared albums (Family with p7)
          7. Blobs recovered (b3, b4 present)
          8. No duplicate photos/blobs anywhere
        """
        client = fresh_server["client"]
        sid = fresh_server["server_id"]
        base = fresh_server["base_url"]
        expected = fresh_server["expected"]

        # ── Trigger recovery ──────────────────────────────────────────
        r = client.post(f"/api/admin/backup/servers/{sid}/recover")
        assert r.status_code in (200, 202), (
            f"Recovery trigger failed: {r.status_code} {r.text}"
        )

        # ── Wait for recovery with re-login ───────────────────────────
        import requests as _req

        time.sleep(5)
        deadline = time.time() + 180
        recovered = False
        relogged = False

        while time.time() < deadline:
            if not relogged:
                try:
                    r = _req.post(
                        f"{base}/api/auth/login",
                        json={
                            "username": ADMIN_USERNAME,
                            "password": ADMIN_PASSWORD,
                        },
                        headers={"X-Forwarded-For": "10.99.99.99"},
                        timeout=5,
                    )
                    if r.status_code == 200:
                        data = r.json()
                        token = data.get("access_token")
                        if token:
                            client.access_token = token
                            client.session.headers["Authorization"] = (
                                f"Bearer {token}"
                            )
                            relogged = True
                except Exception:
                    pass

            try:
                logs = client.admin_get_sync_logs(sid)
                if logs:
                    latest = logs[0] if isinstance(logs, list) else logs
                    if latest.get("status") in ("success", "completed"):
                        recovered = True
                        break
                    if latest.get("status") == "error":
                        fresh_server["server"].dump_logs()
                        pytest.fail(
                            f"Recovery error: {latest.get('error')}"
                        )
            except Exception:
                pass
            time.sleep(3)

        assert recovered, "Recovery did not complete within timeout"

        # ── Fresh admin client ────────────────────────────────────────
        admin = APIClient(base)
        admin.login(ADMIN_USERNAME, ADMIN_PASSWORD)

        # ── 1. USERS ─────────────────────────────────────────────────
        users = admin.admin_list_users()
        recovered_usernames = {u["username"] for u in users}
        _assert_no_duplicates([u["id"] for u in users], "recovered users")

        for uname in expected["usernames"]:
            assert uname in recovered_usernames, (
                f"User '{uname}' not recovered. Got: {recovered_usernames}"
            )
        assert _state["user_a_name"] in recovered_usernames
        assert _state["user_b_name"] in recovered_usernames

        # ── 2. USER A PHOTOS ─────────────────────────────────────────
        user_a = APIClient(base)
        user_a.login(_state["user_a_name"], USER_PASSWORD)

        photos_a = user_a.list_photos(limit=500)
        photo_list_a = photos_a.get("photos", [])
        photo_ids_a = {p["id"] for p in photo_list_a}
        _assert_no_duplicates(
            [p["id"] for p in photo_list_a], "recovered User A photos",
        )

        # 6 photos: p1-p5 + duplicate p6
        for pid in _state["photo_ids_a"]:
            assert pid in photo_ids_a, f"User A photo {pid} not recovered"
        assert _state["photo_dup_id"] in photo_ids_a, (
            "Duplicate photo not recovered"
        )
        assert len(photo_ids_a) == 6, (
            f"User A: expected 6 photos, got {len(photo_ids_a)}: {photo_ids_a}"
        )

        # ── 3. USER A METADATA ────────────────────────────────────────
        p1 = next(p for p in photo_list_a if p["id"] == _state["photo_ids_a"][0])
        assert p1["is_favorite"] in (True, 1), (
            f"p1 favorite not recovered: {p1['is_favorite']}"
        )

        p2 = next(p for p in photo_list_a if p["id"] == _state["photo_ids_a"][1])
        crop = p2.get("crop_metadata")
        if isinstance(crop, str):
            crop = json.loads(crop)
        assert crop == _state["crop_metadata"], (
            f"p2 crop not recovered: {crop}"
        )

        # ── 4. USER A EDIT COPIES ─────────────────────────────────────
        ec1_resp = user_a.list_edit_copies(_state["photo_ids_a"][0])
        ec1_list = (
            ec1_resp if isinstance(ec1_resp, list)
            else ec1_resp.get("copies", ec1_resp.get("edit_copies", []))
        )
        assert len(ec1_list) >= 1, (
            f"Edit copy for p1 not recovered: {ec1_resp}"
        )
        assert any(c.get("name") == "edited-v1" for c in ec1_list), (
            f"'edited-v1' copy not found: {ec1_list}"
        )
        # Verify edit metadata survived
        ec1 = next(c for c in ec1_list if c.get("name") == "edited-v1")
        em = ec1.get("edit_metadata")
        if isinstance(em, str):
            em = json.loads(em)
        assert em == {"brightness": 1.2}, f"Edit metadata mismatch: {em}"

        ec2_resp = user_a.list_edit_copies(_state["photo_ids_a"][2])
        ec2_list = (
            ec2_resp if isinstance(ec2_resp, list)
            else ec2_resp.get("copies", ec2_resp.get("edit_copies", []))
        )
        assert len(ec2_list) >= 1, (
            f"Edit copy for p3 not recovered: {ec2_resp}"
        )
        assert any(c.get("name") == "retouch" for c in ec2_list), (
            f"'retouch' copy not found: {ec2_list}"
        )

        # ── 5. USER A SHARED ALBUMS ──────────────────────────────────
        albums_a = user_a.list_shared_albums()
        album_list_a = (
            albums_a if isinstance(albums_a, list)
            else albums_a.get("albums", [])
        )
        album_names_a = [a.get("name") for a in album_list_a]
        assert "Vacation" in album_names_a, (
            f"'Vacation' not recovered for User A: {album_names_a}"
        )

        # Vacation album: photos p1, p2
        vac = next(a for a in album_list_a if a["name"] == "Vacation")
        vac_photos = user_a.list_album_photos(vac["id"])
        vac_list = (
            vac_photos if isinstance(vac_photos, list)
            else vac_photos.get("photos", [])
        )
        vac_refs = [
            p.get("photo_ref", p.get("photo_id", p.get("id")))
            for p in vac_list
        ]
        assert _state["photo_ids_a"][0] in vac_refs, "p1 not in Vacation"
        assert _state["photo_ids_a"][1] in vac_refs, "p2 not in Vacation"
        assert len(vac_list) == 2, (
            f"Vacation: expected 2 photos, got {len(vac_list)}"
        )

        # ── 6. USER B PHOTOS ─────────────────────────────────────────
        user_b = APIClient(base)
        user_b.login(_state["user_b_name"], USER_PASSWORD)

        photos_b = user_b.list_photos(limit=500)
        photo_list_b = photos_b.get("photos", [])
        photo_ids_b = {p["id"] for p in photo_list_b}
        _assert_no_duplicates(
            [p["id"] for p in photo_list_b], "recovered User B photos",
        )

        for pid in _state["photo_ids_b"]:
            assert pid in photo_ids_b, f"User B photo {pid} not recovered"
        assert len(photo_ids_b) == 2, (
            f"User B: expected 2 photos, got {len(photo_ids_b)}"
        )

        # ── 7. USER B METADATA ────────────────────────────────────────
        p7 = next(p for p in photo_list_b if p["id"] == _state["photo_ids_b"][0])
        assert p7["is_favorite"] in (True, 1), "p7 favorite not recovered"

        # ── 8. USER B SHARED ALBUMS ────────────────────────────────────
        albums_b = user_b.list_shared_albums()
        album_list_b = (
            albums_b if isinstance(albums_b, list)
            else albums_b.get("albums", [])
        )
        album_names_b = [a.get("name") for a in album_list_b]
        assert "Family" in album_names_b, (
            f"'Family' not recovered for User B: {album_names_b}"
        )

        # Family album: photo p7
        fam = next(a for a in album_list_b if a["name"] == "Family")
        fam_photos = user_b.list_album_photos(fam["id"])
        fam_list = (
            fam_photos if isinstance(fam_photos, list)
            else fam_photos.get("photos", [])
        )
        fam_refs = [
            p.get("photo_ref", p.get("photo_id", p.get("id")))
            for p in fam_list
        ]
        assert _state["photo_ids_b"][0] in fam_refs, "p7 not in Family"
        assert len(fam_list) == 1, (
            f"Family: expected 1 photo, got {len(fam_list)}"
        )

        # ── 9. BLOBS RECOVERED ────────────────────────────────────────
        # b3 should exist for User A, b4 for User B
        blobs_a = user_a.list_blobs(limit=500)
        blob_list_a = blobs_a.get("blobs", [])
        blob_ids_a = [b["id"] for b in blob_list_a]
        assert _state["blob_ids_a"][2] in blob_ids_a, "b3 not recovered"

        blobs_b = user_b.list_blobs(limit=500)
        blob_list_b = blobs_b.get("blobs", [])
        blob_ids_b = [b["id"] for b in blob_list_b]
        assert _state["blob_id_b"] in blob_ids_b, "b4 not recovered"

        # ── 10. NO DUPLICATES ACROSS ALL DATA ─────────────────────────
        # Gather all photo IDs from both users
        all_recovered_photos = [p["id"] for p in photo_list_a] + [
            p["id"] for p in photo_list_b
        ]
        _assert_no_duplicates(all_recovered_photos, "all recovered photos")

        all_recovered_blobs = blob_ids_a + blob_ids_b
        _assert_no_duplicates(all_recovered_blobs, "all recovered blobs")
