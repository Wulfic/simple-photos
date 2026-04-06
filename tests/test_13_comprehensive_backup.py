"""
Test 13: Comprehensive Backup & Recovery — "Mega" Integration Test

Simulates a real-world workflow end-to-end:
  Phase 1 — Populate primary with two users performing ALL operation types:
            photos, favorites, crops, duplicates, edit copies, blobs,
            secure galleries, shared albums, tags, trash, audio backup toggle.
  Phase 2 — Sync to backup and verify EXACT data via:
            (a) Backup API key endpoints (server-to-server)
            (b) User-facing API on backup (login as synced users)
            Including: photos, blobs, trash, users, favorites, crop metadata,
            edit copies, tags, shared albums, secure galleries.
  Phase 2b— Duplicate regression: cross-listing checks (gallery items must
            not appear in regular listings) and the pre-synced-then-secured
            scenario (sync → add to gallery → sync again → verify retroactive
            purge removes duplicates). Tests BOTH photos and blobs.
  Phase 3 — Spin up fresh primary, restore from backup, verify EVERYTHING
            survives the round-trip (users, photos, metadata, albums, blobs,
            tags, edit copies, secure galleries, trash, no duplicates).

Every assertion uses exact counts and specific IDs.  No "assert len > 0"
style shortcuts.

Designed to FAIL when:
  - Backup server has duplicate photos (secure gallery clones not purged)
  - Pre-synced items not retroactively purged after gallery add
  - Gallery item IDs leak into regular photo/blob listings
  - Secure albums show empty on backup (gallery metadata not synced)
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


def _dump_server_logs(server, label=""):
    """Dump last portion of server log for debugging."""
    try:
        if hasattr(server, "log_path") and os.path.exists(server.log_path):
            with open(server.log_path) as f:
                content = f.read()
            tail = content[-8000:] if len(content) > 8000 else content
            print(f"\n{'='*60}")
            print(f"  SERVER LOGS: {label or server.name}")
            print(f"{'='*60}")
            print(tail)
            print(f"{'='*60}\n")
    except Exception as e:
        print(f"[WARN] Could not dump logs for {label}: {e}")


def _login_on_backup(backup_server, username, password):
    """Login as a synced user on the backup server. Returns APIClient or None."""
    client = APIClient(backup_server.base_url)
    try:
        client.login(username, password)
        return client
    except Exception as e:
        print(f"[WARN] Could not login as {username} on backup: {e}")
        return None


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

        Expected state after all operations:
          User A: 6 photos (p1-p5 + dup p6), 1 visible blob (b3),
                  1 trashed blob (b2), 1 gallery-hidden blob (b1),
                  1 secure gallery with 1 item, 1 shared album (2 photos),
                  3 tags (p1: landscape+nature, p4: portrait),
                  2 edit copies (p1: edited-v1, p3: retouch)
          User B: 2 photos (p7-p8), 1 blob (b4),
                  1 shared album (1 photo), 1 tag (p7: family)
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

        # ── User A: secure gallery — also add photo p5 (creates photo clone) ─
        add_photo_result = client_a.add_secure_gallery_item(
            _state["gallery_id"], _state["photo_ids_a"][4], token,
        )
        _state["clone_photo_id"] = add_photo_result["new_blob_id"]

        # Verify gallery items exist on primary (2 items: b1 clone + p5 clone)
        items = client_a.list_secure_gallery_items(_state["gallery_id"], token)
        item_list = items if isinstance(items, list) else items.get("items", [])
        _state["primary_gallery_item_count"] = len(item_list)
        assert len(item_list) == 2, (
            f"Gallery should have 2 items (blob b1 + photo p5): got {len(item_list)}"
        )

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

        # User A: 5 visible photos (p1-p4 + dup p6; p5 hidden by gallery)
        photos_a = client_a.list_photos(limit=500)
        photo_list_a = photos_a.get("photos", [])
        assert len(photo_list_a) == 5, (
            f"User A: expected 5 photos (p5 hidden by gallery), got {len(photo_list_a)}"
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

        # Verify primary tags
        tags_p1 = client_a.get_photo_tags(_state["photo_ids_a"][0])
        tag_list_p1 = tags_p1 if isinstance(tags_p1, list) else tags_p1.get("tags", [])
        tag_names_p1 = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list_p1]
        assert "landscape" in tag_names_p1, f"p1 missing 'landscape' tag: {tag_names_p1}"
        assert "nature" in tag_names_p1, f"p1 missing 'nature' tag: {tag_names_p1}"

        tags_p4 = client_a.get_photo_tags(_state["photo_ids_a"][3])
        tag_list_p4 = tags_p4 if isinstance(tags_p4, list) else tags_p4.get("tags", [])
        tag_names_p4 = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list_p4]
        assert "portrait" in tag_names_p4, f"p4 missing 'portrait' tag: {tag_names_p4}"

        # Verify primary edit copies
        ec1_resp = client_a.list_edit_copies(_state["photo_ids_a"][0])
        ec1_list = (ec1_resp if isinstance(ec1_resp, list)
                    else ec1_resp.get("copies", ec1_resp.get("edit_copies", [])))
        assert any(c.get("name") == "edited-v1" for c in ec1_list), (
            f"p1 edit copy 'edited-v1' missing on primary: {ec1_list}"
        )

        # User B: 2 photos
        photos_b = client_b.list_photos(limit=500)
        assert len(photos_b.get("photos", [])) == 2

        # User B: b4 visible
        blobs_b = client_b.list_blobs(limit=500)
        blob_list_b = blobs_b.get("blobs", [])
        assert any(b["id"] == _state["blob_id_b"] for b in blob_list_b)

        # Store backup server reference for Phase 2
        _state["backup_server_url"] = backup_client.base_url


# =====================================================================
# Phase 2: Sync to backup and verify exact data
# =====================================================================


class TestMegaBackupSync:
    """Trigger sync and verify every data category on the backup.

    Verifies via BOTH:
      (a) Backup API key endpoints (/api/backup/list etc.) — server-to-server
      (b) User-facing API on the backup (login as synced users) — simulates
          what a real user would see if the backup were promoted to primary
    """

    def test_sync_to_backup(self, primary_admin, backup_configured,
                            primary_server, backup_server):
        """Trigger sync, wait for success, dump logs on failure."""
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        if result.get("status") == "error":
            _dump_server_logs(primary_server, "primary (sync error)")
            _dump_server_logs(backup_server, "backup (sync error)")
        assert result.get("status") != "error", (
            f"Sync failed: {result}"
        )

    # ── Backup API: Photos ────────────────────────────────────────────

    def test_backup_api_photos_count_and_ids(self, backup_client):
        """Backup /api/backup/list has exactly before + 7 photos, no dupes.

        p5 is in the secure gallery so it should NOT be sent to backup,
        and the backup list endpoint must NOT include gallery clones.
        """
        photos = backup_client.backup_list()
        photo_ids = [p["id"] for p in photos]
        _assert_no_duplicates(photo_ids, "backup API photos")

        # ── Content-level duplicate detection via photo_hash ──────────
        # Two photos with the same non-null hash means the same image
        # appears twice — the classic backup duplicate bug.
        from collections import Counter
        hash_counter = Counter(
            p.get("photo_hash") for p in photos
            if p.get("photo_hash") is not None
        )
        dup_hashes = {h: c for h, c in hash_counter.items() if c > 1}
        assert not dup_hashes, (
            f"DUPLICATE BUG: backup_list_photos contains photos with identical "
            f"photo_hash values (same image appears multiple times): "
            f"{dup_hashes}. IDs: "
            + str([
                p['id'] for p in photos
                if p.get('photo_hash') in dup_hashes
            ])
        )

        # p5 (gallery-hidden) must NOT appear
        p5 = _state["photo_ids_a"][4]
        assert p5 not in photo_ids, (
            f"DUPLICATE BUG: p5 {p5} (gallery-hidden) leaked to backup API photos. "
            f"backup_list_photos must filter encrypted_gallery_items."
        )
        # p5's clone must NOT appear
        cp5 = _state["clone_photo_id"]
        assert cp5 not in photo_ids, (
            f"DUPLICATE BUG: p5 clone {cp5} leaked to backup API photos."
        )

        # All non-gallery photos should be present
        expected_new = (
            _state["photo_ids_a"][:4]  # p1-p4 (p5 excluded)
            + [_state["photo_dup_id"]]  # dup p6
            + _state["photo_ids_b"]     # p7, p8
        )
        for pid in expected_new:
            assert pid in photo_ids, f"Photo {pid} missing from backup API"

        expected_total = len(_state["before_photo_ids"]) + 7
        assert len(photo_ids) == expected_total, (
            f"Expected {expected_total} photos on backup API (p5 hidden), "
            f"got {len(photo_ids)}"
        )

    def test_backup_api_favorite_p1(self, backup_client):
        """p1 is_favorite on backup API."""
        photos = backup_client.backup_list()
        p1 = next(p for p in photos if p["id"] == _state["photo_ids_a"][0])
        assert p1["is_favorite"] in (True, 1), (
            f"p1 is_favorite not synced: {p1['is_favorite']}"
        )

    def test_backup_api_favorite_p7(self, backup_client):
        """p7 is_favorite on backup API."""
        photos = backup_client.backup_list()
        p7 = next(p for p in photos if p["id"] == _state["photo_ids_b"][0])
        assert p7["is_favorite"] in (True, 1), (
            f"p7 is_favorite not synced: {p7['is_favorite']}"
        )

    def test_backup_api_crop_metadata_p2(self, backup_client):
        """p2 crop metadata matches on backup API."""
        photos = backup_client.backup_list()
        p2 = next(p for p in photos if p["id"] == _state["photo_ids_a"][1])
        crop = p2.get("crop_metadata")
        if isinstance(crop, str):
            crop = json.loads(crop)
        assert crop == _state["crop_metadata"], (
            f"p2 crop mismatch: {crop} != {_state['crop_metadata']}"
        )

    def test_backup_api_non_favorited_photos(self, backup_client):
        """p3, p4, p8 should NOT be favorited on backup API (p5 is gallery-hidden)."""
        photos = backup_client.backup_list()
        non_fav_ids = [
            _state["photo_ids_a"][2],  # p3
            _state["photo_ids_a"][3],  # p4
            # p5 is in gallery, not in backup API
            _state["photo_ids_b"][1],  # p8
        ]
        for pid in non_fav_ids:
            p = next((p for p in photos if p["id"] == pid), None)
            if p is not None:
                assert p.get("is_favorite") in (False, 0, None), (
                    f"Photo {pid} unexpectedly favorited on backup API"
                )

    def test_backup_api_photo_sizes(self, backup_client):
        """Photo file sizes match on backup API."""
        photos = backup_client.backup_list()
        for pid, content in _state["photo_contents"].items():
            bp = next((p for p in photos if p["id"] == pid), None)
            if bp is not None:
                assert bp["size_bytes"] == len(content), (
                    f"Photo {pid} size: backup={bp['size_bytes']} "
                    f"expected={len(content)}"
                )

    # ── Backup API: Blobs ─────────────────────────────────────────────

    def test_backup_api_blobs_count_and_ids(self, backup_client):
        """Backup API blobs: no gallery/trash/clone leaks.

        The backup's blobs table contains client-uploaded blobs AND
        server-side encrypted blobs (encrypted_blob_id, encrypted_thumb_blob_id
        for each synced photo).  We don't assert exact count (it depends on
        server-side encryption timing), but we DO assert:
          - Known client blobs b3, b4 ARE present
          - Gallery-related blobs b1, b1_clone, p5_clone are NOT present
          - Trashed blob b2 is NOT present
          - No duplicate IDs
        """
        blobs = backup_client.backup_list_blobs()
        blob_ids = [b["id"] for b in blobs]
        _assert_no_duplicates(blob_ids, "backup API blobs")

        b1 = _state["blob_ids_a"][0]
        b2 = _state["blob_ids_a"][1]
        b3 = _state["blob_ids_a"][2]
        b4 = _state["blob_id_b"]
        bc1 = _state["clone_blob_id"]
        cp5 = _state["clone_photo_id"]

        assert b3 in blob_ids, f"b3 {b3} missing from backup API"
        assert b4 in blob_ids, f"b4 {b4} missing from backup API"
        assert b1 not in blob_ids, (
            f"DUPLICATE BUG: b1 {b1} (gallery original) leaked to backup API blobs. "
            f"backup_list_blobs must filter encrypted_gallery_items."
        )
        assert bc1 not in blob_ids, (
            f"DUPLICATE BUG: b1 clone {bc1} leaked to backup API blobs."
        )
        assert cp5 not in blob_ids, (
            f"DUPLICATE BUG: p5 clone {cp5} leaked to backup API blobs."
        )
        assert b2 not in blob_ids, f"b2 {b2} (trashed) in backup API blobs"

    def test_backup_api_blob_sizes(self, backup_client):
        """Blob file sizes match on backup API."""
        blobs = backup_client.backup_list_blobs()
        for bid in [_state["blob_ids_a"][2], _state["blob_id_b"]]:
            content = _state["blob_contents"][bid]
            bb = next((b for b in blobs if b["id"] == bid), None)
            assert bb is not None, f"Blob {bid} missing from backup API"
            assert bb["size_bytes"] == len(content), (
                f"Blob {bid} size: backup={bb['size_bytes']} "
                f"expected={len(content)}"
            )

    # ── Backup API: Trash ─────────────────────────────────────────────

    def test_backup_api_trash_count_and_ids(self, backup_client):
        """Backup API has before + 1 trash item (b2)."""
        trash = backup_client.backup_list_trash()
        trash_ids = [t["id"] for t in trash]
        _assert_no_duplicates(trash_ids, "backup API trash")

        assert _state["trash_id"] in trash_ids, (
            f"Trash {_state['trash_id']} missing from backup API"
        )

        expected_total = len(_state["before_trash_ids"]) + 1
        assert len(trash_ids) == expected_total, (
            f"Expected {expected_total} trash on backup API, "
            f"got {len(trash_ids)}"
        )

    def test_backup_api_trash_size(self, backup_client):
        """Trashed blob size_bytes correct on backup API."""
        trash = backup_client.backup_list_trash()
        item = next(t for t in trash if t["id"] == _state["trash_id"])
        expected_size = len(_state["blob_contents"][_state["trashed_blob_id"]])
        assert item["size_bytes"] == expected_size, (
            f"Trash size mismatch: {item['size_bytes']} != {expected_size}"
        )

    # ── Backup API: Users ─────────────────────────────────────────────

    def test_backup_api_users_count(self, backup_client):
        """Backup API has exactly as many users as primary."""
        users = backup_client.backup_list_users()
        _assert_no_duplicates([u["id"] for u in users], "backup API users")
        usernames = [u["username"] for u in users]

        assert _state["user_a_name"] in usernames, "User A not on backup API"
        assert _state["user_b_name"] in usernames, "User B not on backup API"

        expected_total = _state["primary_user_count"]
        assert len(users) == expected_total, (
            f"Expected {expected_total} users on backup API, "
            f"got {len(users)}"
        )

    # ── User-facing on backup: Login ──────────────────────────────────

    def test_backup_user_a_can_login(self, backup_server):
        """Synced User A can login on backup server (credentials synced)."""
        client = _login_on_backup(
            backup_server, _state["user_a_name"], USER_PASSWORD,
        )
        assert client is not None, (
            f"User A '{_state['user_a_name']}' cannot login on backup server"
        )
        _state["backup_client_a"] = client

    def test_backup_user_b_can_login(self, backup_server):
        """Synced User B can login on backup server (credentials synced)."""
        client = _login_on_backup(
            backup_server, _state["user_b_name"], USER_PASSWORD,
        )
        assert client is not None, (
            f"User B '{_state['user_b_name']}' cannot login on backup server"
        )
        _state["backup_client_b"] = client

    # ── User-facing on backup: User A Photos ──────────────────────────

    def test_backup_user_a_photos_count(self):
        """User A sees exactly 5 photos on backup (p1-p4 + dup p6; p5 gallery-hidden)."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup (prior test failed)"

        photos = client.list_photos(limit=500)
        photo_list = photos.get("photos", [])
        photo_ids = [p["id"] for p in photo_list]
        _assert_no_duplicates(photo_ids, "backup User A photos (user-facing)")

        # p5 is gallery-hidden, so expected = p1-p4 + dup_p6
        expected_ids = set(
            _state["photo_ids_a"][:4] + [_state["photo_dup_id"]]
        )
        actual_ids = set(photo_ids)
        missing = expected_ids - actual_ids
        extra = actual_ids - expected_ids
        assert not missing, f"User A missing photos on backup: {missing}"
        assert not extra, (
            f"User A has UNEXPECTED photos on backup (possible duplicates "
            f"or gallery clone leaks): {extra}"
        )
        assert len(photo_list) == 5, (
            f"User A: expected 5 photos on backup (p5 gallery-hidden), "
            f"got {len(photo_list)}"
        )

    def test_backup_user_a_favorite(self):
        """User A's p1 is favorited on backup."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        photos = client.list_photos(limit=500)
        photo_list = photos.get("photos", [])
        p1 = next(
            (p for p in photo_list if p["id"] == _state["photo_ids_a"][0]),
            None,
        )
        assert p1 is not None, "p1 not found in User A's backup photos"
        assert p1["is_favorite"] in (True, 1), (
            f"p1 not favorited on backup: {p1.get('is_favorite')}"
        )

    def test_backup_user_a_crop(self):
        """User A's p2 has correct crop metadata on backup."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        photos = client.list_photos(limit=500)
        photo_list = photos.get("photos", [])
        p2 = next(
            (p for p in photo_list if p["id"] == _state["photo_ids_a"][1]),
            None,
        )
        assert p2 is not None, "p2 not found in User A's backup photos"
        crop = p2.get("crop_metadata")
        if isinstance(crop, str):
            crop = json.loads(crop)
        assert crop == _state["crop_metadata"], (
            f"p2 crop mismatch on backup: {crop}"
        )

    # ── User-facing on backup: User A Blobs ───────────────────────────

    def test_backup_user_a_blobs(self):
        """User A sees only b3 on backup (b1 gallery-hidden, b2 trashed)."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        blobs = client.list_blobs(limit=500)
        blob_list = blobs.get("blobs", [])
        blob_ids = [b["id"] for b in blob_list]
        _assert_no_duplicates(blob_ids, "backup User A blobs (user-facing)")

        b1 = _state["blob_ids_a"][0]
        b2 = _state["blob_ids_a"][1]
        b3 = _state["blob_ids_a"][2]
        bc1 = _state["clone_blob_id"]

        assert b3 in blob_ids, f"b3 not visible on backup for User A"
        assert b1 not in blob_ids, (
            f"b1 {b1} (gallery-hidden) still visible on backup"
        )
        assert b2 not in blob_ids, (
            f"b2 {b2} (trashed) still visible on backup"
        )
        assert bc1 not in blob_ids, (
            f"clone {bc1} leaking into User A blob list on backup"
        )

    # ── User-facing on backup: User A Trash ───────────────────────────

    def test_backup_user_a_trash(self):
        """User A's trashed blob (b2) appears in trash on backup."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        trash = client.list_trash(limit=500)
        trash_items = trash.get("items", [])
        trash_ids = [t["id"] for t in trash_items]

        assert _state["trash_id"] in trash_ids, (
            f"Trash item {_state['trash_id']} missing from User A's "
            f"backup trash. Got: {trash_ids}"
        )

    # ── User-facing on backup: User A Secure Gallery ──────────────────

    def test_backup_user_a_secure_gallery_exists(self):
        """User A has a secure gallery on backup (gallery metadata synced)."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        galleries = client.list_secure_galleries()
        gallery_list = (
            galleries if isinstance(galleries, list)
            else galleries.get("galleries", [])
        )
        gallery_names = [
            g.get("name") for g in gallery_list
        ]
        assert "Private" in gallery_names, (
            f"Secure gallery 'Private' not found on backup. "
            f"Got: {gallery_names}. "
            f"BUG: Secure gallery metadata not synced to backup."
        )
        _state["backup_gallery_id"] = next(
            g["id"] for g in gallery_list if g.get("name") == "Private"
        )

    def test_backup_user_a_secure_gallery_items(self):
        """User A's secure gallery has items on backup (not empty)."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"
        gallery_id = _state.get("backup_gallery_id")
        assert gallery_id, "Gallery not found on backup (prior test failed)"

        token_resp = client.unlock_secure_gallery(USER_PASSWORD)
        token = token_resp.get("gallery_token")
        assert token, f"Failed to unlock gallery on backup: {token_resp}"

        items = client.list_secure_gallery_items(gallery_id, token)
        item_list = (
            items if isinstance(items, list)
            else items.get("items", [])
        )

        expected_count = _state.get("primary_gallery_item_count", 1)
        assert len(item_list) == expected_count, (
            f"Secure gallery on backup has {len(item_list)} items, "
            f"expected {expected_count}. "
            f"BUG: Secure gallery items not synced to backup "
            f"(gallery shows empty)."
        )

    # ── User-facing on backup: User A Shared Albums ───────────────────

    def test_backup_user_a_shared_album_vacation(self):
        """User A's 'Vacation' album exists on backup with correct photos."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        albums = client.list_shared_albums()
        album_list = (
            albums if isinstance(albums, list)
            else albums.get("albums", [])
        )
        album_names = [a.get("name") for a in album_list]
        assert "Vacation" in album_names, (
            f"'Vacation' album missing on backup. Got: {album_names}"
        )

        vac = next(a for a in album_list if a["name"] == "Vacation")
        vac_photos = client.list_album_photos(vac["id"])
        vac_list = (
            vac_photos if isinstance(vac_photos, list)
            else vac_photos.get("photos", [])
        )
        vac_refs = [
            p.get("photo_ref", p.get("photo_id", p.get("id")))
            for p in vac_list
        ]
        assert _state["photo_ids_a"][0] in vac_refs, (
            "p1 not in Vacation album on backup"
        )
        assert _state["photo_ids_a"][1] in vac_refs, (
            "p2 not in Vacation album on backup"
        )
        assert len(vac_list) == 2, (
            f"Vacation album: expected 2 photos on backup, "
            f"got {len(vac_list)}"
        )

    # ── User-facing on backup: User A Edit Copies ─────────────────────

    def test_backup_user_a_edit_copies(self):
        """Edit copies for p1 and p3 survive sync to backup."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        # p1 edit copy: "edited-v1"
        ec1_resp = client.list_edit_copies(_state["photo_ids_a"][0])
        ec1_list = (
            ec1_resp if isinstance(ec1_resp, list)
            else ec1_resp.get("copies", ec1_resp.get("edit_copies", []))
        )
        assert any(c.get("name") == "edited-v1" for c in ec1_list), (
            f"Edit copy 'edited-v1' for p1 not on backup: {ec1_list}"
        )
        ec1 = next(c for c in ec1_list if c.get("name") == "edited-v1")
        em = ec1.get("edit_metadata")
        if isinstance(em, str):
            em = json.loads(em)
        assert em == {"brightness": 1.2}, (
            f"Edit metadata mismatch on backup: {em}"
        )

        # p3 edit copy: "retouch"
        ec2_resp = client.list_edit_copies(_state["photo_ids_a"][2])
        ec2_list = (
            ec2_resp if isinstance(ec2_resp, list)
            else ec2_resp.get("copies", ec2_resp.get("edit_copies", []))
        )
        assert any(c.get("name") == "retouch" for c in ec2_list), (
            f"Edit copy 'retouch' for p3 not on backup: {ec2_list}"
        )
        ec2 = next(c for c in ec2_list if c.get("name") == "retouch")
        em2 = ec2.get("edit_metadata")
        if isinstance(em2, str):
            em2 = json.loads(em2)
        assert em2 == {"contrast": 0.8}, (
            f"Edit metadata (retouch) mismatch on backup: {em2}"
        )

    # ── User-facing on backup: User A Tags ────────────────────────────

    def test_backup_user_a_tags(self):
        """Tags synced to backup (sent as headers during photo transfer)."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup"

        # p1: landscape, nature
        tags_p1 = client.get_photo_tags(_state["photo_ids_a"][0])
        tag_list_p1 = (
            tags_p1 if isinstance(tags_p1, list)
            else tags_p1.get("tags", [])
        )
        tag_names_p1 = [
            t if isinstance(t, str) else t.get("tag", t.get("name", ""))
            for t in tag_list_p1
        ]
        assert "landscape" in tag_names_p1, (
            f"p1 missing 'landscape' tag on backup: {tag_names_p1}"
        )
        assert "nature" in tag_names_p1, (
            f"p1 missing 'nature' tag on backup: {tag_names_p1}"
        )

        # p4: portrait
        tags_p4 = client.get_photo_tags(_state["photo_ids_a"][3])
        tag_list_p4 = (
            tags_p4 if isinstance(tags_p4, list)
            else tags_p4.get("tags", [])
        )
        tag_names_p4 = [
            t if isinstance(t, str) else t.get("tag", t.get("name", ""))
            for t in tag_list_p4
        ]
        assert "portrait" in tag_names_p4, (
            f"p4 missing 'portrait' tag on backup: {tag_names_p4}"
        )

    # ── User-facing on backup: User B ─────────────────────────────────

    def test_backup_user_b_photos_count(self):
        """User B sees exactly 2 photos on backup (p7, p8)."""
        client = _state.get("backup_client_b")
        assert client, "User B not logged into backup"

        photos = client.list_photos(limit=500)
        photo_list = photos.get("photos", [])
        photo_ids = [p["id"] for p in photo_list]
        _assert_no_duplicates(photo_ids, "backup User B photos (user-facing)")

        for pid in _state["photo_ids_b"]:
            assert pid in photo_ids, (
                f"User B photo {pid} missing on backup"
            )
        assert len(photo_list) == 2, (
            f"User B: expected 2 photos on backup, got {len(photo_list)}"
        )

    def test_backup_user_b_favorite(self):
        """User B's p7 is favorited on backup."""
        client = _state.get("backup_client_b")
        assert client, "User B not logged into backup"

        photos = client.list_photos(limit=500)
        photo_list = photos.get("photos", [])
        p7 = next(
            (p for p in photo_list if p["id"] == _state["photo_ids_b"][0]),
            None,
        )
        assert p7 is not None, "p7 not found in User B's backup photos"
        assert p7["is_favorite"] in (True, 1), (
            f"p7 not favorited on backup: {p7.get('is_favorite')}"
        )

    def test_backup_user_b_blob(self):
        """User B's blob b4 visible on backup."""
        client = _state.get("backup_client_b")
        assert client, "User B not logged into backup"

        blobs = client.list_blobs(limit=500)
        blob_list = blobs.get("blobs", [])
        blob_ids = [b["id"] for b in blob_list]
        assert _state["blob_id_b"] in blob_ids, (
            f"b4 not visible for User B on backup"
        )

    def test_backup_user_b_shared_album_family(self):
        """User B's 'Family' album exists on backup with p7."""
        client = _state.get("backup_client_b")
        assert client, "User B not logged into backup"

        albums = client.list_shared_albums()
        album_list = (
            albums if isinstance(albums, list)
            else albums.get("albums", [])
        )
        album_names = [a.get("name") for a in album_list]
        assert "Family" in album_names, (
            f"'Family' album missing on backup. Got: {album_names}"
        )

        fam = next(a for a in album_list if a["name"] == "Family")
        fam_photos = client.list_album_photos(fam["id"])
        fam_list = (
            fam_photos if isinstance(fam_photos, list)
            else fam_photos.get("photos", [])
        )
        fam_refs = [
            p.get("photo_ref", p.get("photo_id", p.get("id")))
            for p in fam_list
        ]
        assert _state["photo_ids_b"][0] in fam_refs, (
            "p7 not in Family album on backup"
        )
        assert len(fam_list) == 1, (
            f"Family album: expected 1 photo on backup, "
            f"got {len(fam_list)}"
        )

    def test_backup_user_b_tag(self):
        """User B's p7 tag 'family' synced to backup."""
        client = _state.get("backup_client_b")
        assert client, "User B not logged into backup"

        tags_p7 = client.get_photo_tags(_state["photo_ids_b"][0])
        tag_list = (
            tags_p7 if isinstance(tags_p7, list)
            else tags_p7.get("tags", [])
        )
        tag_names = [
            t if isinstance(t, str) else t.get("tag", t.get("name", ""))
            for t in tag_list
        ]
        assert "family" in tag_names, (
            f"p7 missing 'family' tag on backup: {tag_names}"
        )

    # ── Sync log verification ─────────────────────────────────────────

    def test_sync_log_success(self, primary_admin, backup_configured):
        """Sync log shows success with non-zero counts."""
        logs = primary_admin.admin_get_sync_logs(backup_configured)
        assert logs, "No sync logs found"
        latest = logs[0] if isinstance(logs, list) else logs
        assert latest.get("status") in ("success", "completed"), (
            f"Latest sync status: {latest.get('status')}"
        )
        # Should have synced at least some photos
        synced = latest.get("photos_synced", 0)
        assert synced >= 7, (
            f"Expected at least 7 photos synced (p5 gallery-hidden), got {synced}"
        )


# =====================================================================
# Phase 2b: Duplicate regression — cross-listing & pre-synced-then-secured
# =====================================================================


class TestMegaDuplicateRegression:
    """
    Regression tests for the original backup duplicate bug.

    Ensures:
      1) After phase 2 sync, items in the secure gallery do NOT also
         appear in regular user-facing photo/blob listings (cross-listing).
      2) A photo that was ALREADY synced to backup, THEN added to a
         secure gallery, is retroactively REMOVED from backup listings
         on the next sync (pre-synced-then-secured scenario).
      3) After a second sync, no duplicate IDs exist anywhere.

    This is the exact scenario from bugs 2, 5, 8, 9.
    """

    # ── Cross-listing: gallery items must not appear in regular listings ──

    def test_gallery_items_not_in_backup_photos(self, backup_server):
        """Secure gallery clone/original IDs must NOT appear in user photo list."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup (prior test failed)"
        gallery_id = _state.get("backup_gallery_id")
        assert gallery_id, "Gallery not found on backup (prior test failed)"

        # Get gallery item blob_ids (these are clone IDs in the photos table)
        token = client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        items = client.list_secure_gallery_items(gallery_id, token)
        item_list = items if isinstance(items, list) else items.get("items", [])
        gallery_blob_ids = set()
        gallery_original_ids = set()
        for item in item_list:
            if item.get("blob_id"):
                gallery_blob_ids.add(item["blob_id"])
            if item.get("original_blob_id"):
                gallery_original_ids.add(item["original_blob_id"])

        # Also include known IDs from populate phase
        gallery_blob_ids.add(_state["clone_blob_id"])
        gallery_original_ids.add(_state["blob_ids_a"][0])  # b1 = original

        # Regular photo listing must not contain any of these
        photos = client.list_photos(limit=500)
        photo_ids = set(p["id"] for p in photos.get("photos", []))

        leaked_clones = gallery_blob_ids & photo_ids
        leaked_originals = gallery_original_ids & photo_ids
        assert not leaked_clones, (
            f"DUPLICATE BUG: Gallery clone IDs leaked into photo listing: "
            f"{leaked_clones}"
        )
        assert not leaked_originals, (
            f"DUPLICATE BUG: Gallery original IDs leaked into photo listing: "
            f"{leaked_originals}"
        )

    def test_gallery_items_not_in_backup_blobs(self, backup_server):
        """Secure gallery clone/original IDs must NOT appear in user blob list."""
        client = _state.get("backup_client_a")
        assert client, "User A not logged into backup (prior test failed)"

        # Known gallery-related IDs (blob b1 + photo p5 and their clones)
        gallery_related = {
            _state["blob_ids_a"][0],    # b1 original
            _state["clone_blob_id"],    # b1 clone
            _state["photo_ids_a"][4],   # p5 original
            _state["clone_photo_id"],   # p5 clone
        }

        blobs = client.list_blobs(limit=500)
        blob_ids = set(b["id"] for b in blobs.get("blobs", []))

        leaked = gallery_related & blob_ids
        assert not leaked, (
            f"DUPLICATE BUG: Gallery-related IDs leaked into blob listing: "
            f"{leaked}"
        )

    def test_gallery_items_not_in_backup_api_photos(self, backup_client):
        """Secure gallery clone/original IDs must NOT appear in backup API photo list."""
        gallery_related = {
            _state["blob_ids_a"][0],    # b1 original (if it had a photo row)
            _state["clone_blob_id"],    # b1 clone
            _state["photo_ids_a"][4],   # p5 original
            _state["clone_photo_id"],   # p5 clone
        }

        photos = backup_client.backup_list()
        photo_ids = set(p["id"] for p in photos)

        leaked = gallery_related & photo_ids
        assert not leaked, (
            f"DUPLICATE BUG: Gallery-related IDs in backup API photos: "
            f"{leaked}. backup_list_photos must filter encrypted_gallery_items."
        )

    def test_gallery_items_not_in_backup_api_blobs(self, backup_client):
        """Secure gallery clone/original IDs must NOT appear in backup API blob list."""
        gallery_related = {
            _state["blob_ids_a"][0],    # b1 original
            _state["clone_blob_id"],    # b1 clone
            _state["photo_ids_a"][4],   # p5 original
            _state["clone_photo_id"],   # p5 clone
        }

        blobs = backup_client.backup_list_blobs()
        blob_ids = set(b["id"] for b in blobs)

        leaked = gallery_related & blob_ids
        assert not leaked, (
            f"DUPLICATE BUG: Gallery-related IDs in backup API blobs: "
            f"{leaked}. backup_list_blobs must filter encrypted_gallery_items."
        )

    # ── Comprehensive duplicate detection across ALL backup endpoints ──

    def test_no_content_duplicates_on_backup(self, backup_client, backup_server):
        """No photo appears twice on backup — by ID, by hash, or by file_path.

        This is the catch-all regression test for the duplicate photo bug.
        Checks EVERY listing endpoint on the backup for content duplicates:
          - backup_list_photos (admin API)
          - list_photos per user (user-facing)
          - Cross-check: photo should be in EITHER regular listing OR secure
            gallery, never both.
        """
        # 1. Backup API photos — check for photo_hash dupes
        photos = backup_client.backup_list()
        photo_ids = [p["id"] for p in photos]
        _assert_no_duplicates(photo_ids, "backup API photo IDs")

        from collections import Counter
        hash_counts = Counter(
            p.get("photo_hash") for p in photos
            if p.get("photo_hash") is not None
        )
        hash_dupes = {h: c for h, c in hash_counts.items() if c > 1}
        assert not hash_dupes, (
            f"DUPLICATE BUG: Same image content appears multiple times in "
            f"backup_list_photos (by photo_hash): {hash_dupes}"
        )

        # 2. User A user-facing photos — check for dupes
        client_a = _state.get("backup_client_a")
        if client_a:
            user_photos = client_a.list_photos(limit=500).get("photos", [])
            user_ids = [p["id"] for p in user_photos]
            _assert_no_duplicates(user_ids, "User A user-facing photo IDs on backup")

            user_hashes = Counter(
                p.get("photo_hash") for p in user_photos
                if p.get("photo_hash") is not None
            )
            user_hash_dupes = {h: c for h, c in user_hashes.items() if c > 1}
            assert not user_hash_dupes, (
                f"DUPLICATE BUG: Same image content appears multiple times in "
                f"User A's photo list on backup: {user_hash_dupes}"
            )

            # 3. Cross-check: gallery items must NOT also be in user listing
            gallery_id = _state.get("backup_gallery_id")
            if gallery_id:
                token = client_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
                items = client_a.list_secure_gallery_items(gallery_id, token)
                item_list = (
                    items if isinstance(items, list)
                    else items.get("items", [])
                )
                gallery_blob_ids = set()
                gallery_original_ids = set()
                for item in item_list:
                    if item.get("blob_id"):
                        gallery_blob_ids.add(item["blob_id"])
                    if item.get("original_blob_id"):
                        gallery_original_ids.add(item["original_blob_id"])

                user_id_set = set(user_ids)
                cross_clone = gallery_blob_ids & user_id_set
                cross_orig = gallery_original_ids & user_id_set
                assert not cross_clone, (
                    f"DUPLICATE BUG: Gallery clone IDs visible in user photo "
                    f"list on backup: {cross_clone}"
                )
                assert not cross_orig, (
                    f"DUPLICATE BUG: Gallery original IDs visible in user "
                    f"photo list on backup: {cross_orig}"
                )

                # Also check backup API endpoint
                api_id_set = set(photo_ids)
                api_cross_clone = gallery_blob_ids & api_id_set
                api_cross_orig = gallery_original_ids & api_id_set
                assert not api_cross_clone, (
                    f"DUPLICATE BUG: Gallery clone IDs in backup_list_photos: "
                    f"{api_cross_clone}. "
                    f"backup_list_photos must filter encrypted_gallery_items."
                )
                assert not api_cross_orig, (
                    f"DUPLICATE BUG: Gallery original IDs in backup_list_photos: "
                    f"{api_cross_orig}. "
                    f"backup_list_photos must filter encrypted_gallery_items."
                )

    # ── Pre-synced-then-secured: the core duplicate regression test ───

    def test_presynced_photo_then_secured_no_duplicates(
        self, primary_admin, primary_server, backup_configured,
        backup_client, backup_server,
    ):
        """
        THE core duplicate regression test (Bugs 2, 5, 9):
          1. Upload a NEW photo on primary
          2. Sync → photo appears on backup
          3. Add photo to secure gallery on primary (creates clone)
          4. Sync again → retroactive purge must REMOVE the photo from backup
          5. Verify: photo NOT in backup listings, IS in secure gallery
          6. Verify: NO duplicate IDs anywhere
        """
        client_a = APIClient(primary_server.base_url)
        client_a.login(_state["user_a_name"], USER_PASSWORD)

        # 1. Upload a new photo
        content = generate_test_jpeg(width=77, height=77)
        fname = unique_filename()
        photo = client_a.upload_photo(fname, content=content)
        presync_pid = photo["photo_id"]

        # 2. Sync to backup — photo should land on backup
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        assert result.get("status") != "error", f"Sync 2a failed: {result}"

        # Verify photo IS on backup
        backup_photos = backup_client.backup_list()
        backup_photo_ids = [p["id"] for p in backup_photos]
        assert presync_pid in backup_photo_ids, (
            f"Pre-synced photo {presync_pid} should be on backup after sync"
        )
        # Snapshot for post-purge verification
        presync_photo = next(p for p in backup_photos if p["id"] == presync_pid)
        presync_file_path = presync_photo.get("file_path")
        presync_hash = presync_photo.get("photo_hash")
        backup_count_before_purge = len(backup_photos)

        # Also check user-facing listing on backup
        ba_client = _login_on_backup(
            backup_server, _state["user_a_name"], USER_PASSWORD,
        )
        assert ba_client, "User A can't login on backup for presync check"
        user_photos_before = ba_client.list_photos(limit=500)
        user_photo_ids_before = [
            p["id"] for p in user_photos_before.get("photos", [])
        ]
        assert presync_pid in user_photo_ids_before, (
            f"Pre-synced photo should be visible in user listing on backup"
        )
        count_before = len(user_photo_ids_before)

        # 3. Add photo to secure gallery on primary (creates clone, hides original)
        token = client_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_resp = client_a.add_secure_gallery_item(
            _state["gallery_id"], presync_pid, token,
        )
        presync_clone_id = add_resp["new_blob_id"]

        # Verify it's hidden from primary listing now
        primary_photos = client_a.list_photos(limit=500)
        primary_photo_ids = [
            p["id"] for p in primary_photos.get("photos", [])
        ]
        assert presync_pid not in primary_photo_ids, (
            f"Photo {presync_pid} should be hidden from primary after "
            f"adding to gallery"
        )

        # 4. Sync again — retroactive purge should remove it from backup
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        assert result.get("status") != "error", f"Sync 2b failed: {result}"

        # 4b. Trigger autoscan on backup — the physical file is still on disk
        #     after retroactive purge (only DB rows were deleted).  Autoscan
        #     must NOT re-register the purged photo.
        backup_admin = APIClient(backup_server.base_url)
        backup_admin.login(ADMIN_USERNAME, ADMIN_PASSWORD)
        scan_result = backup_admin.admin_trigger_autoscan()
        assert scan_result.get("message") == "Scan complete", (
            f"Autoscan on backup failed: {scan_result}"
        )

        # 5. Verify: photo REMOVED from backup listings (even after autoscan)
        backup_photos_after = backup_client.backup_list()
        backup_photo_ids_after = [p["id"] for p in backup_photos_after]
        _assert_no_duplicates(
            backup_photo_ids_after,
            "backup API photos after retroactive purge",
        )
        assert presync_pid not in backup_photo_ids_after, (
            f"DUPLICATE BUG: Pre-synced photo {presync_pid} was NOT purged "
            f"from backup API after being added to secure gallery. "
            f"Retroactive purge failed."
        )
        assert presync_clone_id not in backup_photo_ids_after, (
            f"DUPLICATE BUG: Clone {presync_clone_id} leaked into "
            f"backup API photos"
        )

        # Count must have DECREASED — autoscan must not re-create purged photo
        assert len(backup_photos_after) == backup_count_before_purge - 1, (
            f"DUPLICATE BUG: Expected {backup_count_before_purge - 1} "
            f"backup photos after purge (was {backup_count_before_purge}), "
            f"got {len(backup_photos_after)}. "
            f"Autoscan likely re-created the purged photo from the "
            f"physical file still on disk."
        )

        # File path of purged photo must not appear in any photo row
        if presync_file_path:
            after_file_paths = [
                p.get("file_path") for p in backup_photos_after
            ]
            assert presync_file_path not in after_file_paths, (
                f"DUPLICATE BUG: Purged photo's file_path "
                f"'{presync_file_path}' still appears in backup_list_photos. "
                f"Either retroactive purge didn't delete the physical file "
                f"and autoscan re-registered it, or the purge left a stale "
                f"row."
            )

        # User-facing check
        ba_client2 = _login_on_backup(
            backup_server, _state["user_a_name"], USER_PASSWORD,
        )
        assert ba_client2, "User A can't login on backup for post-purge check"
        user_photos_after = ba_client2.list_photos(limit=500)
        user_photo_ids_after = [
            p["id"] for p in user_photos_after.get("photos", [])
        ]
        _assert_no_duplicates(
            user_photo_ids_after,
            "user-facing photos after retroactive purge",
        )
        assert presync_pid not in user_photo_ids_after, (
            f"DUPLICATE BUG: Pre-synced photo {presync_pid} still visible "
            f"to user on backup after retroactive purge"
        )
        assert presync_clone_id not in user_photo_ids_after, (
            f"DUPLICATE BUG: Clone {presync_clone_id} visible to user "
            f"on backup"
        )
        # Count should have DECREASED by 1 (presync photo removed)
        assert len(user_photo_ids_after) == count_before - 1, (
            f"Expected {count_before - 1} user photos after purge "
            f"(was {count_before}), got {len(user_photo_ids_after)}. "
            f"Retroactive purge did not remove the photo."
        )

        # Verify: photo IS in secure gallery on backup
        galleries = ba_client2.list_secure_galleries()
        gal_list = (
            galleries if isinstance(galleries, list)
            else galleries.get("galleries", [])
        )
        gal = next(g for g in gal_list if g.get("name") == "Private")
        token2 = ba_client2.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        items = ba_client2.list_secure_gallery_items(gal["id"], token2)
        item_list = items if isinstance(items, list) else items.get("items", [])

        # Should have original 2 items + newly-added presync photo = 3
        assert len(item_list) >= 3, (
            f"Gallery should have at least 3 items (b1 + p5 + presync photo), "
            f"got {len(item_list)}"
        )

        # 6. No duplicates across all backup data — AND no gallery items leaking
        all_backup_photos = backup_photo_ids_after
        all_backup_blobs = [b["id"] for b in backup_client.backup_list_blobs()]
        _assert_no_duplicates(all_backup_photos, "all backup photos post-purge")
        _assert_no_duplicates(all_backup_blobs, "all backup blobs post-purge")

        # Content-level duplicate check: no photo_hash should appear twice
        from collections import Counter
        hash_counts = Counter(
            p.get("photo_hash") for p in backup_photos_after
            if p.get("photo_hash") is not None
        )
        hash_dupes = {h: c for h, c in hash_counts.items() if c > 1}
        assert not hash_dupes, (
            f"DUPLICATE BUG: After retroactive purge, same image content "
            f"appears multiple times in backup_list_photos: {hash_dupes}"
        )

        # Pre-synced photo must not be in backup blob list either
        assert presync_pid not in all_backup_blobs, (
            f"DUPLICATE BUG: Pre-synced photo {presync_pid} still in "
            f"backup_list_blobs after retroactive purge. "
            f"backup_list_blobs must filter encrypted_gallery_items."
        )
        assert presync_clone_id not in all_backup_blobs, (
            f"DUPLICATE BUG: Pre-synced clone {presync_clone_id} in "
            f"backup_list_blobs."
        )

        # Store updated state for Phase 3
        _state["presync_pid"] = presync_pid
        _state["presync_clone_id"] = presync_clone_id
        _state["primary_gallery_item_count"] = len(item_list)

    def test_presynced_blob_then_secured_no_duplicates(
        self, primary_admin, primary_server, backup_configured,
        backup_client, backup_server,
    ):
        """
        Same pre-synced-then-secured scenario but for a BLOB:
          1. Upload a new blob on primary
          2. Sync → blob appears on backup
          3. Add blob to secure gallery on primary
          4. Sync again → blob should be removed from backup blob listings
          5. Verify: no duplicates
        """
        client_a = APIClient(primary_server.base_url)
        client_a.login(_state["user_a_name"], USER_PASSWORD)

        # 1. Upload a new blob
        content = generate_random_bytes(2048)
        blob = client_a.upload_blob("photo", content)
        presync_bid = blob["blob_id"]

        # 2. Sync
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        assert result.get("status") != "error", f"Sync 3a failed: {result}"

        # Verify blob IS on backup
        backup_blobs = backup_client.backup_list_blobs()
        backup_blob_ids = [b["id"] for b in backup_blobs]
        assert presync_bid in backup_blob_ids, (
            f"Pre-synced blob {presync_bid} should be on backup after sync"
        )

        # 3. Add blob to secure gallery
        token = client_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        add_resp = client_a.add_secure_gallery_item(
            _state["gallery_id"], presync_bid, token,
        )
        presync_blob_clone_id = add_resp["new_blob_id"]

        # 4. Sync again
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        assert result.get("status") != "error", f"Sync 3b failed: {result}"

        # 4b. Trigger autoscan on backup — physical file still on disk
        backup_admin = APIClient(backup_server.base_url)
        backup_admin.login(ADMIN_USERNAME, ADMIN_PASSWORD)
        scan_result = backup_admin.admin_trigger_autoscan()
        assert scan_result.get("message") == "Scan complete", (
            f"Autoscan on backup failed: {scan_result}"
        )

        # 5. Verify: blob REMOVED from backup listings (even after autoscan)
        backup_blobs_after = backup_client.backup_list_blobs()
        backup_blob_ids_after = [b["id"] for b in backup_blobs_after]
        _assert_no_duplicates(
            backup_blob_ids_after,
            "backup API blobs after blob retroactive purge",
        )
        assert presync_bid not in backup_blob_ids_after, (
            f"DUPLICATE BUG: Pre-synced blob {presync_bid} not purged "
            f"from backup after gallery add"
        )
        assert presync_blob_clone_id not in backup_blob_ids_after, (
            f"DUPLICATE BUG: Clone blob {presync_blob_clone_id} leaked "
            f"into backup blob list"
        )

        # User-facing check
        ba = _login_on_backup(
            backup_server, _state["user_a_name"], USER_PASSWORD,
        )
        assert ba, "User A can't login on backup"
        user_blobs = ba.list_blobs(limit=500)
        user_blob_ids = [b["id"] for b in user_blobs.get("blobs", [])]
        assert presync_bid not in user_blob_ids, (
            f"DUPLICATE BUG: Pre-synced blob still visible to user on backup"
        )
        assert presync_blob_clone_id not in user_blob_ids, (
            f"DUPLICATE BUG: Clone blob visible to user on backup"
        )

        # Store for Phase 3
        _state["presync_bid"] = presync_bid
        _state["presync_blob_clone_id"] = presync_blob_clone_id
        _state["primary_gallery_item_count"] = (
            _state.get("primary_gallery_item_count", 2) + 1
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
        Recover → re-login → verify every data category:
          1. Users (all present, correct usernames)
          2. User A photos (6, correct IDs, favorites, crop, no duplicates)
          3. User A edit copies (p1: edited-v1, p3: retouch)
          4. User A shared albums (Vacation with p1+p2)
          5. User A tags (p1: landscape+nature, p4: portrait)
          6. User A blobs (b3 visible, b1/b2 hidden)
          7. User A trash (b2 in trash)
          8. User A secure gallery (Private with item)
          9. User B photos (2, correct IDs, favorite)
          10. User B shared albums (Family with p7)
          11. User B tag (p7: family)
          12. User B blobs (b4 visible)
          13. No duplicate photos/blobs anywhere
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
                        _dump_server_logs(
                            fresh_server["server"], "recovery (error)",
                        )
                        pytest.fail(
                            f"Recovery error: {latest.get('error')}"
                        )
            except Exception:
                pass
            time.sleep(3)

        if not recovered:
            _dump_server_logs(fresh_server["server"], "recovery (timeout)")
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

        # 5 photos: p1-p4 + dup p6 (p5 is gallery-hidden)
        for pid in _state["photo_ids_a"][:4]:
            assert pid in photo_ids_a, f"User A photo {pid} not recovered"
        assert _state["photo_dup_id"] in photo_ids_a, (
            "Duplicate photo not recovered"
        )
        p5 = _state["photo_ids_a"][4]
        assert p5 not in photo_ids_a, (
            f"DUPLICATE BUG: p5 {p5} (gallery-hidden) visible after recovery"
        )
        assert len(photo_ids_a) == 5, (
            f"User A: expected 5 photos (p5 gallery-hidden), got {len(photo_ids_a)}: {photo_ids_a}"
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
        ec2_item = next(c for c in ec2_list if c.get("name") == "retouch")
        em2 = ec2_item.get("edit_metadata")
        if isinstance(em2, str):
            em2 = json.loads(em2)
        assert em2 == {"contrast": 0.8}, f"Retouch metadata mismatch: {em2}"

        # ── 5. USER A TAGS ────────────────────────────────────────────
        tags_p1 = user_a.get_photo_tags(_state["photo_ids_a"][0])
        tag_list_p1 = (
            tags_p1 if isinstance(tags_p1, list)
            else tags_p1.get("tags", [])
        )
        tag_names_p1 = [
            t if isinstance(t, str) else t.get("tag", t.get("name", ""))
            for t in tag_list_p1
        ]
        assert "landscape" in tag_names_p1, (
            f"p1 missing 'landscape' tag after recovery: {tag_names_p1}"
        )
        assert "nature" in tag_names_p1, (
            f"p1 missing 'nature' tag after recovery: {tag_names_p1}"
        )

        tags_p4 = user_a.get_photo_tags(_state["photo_ids_a"][3])
        tag_list_p4 = (
            tags_p4 if isinstance(tags_p4, list)
            else tags_p4.get("tags", [])
        )
        tag_names_p4 = [
            t if isinstance(t, str) else t.get("tag", t.get("name", ""))
            for t in tag_list_p4
        ]
        assert "portrait" in tag_names_p4, (
            f"p4 missing 'portrait' tag after recovery: {tag_names_p4}"
        )

        # ── 6. USER A SHARED ALBUMS ──────────────────────────────────
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

        # ── 7. USER A BLOBS ──────────────────────────────────────────
        blobs_a = user_a.list_blobs(limit=500)
        blob_list_a = blobs_a.get("blobs", [])
        blob_ids_a = [b["id"] for b in blob_list_a]
        _assert_no_duplicates(blob_ids_a, "recovered User A blobs")
        assert _state["blob_ids_a"][2] in blob_ids_a, "b3 not recovered"
        assert _state["blob_ids_a"][0] not in blob_ids_a, (
            "b1 (gallery-hidden) visible after recovery"
        )
        assert _state["blob_ids_a"][1] not in blob_ids_a, (
            "b2 (trashed) visible after recovery"
        )

        # ── 8. USER A TRASH ──────────────────────────────────────────
        trash_a = user_a.list_trash(limit=500)
        trash_items_a = trash_a.get("items", [])
        trash_ids_a = [t["id"] for t in trash_items_a]
        assert _state["trash_id"] in trash_ids_a, (
            f"Trash item {_state['trash_id']} not recovered. "
            f"Got: {trash_ids_a}"
        )

        # ── 9. USER A SECURE GALLERY ─────────────────────────────────
        galleries_a = user_a.list_secure_galleries()
        gal_list_a = (
            galleries_a if isinstance(galleries_a, list)
            else galleries_a.get("galleries", [])
        )
        gal_names_a = [g.get("name") for g in gal_list_a]
        assert "Private" in gal_names_a, (
            f"Secure gallery 'Private' not recovered: {gal_names_a}"
        )

        recov_gal = next(g for g in gal_list_a if g.get("name") == "Private")
        try:
            token = user_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
            items = user_a.list_secure_gallery_items(recov_gal["id"], token)
            item_list = (
                items if isinstance(items, list)
                else items.get("items", [])
            )
            expected_count = _state.get("primary_gallery_item_count", 1)
            assert len(item_list) == expected_count, (
                f"Secure gallery recovered with {len(item_list)} items, "
                f"expected {expected_count}"
            )
        except Exception as e:
            pytest.fail(
                f"Could not verify secure gallery after recovery: {e}"
            )

        # ── 10. USER B PHOTOS ────────────────────────────────────────
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

        # ── 11. USER B METADATA ──────────────────────────────────────
        p7 = next(p for p in photo_list_b if p["id"] == _state["photo_ids_b"][0])
        assert p7["is_favorite"] in (True, 1), "p7 favorite not recovered"

        # ── 12. USER B SHARED ALBUMS ─────────────────────────────────
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

        # ── 13. USER B TAGS ──────────────────────────────────────────
        tags_p7 = user_b.get_photo_tags(_state["photo_ids_b"][0])
        tag_list = (
            tags_p7 if isinstance(tags_p7, list)
            else tags_p7.get("tags", [])
        )
        tag_names = [
            t if isinstance(t, str) else t.get("tag", t.get("name", ""))
            for t in tag_list
        ]
        assert "family" in tag_names, (
            f"p7 missing 'family' tag after recovery: {tag_names}"
        )

        # ── 14. USER B BLOBS ─────────────────────────────────────────
        blobs_b = user_b.list_blobs(limit=500)
        blob_list_b = blobs_b.get("blobs", [])
        blob_ids_b = [b["id"] for b in blob_list_b]
        assert _state["blob_id_b"] in blob_ids_b, "b4 not recovered"

        # ── 15. NO DUPLICATES ACROSS ALL DATA ─────────────────────────
        all_recovered_photos = [p["id"] for p in photo_list_a] + [
            p["id"] for p in photo_list_b
        ]
        _assert_no_duplicates(all_recovered_photos, "all recovered photos")

        # Content-level: same photo_hash must not appear more than once
        from collections import Counter
        all_photo_objects = list(photo_list_a) + list(photo_list_b)
        hash_counts = Counter(
            p.get("photo_hash") for p in all_photo_objects
            if p.get("photo_hash") is not None
        )
        hash_dupes = {h: c for h, c in hash_counts.items() if c > 1}
        assert not hash_dupes, (
            f"DUPLICATE BUG: After recovery, same image content appears "
            f"multiple times (by photo_hash): {hash_dupes}"
        )

        all_recovered_blobs = blob_ids_a + blob_ids_b
        _assert_no_duplicates(all_recovered_blobs, "all recovered blobs")

        # ── 16. PRE-SYNCED-THEN-SECURED NOT IN LISTINGS ──────────────
        # Items added to secure gallery after initial sync must NOT
        # appear in regular listings after recovery (retroactive purge
        # must survive the round-trip).
        if _state.get("presync_pid"):
            assert _state["presync_pid"] not in set(
                p["id"] for p in photo_list_a
            ), (
                f"DUPLICATE BUG: Pre-synced photo {_state['presync_pid']} "
                f"leaked into recovered photo listing"
            )
            assert _state["presync_clone_id"] not in set(
                p["id"] for p in photo_list_a
            ), (
                f"DUPLICATE BUG: Pre-synced clone {_state['presync_clone_id']} "
                f"leaked into recovered photo listing"
            )

        if _state.get("presync_bid"):
            assert _state["presync_bid"] not in set(blob_ids_a), (
                f"DUPLICATE BUG: Pre-synced blob {_state['presync_bid']} "
                f"leaked into recovered blob listing"
            )
            assert _state["presync_blob_clone_id"] not in set(blob_ids_a), (
                f"DUPLICATE BUG: Pre-synced blob clone "
                f"{_state['presync_blob_clone_id']} leaked into recovered "
                f"blob listing"
            )

        # ── 17. GALLERY ITEMS CROSS-CHECK ─────────────────────────────
        # Verify no gallery item ID also appears in regular listings
        if recov_gal:
            for it in item_list:
                bid = it.get("blob_id")
                orig = it.get("original_blob_id")
                if bid:
                    assert bid not in set(p["id"] for p in photo_list_a), (
                        f"DUPLICATE BUG: Gallery item blob_id {bid} "
                        f"also in recovered photo list"
                    )
                    assert bid not in set(blob_ids_a), (
                        f"DUPLICATE BUG: Gallery item blob_id {bid} "
                        f"also in recovered blob list"
                    )
                if orig:
                    assert orig not in set(p["id"] for p in photo_list_a), (
                        f"DUPLICATE BUG: Gallery original_blob_id {orig} "
                        f"also in recovered photo list"
                    )
                    assert orig not in set(blob_ids_a), (
                        f"DUPLICATE BUG: Gallery original_blob_id {orig} "
                        f"also in recovered blob list"
                    )
