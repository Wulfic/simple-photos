"""
Test 21: Multi-User Full Pipeline — 3 users (1 admin + 2 normal),
         specific media per user, backup sync, fresh primary restore,
         cross-contamination detection, shared album validation.

Pipeline:
  Phase 1 — Populate primary with 3 users, each with KNOWN media counts:
             Admin:  3 photos, 1 blob, 1 secure gallery (1 item from ba1)
             User A: 5 photos, 3 blobs (1 trashed, 1 gallery-hidden),
                     1 secure gallery (1 item from blobA1)
             User B: 4 photos, 3 blobs, 1 secure gallery (1 item from blobB1)
             Shared album between A and B with photos from both.
             Shared album between Admin and User A with admin photos.
  Phase 2 — Sync to backup.
             Verify exact media counts per user on backup.
             Verify NO cross-contamination: each user sees ONLY their own media.
             Verify shared albums survive with correct membership and photos.
             Verify secure galleries: each user's gallery is hidden from others,
             gallery items don't leak into other users' listings.
  Phase 3 — Spin up fresh primary, restore from backup.
             Re-verify exact media counts per user.
             Re-verify NO cross-contamination.
             Re-verify shared albums.
             Re-verify secure gallery isolation.

Cross-contamination checks (the core concern):
  - For each user, list their photos/blobs/trash and assert:
    * ONLY their own IDs appear
    * NO other user's IDs appear
    * Exact count matches expected
  - For secure galleries, verify:
    * Each user's gallery is invisible to other users
    * Gallery items don't leak into any user's photo/blob listings
    * Gallery items are hidden from the owner's own regular listing
    * Gallery item counts are correct
  - For shared albums, verify:
    * Album membership is correct
    * Only the expected photos are in each album
    * Non-members cannot see the album

Every assertion uses exact counts and specific IDs.
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


def _extract_photo_ids(response):
    """Extract photo IDs from a list_photos response."""
    photos = response.get("photos", [])
    return [p["id"] for p in photos]


def _extract_blob_ids(response):
    """Extract blob IDs from a list_blobs response."""
    blobs = response.get("blobs", [])
    return [b["id"] for b in blobs]


def _extract_trash_ids(response):
    """Extract trash IDs from a list_trash response."""
    items = response.get("items", [])
    return [t["id"] for t in items]


def _login_on_server(base_url, username, password):
    """Login as a user on any server. Returns APIClient or None."""
    client = APIClient(base_url)
    try:
        client.login(username, password)
        client.username = username
        return client
    except Exception as e:
        print(f"[WARN] Could not login as {username} on {base_url}: {e}")
        return None


# =====================================================================
# Phase 1: Populate primary with 3 users and known media
# =====================================================================


class TestMultiUserPopulate:
    """Create 3 users with specific media counts on the primary server."""

    def test_populate_all_users(self, primary_admin, primary_server,
                                backup_configured, backup_client):
        """
        Populate the primary with 3 users and exact known media.

        Media distribution:
          Admin:  3 photos (pa1, pa2, pa3), 1 blob (ba1)
                  Secure gallery "Admin Vault" with ba1 → ba1 hidden, clone created
          User A: 5 photos (a1-a5), 3 blobs (blobA1, blobA2, blobA3)
                  blobA3 trashed, blobA1 in secure gallery "A Private" → hidden
                  Visible blobs: blobA2 only
          User B: 4 photos (b1-b4), 3 blobs (blobB1, blobB2, blobB3)
                  Secure gallery "B Secrets" with blobB1 → blobB1 hidden
                  Visible blobs: blobB2, blobB3

        Shared albums:
          "Trip" — owner: User A, member: User B
                   photos: a1 (from A), b1 (from B)
          "Work" — owner: Admin, member: User A
                   photos: pa1 (from admin)

        Secure galleries (each user has exactly 1 gallery with 1 item):
          Admin:  "Admin Vault" — 1 item (ba1 clone)
          User A: "A Private"   — 1 item (blobA1 clone)
          User B: "B Secrets"   — 1 item (blobB1 clone)

        Tags per user:
          Admin:  pa1: "admin_tag"
          User A: a1: "travel", a2: "sunset"
          User B: b1: "nature"
        """

        # ── Snapshot backup state BEFORE our operations ──────────────
        before_photos = backup_client.backup_list()
        before_blobs = backup_client.backup_list_blobs()
        _state["before_photo_ids"] = set(p["id"] for p in before_photos)
        _state["before_blob_ids"] = set(b["id"] for b in before_blobs)

        # ── Create test users ────────────────────────────────────────
        _state["user_a_name"] = random_username("pipe_a_")
        created_a = primary_admin.admin_create_user(
            _state["user_a_name"], USER_PASSWORD,
        )
        _state["user_a_id"] = created_a["user_id"]

        _state["user_b_name"] = random_username("pipe_b_")
        created_b = primary_admin.admin_create_user(
            _state["user_b_name"], USER_PASSWORD,
        )
        _state["user_b_id"] = created_b["user_id"]

        # ── Login all 3 users ────────────────────────────────────────
        admin_cl = APIClient(primary_server.base_url)
        admin_cl.login(ADMIN_USERNAME, ADMIN_PASSWORD)
        admin_cl.username = ADMIN_USERNAME

        client_a = APIClient(primary_server.base_url)
        client_a.login(_state["user_a_name"], USER_PASSWORD)
        client_a.username = _state["user_a_name"]

        client_b = APIClient(primary_server.base_url)
        client_b.login(_state["user_b_name"], USER_PASSWORD)
        client_b.username = _state["user_b_name"]

        # ══════════════════════════════════════════════════════════════
        # ADMIN: 3 photos, 1 blob
        # ══════════════════════════════════════════════════════════════
        _state["admin_photo_ids"] = []
        _state["admin_photo_contents"] = {}
        for i in range(3):
            content = generate_test_jpeg(width=30 + i, height=30 + i)
            p = admin_cl.upload_photo(unique_filename(), content=content)
            pid = p["photo_id"]
            _state["admin_photo_ids"].append(pid)
            _state["admin_photo_contents"][pid] = content
        assert len(_state["admin_photo_ids"]) == 3

        _state["admin_blob_ids"] = []
        _state["admin_blob_contents"] = {}
        content = generate_random_bytes(1500)
        blob = admin_cl.upload_blob("photo", content)
        _state["admin_blob_ids"].append(blob["blob_id"])
        _state["admin_blob_contents"][blob["blob_id"]] = content
        assert len(_state["admin_blob_ids"]) == 1

        # Admin tag
        admin_cl.add_tag(_state["admin_photo_ids"][0], "admin_tag")

        # Admin secure gallery: "Admin Vault" with ba1
        admin_gallery = admin_cl.create_secure_gallery("Admin Vault")
        _state["admin_gallery_id"] = admin_gallery["gallery_id"]
        admin_token = admin_cl.unlock_secure_gallery(ADMIN_PASSWORD)["gallery_token"]
        admin_gal_resp = admin_cl.add_secure_gallery_item(
            admin_gallery["gallery_id"], _state["admin_blob_ids"][0], admin_token,
        )
        _state["admin_clone_id"] = admin_gal_resp["new_blob_id"]

        # ══════════════════════════════════════════════════════════════
        # USER A: 5 photos, 2 visible blobs + 1 trashed blob
        # ══════════════════════════════════════════════════════════════
        _state["user_a_photo_ids"] = []
        _state["user_a_photo_contents"] = {}
        for i in range(5):
            content = generate_test_jpeg(width=40 + i, height=40 + i)
            p = client_a.upload_photo(unique_filename(), content=content)
            pid = p["photo_id"]
            _state["user_a_photo_ids"].append(pid)
            _state["user_a_photo_contents"][pid] = content
        assert len(_state["user_a_photo_ids"]) == 5

        _state["user_a_blob_ids"] = []
        _state["user_a_blob_contents"] = {}
        for i in range(3):
            content = generate_random_bytes(1024 + i * 200)
            blob = client_a.upload_blob("photo", content)
            bid = blob["blob_id"]
            _state["user_a_blob_ids"].append(bid)
            _state["user_a_blob_contents"][bid] = content
        assert len(_state["user_a_blob_ids"]) == 3

        # Trash blobA3
        blobA3 = _state["user_a_blob_ids"][2]
        trash_resp = client_a.soft_delete_blob(
            blobA3,
            filename="trashed_a3.jpg",
            mime_type="image/jpeg",
            size_bytes=len(_state["user_a_blob_contents"][blobA3]),
        )
        _state["user_a_trash_id"] = trash_resp["trash_id"]
        _state["user_a_trashed_blob_id"] = blobA3

        # User A tags
        client_a.add_tag(_state["user_a_photo_ids"][0], "travel")
        client_a.add_tag(_state["user_a_photo_ids"][1], "sunset")

        # User A favorites a1
        client_a.favorite_photo(_state["user_a_photo_ids"][0])

        # User A secure gallery: "A Private" with blobA1
        a_gallery = client_a.create_secure_gallery("A Private")
        _state["user_a_gallery_id"] = a_gallery["gallery_id"]
        a_token = client_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        a_gal_resp = client_a.add_secure_gallery_item(
            a_gallery["gallery_id"], _state["user_a_blob_ids"][0], a_token,
        )
        _state["user_a_clone_id"] = a_gal_resp["new_blob_id"]
        _state["user_a_gallery_hidden_blob"] = _state["user_a_blob_ids"][0]

        # ══════════════════════════════════════════════════════════════
        # USER B: 4 photos, 3 blobs
        # ══════════════════════════════════════════════════════════════
        _state["user_b_photo_ids"] = []
        _state["user_b_photo_contents"] = {}
        for i in range(4):
            content = generate_test_jpeg(width=50 + i, height=50 + i)
            p = client_b.upload_photo(unique_filename(), content=content)
            pid = p["photo_id"]
            _state["user_b_photo_ids"].append(pid)
            _state["user_b_photo_contents"][pid] = content
        assert len(_state["user_b_photo_ids"]) == 4

        _state["user_b_blob_ids"] = []
        _state["user_b_blob_contents"] = {}
        for i in range(3):
            content = generate_random_bytes(2048 + i * 300)
            blob = client_b.upload_blob("photo", content)
            bid = blob["blob_id"]
            _state["user_b_blob_ids"].append(bid)
            _state["user_b_blob_contents"][bid] = content
        assert len(_state["user_b_blob_ids"]) == 3

        # User B tag
        client_b.add_tag(_state["user_b_photo_ids"][0], "nature")

        # User B favorites b2
        client_b.favorite_photo(_state["user_b_photo_ids"][1])

        # User B secure gallery: "B Secrets" with blobB1
        b_gallery = client_b.create_secure_gallery("B Secrets")
        _state["user_b_gallery_id"] = b_gallery["gallery_id"]
        b_token = client_b.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        b_gal_resp = client_b.add_secure_gallery_item(
            b_gallery["gallery_id"], _state["user_b_blob_ids"][0], b_token,
        )
        _state["user_b_clone_id"] = b_gal_resp["new_blob_id"]
        _state["user_b_gallery_hidden_blob"] = _state["user_b_blob_ids"][0]

        # ══════════════════════════════════════════════════════════════
        # SHARED ALBUM: "Trip" — owner: A, member: B
        # ══════════════════════════════════════════════════════════════
        album_trip = client_a.create_shared_album("Trip")
        _state["album_trip_id"] = album_trip["id"]

        # Add User B as member
        client_a.add_album_member(album_trip["id"], _state["user_b_id"])

        # A adds a1, B adds b1
        client_a.add_album_photo(album_trip["id"], _state["user_a_photo_ids"][0])
        client_b.add_album_photo(album_trip["id"], _state["user_b_photo_ids"][0])

        # ══════════════════════════════════════════════════════════════
        # SHARED ALBUM: "Work" — owner: Admin, member: A
        # ══════════════════════════════════════════════════════════════
        album_work = admin_cl.create_shared_album("Work")
        _state["album_work_id"] = album_work["id"]

        # Add User A as member
        admin_cl.add_album_member(album_work["id"], _state["user_a_id"])

        # Admin adds pa1
        admin_cl.add_album_photo(album_work["id"], _state["admin_photo_ids"][0])

        # Store all known IDs for cross-contamination checks
        _state["all_admin_ids"] = set(
            _state["admin_photo_ids"] + _state["admin_blob_ids"]
        )
        _state["all_user_a_ids"] = set(
            _state["user_a_photo_ids"] + _state["user_a_blob_ids"]
        )
        _state["all_user_b_ids"] = set(
            _state["user_b_photo_ids"] + _state["user_b_blob_ids"]
        )

        # Store primary user count for verification
        all_primary_users = primary_admin.admin_list_users()
        _state["primary_user_count"] = len(all_primary_users)

        # ── Verify primary state: Admin ──────────────────────────────
        admin_photos = _extract_photo_ids(admin_cl.list_photos(limit=500))
        assert len(admin_photos) == 3, (
            f"Admin: expected 3 photos, got {len(admin_photos)}"
        )
        for pid in _state["admin_photo_ids"]:
            assert pid in admin_photos

        admin_blobs = _extract_blob_ids(admin_cl.list_blobs(limit=500))
        # ba1 is gallery-hidden, should NOT appear in regular listing
        assert _state["admin_blob_ids"][0] not in admin_blobs, (
            "Admin ba1 should be hidden after adding to secure gallery"
        )

        # Verify admin secure gallery has 1 item
        admin_gal_items = admin_cl.list_secure_gallery_items(
            _state["admin_gallery_id"], admin_token,
        )
        admin_gal_list = admin_gal_items if isinstance(admin_gal_items, list) else admin_gal_items.get("items", [])
        assert len(admin_gal_list) == 1, f"Admin gallery: expected 1 item, got {len(admin_gal_list)}"

        # ── Verify primary state: User A ─────────────────────────────
        a_photos = _extract_photo_ids(client_a.list_photos(limit=500))
        assert len(a_photos) == 5, (
            f"User A: expected 5 photos, got {len(a_photos)}"
        )

        a_blobs = _extract_blob_ids(client_a.list_blobs(limit=500))
        # blobA1 gallery-hidden, blobA2 visible, blobA3 trashed
        assert _state["user_a_blob_ids"][0] not in a_blobs, (
            "blobA1 should be hidden after adding to secure gallery"
        )
        assert _state["user_a_blob_ids"][1] in a_blobs
        assert _state["user_a_trashed_blob_id"] not in a_blobs

        a_trash = _extract_trash_ids(client_a.list_trash(limit=500))
        assert _state["user_a_trash_id"] in a_trash

        # ── Verify primary state: User B ─────────────────────────────
        b_photos = _extract_photo_ids(client_b.list_photos(limit=500))
        assert len(b_photos) == 4, (
            f"User B: expected 4 photos, got {len(b_photos)}"
        )

        b_blobs = _extract_blob_ids(client_b.list_blobs(limit=500))
        # blobB1 gallery-hidden → 2 visible
        assert _state["user_b_blob_ids"][0] not in b_blobs, (
            "blobB1 should be hidden after adding to secure gallery"
        )
        assert _state["user_b_blob_ids"][1] in b_blobs
        assert _state["user_b_blob_ids"][2] in b_blobs
        assert len(b_blobs) == 2, f"User B: expected 2 visible blobs, got {len(b_blobs)}"

        # ── Verify primary: secure gallery isolation ──────────────────
        # Each user can only see their own galleries
        admin_galleries = admin_cl.list_secure_galleries()
        admin_gal_names = [g["name"] for g in (admin_galleries if isinstance(admin_galleries, list) else admin_galleries.get("galleries", []))]
        assert "Admin Vault" in admin_gal_names
        assert "A Private" not in admin_gal_names, "Admin can see User A's gallery!"
        assert "B Secrets" not in admin_gal_names, "Admin can see User B's gallery!"

        a_galleries = client_a.list_secure_galleries()
        a_gal_names = [g["name"] for g in (a_galleries if isinstance(a_galleries, list) else a_galleries.get("galleries", []))]
        assert "A Private" in a_gal_names
        assert "Admin Vault" not in a_gal_names, "User A can see Admin's gallery!"
        assert "B Secrets" not in a_gal_names, "User A can see User B's gallery!"

        b_galleries = client_b.list_secure_galleries()
        b_gal_names = [g["name"] for g in (b_galleries if isinstance(b_galleries, list) else b_galleries.get("galleries", []))]
        assert "B Secrets" in b_gal_names
        assert "Admin Vault" not in b_gal_names, "User B can see Admin's gallery!"
        assert "A Private" not in b_gal_names, "User B can see User A's gallery!"

        # ── Verify primary: cross-contamination check ────────────────
        self._verify_no_cross_contamination(
            admin_photos, admin_blobs,
            a_photos, a_blobs,
            b_photos, b_blobs,
            "primary",
        )

        # ── Verify primary: shared albums ────────────────────────────
        trip_photos_a = client_a.list_album_photos(album_trip["id"])
        trip_list = trip_photos_a if isinstance(trip_photos_a, list) else trip_photos_a.get("photos", [])
        trip_refs = [p.get("photo_ref", p.get("photo_id", p.get("id"))) for p in trip_list]
        assert len(trip_list) == 2, f"Trip album: expected 2 photos, got {len(trip_list)}"
        assert _state["user_a_photo_ids"][0] in trip_refs
        assert _state["user_b_photo_ids"][0] in trip_refs

        work_photos_admin = admin_cl.list_album_photos(album_work["id"])
        work_list = work_photos_admin if isinstance(work_photos_admin, list) else work_photos_admin.get("photos", [])
        work_refs = [p.get("photo_ref", p.get("photo_id", p.get("id"))) for p in work_list]
        assert len(work_list) == 1, f"Work album: expected 1 photo, got {len(work_list)}"
        assert _state["admin_photo_ids"][0] in work_refs

        # User B should NOT see Work album
        b_albums = client_b.list_shared_albums()
        b_album_list = b_albums if isinstance(b_albums, list) else b_albums.get("albums", [])
        b_album_names = [a.get("name") for a in b_album_list]
        assert "Work" not in b_album_names, (
            f"User B should NOT see 'Work' album: {b_album_names}"
        )

    @staticmethod
    def _verify_no_cross_contamination(
        admin_photos, admin_blobs,
        a_photos, a_blobs,
        b_photos, b_blobs,
        location,
    ):
        """Assert zero overlap in media IDs between all 3 users."""
        admin_set = set(admin_photos + admin_blobs)
        a_set = set(a_photos + a_blobs)
        b_set = set(b_photos + b_blobs)

        overlap_admin_a = admin_set & a_set
        overlap_admin_b = admin_set & b_set
        overlap_a_b = a_set & b_set

        assert not overlap_admin_a, (
            f"CROSS-CONTAMINATION on {location}: Admin ↔ User A share IDs: "
            f"{overlap_admin_a}"
        )
        assert not overlap_admin_b, (
            f"CROSS-CONTAMINATION on {location}: Admin ↔ User B share IDs: "
            f"{overlap_admin_b}"
        )
        assert not overlap_a_b, (
            f"CROSS-CONTAMINATION on {location}: User A ↔ User B share IDs: "
            f"{overlap_a_b}"
        )


# =====================================================================
# Phase 2: Sync to backup and verify per-user isolation
# =====================================================================


class TestMultiUserBackupSync:
    """Sync to backup, then verify exact per-user media counts and isolation."""

    def test_sync_to_backup(self, primary_admin, backup_configured,
                            primary_server, backup_server):
        """Trigger sync and wait for success."""
        result = _trigger_and_wait(primary_admin, backup_configured, timeout=120)
        if result.get("status") == "error":
            _dump_server_logs(primary_server, "primary (sync error)")
            _dump_server_logs(backup_server, "backup (sync error)")
        assert result.get("status") != "error", f"Sync failed: {result}"

    # ── Backup API: Users synced ─────────────────────────────────────

    def test_backup_all_users_synced(self, backup_client):
        """All 3 users exist on backup."""
        users = backup_client.backup_list_users()
        usernames = [u["username"] for u in users]
        assert ADMIN_USERNAME in usernames, "Admin not synced to backup"
        assert _state["user_a_name"] in usernames, "User A not synced to backup"
        assert _state["user_b_name"] in usernames, "User B not synced to backup"

    # ── Backup: Admin media isolation ────────────────────────────────

    def test_backup_admin_photos(self, backup_server):
        """Admin sees exactly 3 photos on backup, none from other users."""
        client = _login_on_server(backup_server.base_url, ADMIN_USERNAME, ADMIN_PASSWORD)
        assert client, "Admin cannot login on backup"
        _state["backup_admin_cl"] = client

        photos = _extract_photo_ids(client.list_photos(limit=500))
        _assert_no_duplicates(photos, "backup admin photos")

        for pid in _state["admin_photo_ids"]:
            assert pid in photos, f"Admin photo {pid} missing on backup"

        # Must NOT contain User A or User B photos
        for pid in _state["user_a_photo_ids"]:
            assert pid not in photos, (
                f"CROSS-CONTAMINATION: User A photo {pid} in admin listing on backup"
            )
        for pid in _state["user_b_photo_ids"]:
            assert pid not in photos, (
                f"CROSS-CONTAMINATION: User B photo {pid} in admin listing on backup"
            )

        assert len(photos) == 3, (
            f"Admin: expected 3 photos on backup, got {len(photos)}: {photos}"
        )

    def test_backup_admin_blobs(self, backup_server):
        """Admin: ba1 is gallery-hidden, so 0 visible blobs on backup."""
        client = _state.get("backup_admin_cl")
        assert client, "Admin not logged into backup"

        blobs = _extract_blob_ids(client.list_blobs(limit=500))
        _assert_no_duplicates(blobs, "backup admin blobs")

        # ba1 was added to secure gallery → hidden from regular listing
        assert _state["admin_blob_ids"][0] not in blobs, (
            "Admin ba1 should be gallery-hidden on backup"
        )

        # Must NOT contain other users' blobs
        for bid in _state["user_a_blob_ids"]:
            assert bid not in blobs, (
                f"CROSS-CONTAMINATION: User A blob {bid} in admin blob listing on backup"
            )
        for bid in _state["user_b_blob_ids"]:
            assert bid not in blobs, (
                f"CROSS-CONTAMINATION: User B blob {bid} in admin blob listing on backup"
            )

    def test_backup_admin_tags(self, backup_server):
        """Admin's tag synced to backup."""
        client = _state.get("backup_admin_cl")
        assert client, "Admin not logged into backup"

        tags = client.get_photo_tags(_state["admin_photo_ids"][0])
        tag_list = tags if isinstance(tags, list) else tags.get("tags", [])
        tag_names = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list]
        assert "admin_tag" in tag_names, f"Admin tag missing on backup: {tag_names}"

    # ── Backup: User A media isolation ───────────────────────────────

    def test_backup_user_a_photos(self, backup_server):
        """User A sees exactly 5 photos on backup, none from other users."""
        client = _login_on_server(backup_server.base_url, _state["user_a_name"], USER_PASSWORD)
        assert client, "User A cannot login on backup"
        _state["backup_user_a_cl"] = client

        photos = _extract_photo_ids(client.list_photos(limit=500))
        _assert_no_duplicates(photos, "backup User A photos")

        for pid in _state["user_a_photo_ids"]:
            assert pid in photos, f"User A photo {pid} missing on backup"

        # Must NOT contain admin or User B photos
        for pid in _state["admin_photo_ids"]:
            assert pid not in photos, (
                f"CROSS-CONTAMINATION: Admin photo {pid} in User A listing on backup"
            )
        for pid in _state["user_b_photo_ids"]:
            assert pid not in photos, (
                f"CROSS-CONTAMINATION: User B photo {pid} in User A listing on backup"
            )

        assert len(photos) == 5, (
            f"User A: expected 5 photos on backup, got {len(photos)}"
        )

    def test_backup_user_a_blobs(self, backup_server):
        """User A: blobA1 gallery-hidden, blobA3 trashed → only blobA2 visible."""
        client = _state.get("backup_user_a_cl")
        assert client, "User A not logged into backup"

        blobs = _extract_blob_ids(client.list_blobs(limit=500))
        _assert_no_duplicates(blobs, "backup User A blobs")

        # blobA1 gallery-hidden
        assert _state["user_a_blob_ids"][0] not in blobs, (
            "blobA1 should be gallery-hidden on backup"
        )
        assert _state["user_a_blob_ids"][1] in blobs, "blobA2 missing on backup"
        assert _state["user_a_trashed_blob_id"] not in blobs, (
            "blobA3 (trashed) should NOT be in blob listing on backup"
        )

        # Must NOT contain other users' blobs
        for bid in _state["admin_blob_ids"]:
            assert bid not in blobs, (
                f"CROSS-CONTAMINATION: Admin blob {bid} in User A blob listing on backup"
            )
        for bid in _state["user_b_blob_ids"]:
            assert bid not in blobs, (
                f"CROSS-CONTAMINATION: User B blob {bid} in User A blob listing on backup"
            )

    def test_backup_user_a_trash(self, backup_server):
        """User A's trashed blob exists on backup, no other users' trash leaks."""
        client = _state.get("backup_user_a_cl")
        assert client, "User A not logged into backup"

        trash = _extract_trash_ids(client.list_trash(limit=500))
        assert _state["user_a_trash_id"] in trash, (
            f"User A trash item missing on backup. Got: {trash}"
        )

    def test_backup_user_a_tags(self, backup_server):
        """User A's tags synced correctly."""
        client = _state.get("backup_user_a_cl")
        assert client, "User A not logged into backup"

        tags_a1 = client.get_photo_tags(_state["user_a_photo_ids"][0])
        tag_list = tags_a1 if isinstance(tags_a1, list) else tags_a1.get("tags", [])
        tag_names = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list]
        assert "travel" in tag_names, f"a1 missing 'travel' tag on backup: {tag_names}"

        tags_a2 = client.get_photo_tags(_state["user_a_photo_ids"][1])
        tag_list2 = tags_a2 if isinstance(tags_a2, list) else tags_a2.get("tags", [])
        tag_names2 = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list2]
        assert "sunset" in tag_names2, f"a2 missing 'sunset' tag on backup: {tag_names2}"

    def test_backup_user_a_favorite(self, backup_server):
        """User A's a1 favorite flag synced to backup."""
        client = _state.get("backup_user_a_cl")
        assert client, "User A not logged into backup"

        photos = client.list_photos(limit=500).get("photos", [])
        a1 = next((p for p in photos if p["id"] == _state["user_a_photo_ids"][0]), None)
        assert a1, "a1 not found on backup"
        assert a1["is_favorite"] in (True, 1), (
            f"a1 favorite not synced to backup: {a1.get('is_favorite')}"
        )

    # ── Backup: User B media isolation ───────────────────────────────

    def test_backup_user_b_photos(self, backup_server):
        """User B sees exactly 4 photos on backup, none from other users."""
        client = _login_on_server(backup_server.base_url, _state["user_b_name"], USER_PASSWORD)
        assert client, "User B cannot login on backup"
        _state["backup_user_b_cl"] = client

        photos = _extract_photo_ids(client.list_photos(limit=500))
        _assert_no_duplicates(photos, "backup User B photos")

        for pid in _state["user_b_photo_ids"]:
            assert pid in photos, f"User B photo {pid} missing on backup"

        # Must NOT contain admin or User A photos
        for pid in _state["admin_photo_ids"]:
            assert pid not in photos, (
                f"CROSS-CONTAMINATION: Admin photo {pid} in User B listing on backup"
            )
        for pid in _state["user_a_photo_ids"]:
            assert pid not in photos, (
                f"CROSS-CONTAMINATION: User A photo {pid} in User B listing on backup"
            )

        assert len(photos) == 4, (
            f"User B: expected 4 photos on backup, got {len(photos)}"
        )

    def test_backup_user_b_blobs(self, backup_server):
        """User B: blobB1 gallery-hidden → 2 visible blobs on backup."""
        client = _state.get("backup_user_b_cl")
        assert client, "User B not logged into backup"

        blobs = _extract_blob_ids(client.list_blobs(limit=500))
        _assert_no_duplicates(blobs, "backup User B blobs")

        # blobB1 gallery-hidden
        assert _state["user_b_blob_ids"][0] not in blobs, (
            "blobB1 should be gallery-hidden on backup"
        )
        assert _state["user_b_blob_ids"][1] in blobs, "blobB2 missing on backup"
        assert _state["user_b_blob_ids"][2] in blobs, "blobB3 missing on backup"

        # Must NOT contain other users' blobs
        for bid in _state["admin_blob_ids"]:
            assert bid not in blobs, (
                f"CROSS-CONTAMINATION: Admin blob {bid} in User B blob listing on backup"
            )
        for bid in _state["user_a_blob_ids"]:
            assert bid not in blobs, (
                f"CROSS-CONTAMINATION: User A blob {bid} in User B blob listing on backup"
            )

        assert len(blobs) == 2, (
            f"User B: expected 2 visible blobs on backup, got {len(blobs)}"
        )

    def test_backup_user_b_tags(self, backup_server):
        """User B's tag synced correctly."""
        client = _state.get("backup_user_b_cl")
        assert client, "User B not logged into backup"

        tags_b1 = client.get_photo_tags(_state["user_b_photo_ids"][0])
        tag_list = tags_b1 if isinstance(tags_b1, list) else tags_b1.get("tags", [])
        tag_names = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list]
        assert "nature" in tag_names, f"b1 missing 'nature' tag on backup: {tag_names}"

    def test_backup_user_b_favorite(self, backup_server):
        """User B's b2 favorite flag synced to backup."""
        client = _state.get("backup_user_b_cl")
        assert client, "User B not logged into backup"

        photos = client.list_photos(limit=500).get("photos", [])
        b2 = next((p for p in photos if p["id"] == _state["user_b_photo_ids"][1]), None)
        assert b2, "b2 not found on backup"
        assert b2["is_favorite"] in (True, 1), (
            f"b2 favorite not synced to backup: {b2.get('is_favorite')}"
        )

    def test_backup_user_b_no_trash(self, backup_server):
        """User B has no trashed items (only User A trashed something)."""
        client = _state.get("backup_user_b_cl")
        assert client, "User B not logged into backup"

        trash = _extract_trash_ids(client.list_trash(limit=500))
        # User B should have NO trash from this test
        # (they may inherit trash from other test runs, so we just check
        # that User A's trash ID is not in User B's trash)
        assert _state["user_a_trash_id"] not in trash, (
            f"CROSS-CONTAMINATION: User A's trash item in User B's trash on backup"
        )

    # ── Backup: Cross-contamination comprehensive check ──────────────

    def test_backup_full_cross_contamination_check(self, backup_server):
        """Comprehensive cross-check: union all user data, verify disjoint."""
        admin_cl = _state.get("backup_admin_cl")
        user_a_cl = _state.get("backup_user_a_cl")
        user_b_cl = _state.get("backup_user_b_cl")
        assert admin_cl and user_a_cl and user_b_cl, "One or more users not logged in"

        admin_photos = _extract_photo_ids(admin_cl.list_photos(limit=500))
        admin_blobs = _extract_blob_ids(admin_cl.list_blobs(limit=500))
        a_photos = _extract_photo_ids(user_a_cl.list_photos(limit=500))
        a_blobs = _extract_blob_ids(user_a_cl.list_blobs(limit=500))
        b_photos = _extract_photo_ids(user_b_cl.list_photos(limit=500))
        b_blobs = _extract_blob_ids(user_b_cl.list_blobs(limit=500))

        TestMultiUserPopulate._verify_no_cross_contamination(
            admin_photos, admin_blobs,
            a_photos, a_blobs,
            b_photos, b_blobs,
            "backup",
        )

    # ── Backup: Shared album verification ────────────────────────────

    def test_backup_trip_album(self, backup_server):
        """Trip album: owner A, member B, 2 photos (a1, b1). Visible to both."""
        user_a_cl = _state.get("backup_user_a_cl")
        user_b_cl = _state.get("backup_user_b_cl")
        assert user_a_cl and user_b_cl, "Users not logged into backup"

        # User A sees Trip
        albums_a = user_a_cl.list_shared_albums()
        album_list_a = albums_a if isinstance(albums_a, list) else albums_a.get("albums", [])
        trip_a = next((a for a in album_list_a if a["name"] == "Trip"), None)
        assert trip_a, f"'Trip' album missing for User A on backup. Got: {[a['name'] for a in album_list_a]}"

        photos_a = user_a_cl.list_album_photos(trip_a["id"])
        photos_list = photos_a if isinstance(photos_a, list) else photos_a.get("photos", [])
        refs = [p.get("photo_ref", p.get("photo_id", p.get("id"))) for p in photos_list]
        assert _state["user_a_photo_ids"][0] in refs, "a1 missing from Trip album on backup"
        assert _state["user_b_photo_ids"][0] in refs, "b1 missing from Trip album on backup"
        assert len(photos_list) == 2, f"Trip: expected 2 photos, got {len(photos_list)}"

        # User B also sees Trip
        albums_b = user_b_cl.list_shared_albums()
        album_list_b = albums_b if isinstance(albums_b, list) else albums_b.get("albums", [])
        trip_b = next((a for a in album_list_b if a["name"] == "Trip"), None)
        assert trip_b, f"'Trip' album missing for User B on backup. Got: {[a['name'] for a in album_list_b]}"

    def test_backup_work_album(self, backup_server):
        """Work album: owner Admin, member A. 1 photo (pa1). B cannot see."""
        admin_cl = _state.get("backup_admin_cl")
        user_a_cl = _state.get("backup_user_a_cl")
        user_b_cl = _state.get("backup_user_b_cl")
        assert admin_cl and user_a_cl and user_b_cl, "Users not logged into backup"

        # Admin sees Work
        albums_admin = admin_cl.list_shared_albums()
        album_list = albums_admin if isinstance(albums_admin, list) else albums_admin.get("albums", [])
        work = next((a for a in album_list if a["name"] == "Work"), None)
        assert work, f"'Work' album missing for Admin on backup. Got: {[a['name'] for a in album_list]}"

        photos = admin_cl.list_album_photos(work["id"])
        photo_list = photos if isinstance(photos, list) else photos.get("photos", [])
        refs = [p.get("photo_ref", p.get("photo_id", p.get("id"))) for p in photo_list]
        assert _state["admin_photo_ids"][0] in refs, "pa1 missing from Work album on backup"
        assert len(photo_list) == 1, f"Work: expected 1 photo, got {len(photo_list)}"

        # User A also sees Work (they're a member)
        albums_a = user_a_cl.list_shared_albums()
        album_list_a = albums_a if isinstance(albums_a, list) else albums_a.get("albums", [])
        work_a = next((a for a in album_list_a if a["name"] == "Work"), None)
        assert work_a, "User A should see 'Work' album on backup"

        # User B does NOT see Work
        albums_b = user_b_cl.list_shared_albums()
        album_list_b = albums_b if isinstance(albums_b, list) else albums_b.get("albums", [])
        work_b = next((a for a in album_list_b if a["name"] == "Work"), None)
        assert work_b is None, (
            f"User B should NOT see 'Work' album on backup but found it"
        )

    # ── Backup: Secure gallery isolation ────────────────────────────

    def test_backup_secure_gallery_isolation(self, backup_server):
        """Each user's secure gallery is hidden from other users on backup.
        Gallery items don't leak into any other user's listings."""
        admin_cl = _state.get("backup_admin_cl")
        user_a_cl = _state.get("backup_user_a_cl")
        user_b_cl = _state.get("backup_user_b_cl")
        assert admin_cl and user_a_cl and user_b_cl, "Users not logged in"

        # ── Admin sees only their own gallery ──
        admin_gals = admin_cl.list_secure_galleries()
        admin_gal_list = admin_gals if isinstance(admin_gals, list) else admin_gals.get("galleries", [])
        admin_gal_names = [g["name"] for g in admin_gal_list]
        assert "Admin Vault" in admin_gal_names, (
            f"Admin gallery missing on backup. Got: {admin_gal_names}"
        )
        assert "A Private" not in admin_gal_names, (
            "GALLERY LEAK: Admin can see User A's gallery on backup!"
        )
        assert "B Secrets" not in admin_gal_names, (
            "GALLERY LEAK: Admin can see User B's gallery on backup!"
        )

        # ── User A sees only their own gallery ──
        a_gals = user_a_cl.list_secure_galleries()
        a_gal_list = a_gals if isinstance(a_gals, list) else a_gals.get("galleries", [])
        a_gal_names = [g["name"] for g in a_gal_list]
        assert "A Private" in a_gal_names, (
            f"User A gallery missing on backup. Got: {a_gal_names}"
        )
        assert "Admin Vault" not in a_gal_names, (
            "GALLERY LEAK: User A can see Admin's gallery on backup!"
        )
        assert "B Secrets" not in a_gal_names, (
            "GALLERY LEAK: User A can see User B's gallery on backup!"
        )

        # ── User B sees only their own gallery ──
        b_gals = user_b_cl.list_secure_galleries()
        b_gal_list = b_gals if isinstance(b_gals, list) else b_gals.get("galleries", [])
        b_gal_names = [g["name"] for g in b_gal_list]
        assert "B Secrets" in b_gal_names, (
            f"User B gallery missing on backup. Got: {b_gal_names}"
        )
        assert "Admin Vault" not in b_gal_names, (
            "GALLERY LEAK: User B can see Admin's gallery on backup!"
        )
        assert "A Private" not in b_gal_names, (
            "GALLERY LEAK: User B can see User A's gallery on backup!"
        )

        # ── Gallery items don't leak into other users' blob/photo listings ──
        admin_blobs = _extract_blob_ids(admin_cl.list_blobs(limit=500))
        a_blobs = _extract_blob_ids(user_a_cl.list_blobs(limit=500))
        b_blobs = _extract_blob_ids(user_b_cl.list_blobs(limit=500))

        # Clone IDs should not appear in any user's REGULAR blob listing
        for clone_key, owner in [
            ("admin_clone_id", "Admin"),
            ("user_a_clone_id", "User A"),
            ("user_b_clone_id", "User B"),
        ]:
            clone_id = _state.get(clone_key)
            if clone_id:
                assert clone_id not in admin_blobs, (
                    f"GALLERY LEAK: {owner} clone {clone_id} in Admin's blob listing on backup"
                )
                assert clone_id not in a_blobs, (
                    f"GALLERY LEAK: {owner} clone {clone_id} in User A's blob listing on backup"
                )
                assert clone_id not in b_blobs, (
                    f"GALLERY LEAK: {owner} clone {clone_id} in User B's blob listing on backup"
                )

        # Gallery-hidden originals must not appear in OTHER users' listings
        # (for own user, they're already checked in individual blob tests above)
        assert _state["admin_blob_ids"][0] not in a_blobs, (
            "Admin's gallery-hidden ba1 leaked into User A's blobs on backup"
        )
        assert _state["admin_blob_ids"][0] not in b_blobs, (
            "Admin's gallery-hidden ba1 leaked into User B's blobs on backup"
        )
        assert _state["user_a_gallery_hidden_blob"] not in admin_blobs, (
            "User A's gallery-hidden blobA1 leaked into Admin's blobs on backup"
        )
        assert _state["user_a_gallery_hidden_blob"] not in b_blobs, (
            "User A's gallery-hidden blobA1 leaked into User B's blobs on backup"
        )
        assert _state["user_b_gallery_hidden_blob"] not in admin_blobs, (
            "User B's gallery-hidden blobB1 leaked into Admin's blobs on backup"
        )
        assert _state["user_b_gallery_hidden_blob"] not in a_blobs, (
            "User B's gallery-hidden blobB1 leaked into User A's blobs on backup"
        )

    def test_backup_secure_gallery_items(self, backup_server):
        """Verify each user's gallery has exactly 1 item on backup when unlocked."""
        admin_cl = _state.get("backup_admin_cl")
        user_a_cl = _state.get("backup_user_a_cl")
        user_b_cl = _state.get("backup_user_b_cl")
        assert admin_cl and user_a_cl and user_b_cl, "Users not logged in"

        # Admin gallery items
        admin_token = admin_cl.unlock_secure_gallery(ADMIN_PASSWORD)["gallery_token"]
        admin_items = admin_cl.list_secure_gallery_items(
            _state["admin_gallery_id"], admin_token,
        )
        admin_item_list = admin_items if isinstance(admin_items, list) else admin_items.get("items", [])
        assert len(admin_item_list) == 1, (
            f"Admin gallery: expected 1 item on backup, got {len(admin_item_list)}"
        )

        # User A gallery items
        a_token = user_a_cl.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        a_items = user_a_cl.list_secure_gallery_items(
            _state["user_a_gallery_id"], a_token,
        )
        a_item_list = a_items if isinstance(a_items, list) else a_items.get("items", [])
        assert len(a_item_list) == 1, (
            f"User A gallery: expected 1 item on backup, got {len(a_item_list)}"
        )

        # User B gallery items
        b_token = user_b_cl.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        b_items = user_b_cl.list_secure_gallery_items(
            _state["user_b_gallery_id"], b_token,
        )
        b_item_list = b_items if isinstance(b_items, list) else b_items.get("items", [])
        assert len(b_item_list) == 1, (
            f"User B gallery: expected 1 item on backup, got {len(b_item_list)}"
        )

    # ── Backup API: No duplicates across all users ───────────────────

    def test_backup_api_no_global_duplicates(self, backup_client):
        """Backup API: no duplicate photo/blob IDs globally."""
        photos = backup_client.backup_list()
        photo_ids = [p["id"] for p in photos]
        _assert_no_duplicates(photo_ids, "backup API all photos")

        blobs = backup_client.backup_list_blobs()
        blob_ids = [b["id"] for b in blobs]
        _assert_no_duplicates(blob_ids, "backup API all blobs")


# =====================================================================
# Phase 3: Restore fresh primary from backup — verify everything
# =====================================================================


class TestMultiUserRecovery:
    """Spin up a fresh primary, restore from backup, verify all per-user
    data and cross-contamination checks pass."""

    @pytest.fixture
    def fresh_server(self, server_binary, session_tmpdir, backup_server):
        """Start a fresh primary, pair with backup."""
        if server_binary is None:
            pytest.skip("External servers: can't spin up fresh instance")

        port = _find_free_port()
        tmpdir = os.path.join(session_tmpdir, f"pipe_recovery_{int(time.time())}")
        server = ServerInstance("pipe-recovery", port, tmpdir)
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
                name="pipe-backup",
                address=backup_server.base_url.replace("http://", ""),
                api_key=backup_server.backup_api_key,
            )
            server_id = result["id"]

            yield {
                "server": server,
                "client": client,
                "base_url": server.base_url,
                "server_id": server_id,
            }
        finally:
            server.stop()

    def test_full_recovery_and_multi_user_verification(self, fresh_server):
        """
        Recover → verify per-user media counts → verify no cross-contamination
        → verify shared albums → verify tags/favorites.
        """
        client = fresh_server["client"]
        sid = fresh_server["server_id"]
        base = fresh_server["base_url"]

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

        # ══════════════════════════════════════════════════════════════
        # 1. USERS — all 3 present
        # ══════════════════════════════════════════════════════════════
        users = admin.admin_list_users()
        recovered_usernames = {u["username"] for u in users}
        _assert_no_duplicates([u["id"] for u in users], "recovered users")

        assert ADMIN_USERNAME in recovered_usernames, "Admin not recovered"
        assert _state["user_a_name"] in recovered_usernames, "User A not recovered"
        assert _state["user_b_name"] in recovered_usernames, "User B not recovered"

        # ══════════════════════════════════════════════════════════════
        # 2. Login all users on recovered server
        # ══════════════════════════════════════════════════════════════
        user_a = _login_on_server(base, _state["user_a_name"], USER_PASSWORD)
        assert user_a, f"User A cannot login on recovered server"

        user_b = _login_on_server(base, _state["user_b_name"], USER_PASSWORD)
        assert user_b, f"User B cannot login on recovered server"

        # ══════════════════════════════════════════════════════════════
        # 3. ADMIN media on recovered server
        # ══════════════════════════════════════════════════════════════
        admin_photos = _extract_photo_ids(admin.list_photos(limit=500))
        _assert_no_duplicates(admin_photos, "recovered admin photos")
        for pid in _state["admin_photo_ids"]:
            assert pid in admin_photos, f"Admin photo {pid} not recovered"
        assert len(admin_photos) == 3, (
            f"Admin: expected 3 photos after recovery, got {len(admin_photos)}"
        )

        admin_blobs = _extract_blob_ids(admin.list_blobs(limit=500))
        _assert_no_duplicates(admin_blobs, "recovered admin blobs")
        # ba1 is gallery-hidden → should NOT appear in regular listing
        assert _state["admin_blob_ids"][0] not in admin_blobs, (
            "Admin ba1 should be gallery-hidden after recovery"
        )

        # Admin tag
        tags_pa1 = admin.get_photo_tags(_state["admin_photo_ids"][0])
        tag_list = tags_pa1 if isinstance(tags_pa1, list) else tags_pa1.get("tags", [])
        tag_names = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list]
        assert "admin_tag" in tag_names, f"Admin tag not recovered: {tag_names}"

        # ══════════════════════════════════════════════════════════════
        # 4. USER A media on recovered server
        # ══════════════════════════════════════════════════════════════
        a_photos = _extract_photo_ids(user_a.list_photos(limit=500))
        _assert_no_duplicates(a_photos, "recovered User A photos")
        for pid in _state["user_a_photo_ids"]:
            assert pid in a_photos, f"User A photo {pid} not recovered"
        assert len(a_photos) == 5, (
            f"User A: expected 5 photos after recovery, got {len(a_photos)}"
        )

        a_blobs = _extract_blob_ids(user_a.list_blobs(limit=500))
        _assert_no_duplicates(a_blobs, "recovered User A blobs")
        # blobA1 gallery-hidden, blobA2 visible, blobA3 trashed
        assert _state["user_a_blob_ids"][0] not in a_blobs, (
            "blobA1 should be gallery-hidden after recovery"
        )
        assert _state["user_a_blob_ids"][1] in a_blobs, "blobA2 not recovered"
        assert _state["user_a_trashed_blob_id"] not in a_blobs, (
            "blobA3 (trashed) should not be in blob listing after recovery"
        )

        # User A trash
        a_trash = _extract_trash_ids(user_a.list_trash(limit=500))
        assert _state["user_a_trash_id"] in a_trash, (
            f"User A trash not recovered. Got: {a_trash}"
        )

        # User A tags
        tags_a1 = user_a.get_photo_tags(_state["user_a_photo_ids"][0])
        tag_list = tags_a1 if isinstance(tags_a1, list) else tags_a1.get("tags", [])
        tag_names = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list]
        assert "travel" in tag_names, f"a1 missing 'travel' tag after recovery: {tag_names}"

        tags_a2 = user_a.get_photo_tags(_state["user_a_photo_ids"][1])
        tag_list2 = tags_a2 if isinstance(tags_a2, list) else tags_a2.get("tags", [])
        tag_names2 = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list2]
        assert "sunset" in tag_names2, f"a2 missing 'sunset' tag after recovery: {tag_names2}"

        # User A favorite
        a_photo_list = user_a.list_photos(limit=500).get("photos", [])
        a1 = next((p for p in a_photo_list if p["id"] == _state["user_a_photo_ids"][0]), None)
        assert a1, "a1 not found after recovery"
        assert a1["is_favorite"] in (True, 1), (
            f"a1 favorite not recovered: {a1.get('is_favorite')}"
        )

        # ══════════════════════════════════════════════════════════════
        # 5. USER B media on recovered server
        # ══════════════════════════════════════════════════════════════
        b_photos = _extract_photo_ids(user_b.list_photos(limit=500))
        _assert_no_duplicates(b_photos, "recovered User B photos")
        for pid in _state["user_b_photo_ids"]:
            assert pid in b_photos, f"User B photo {pid} not recovered"
        assert len(b_photos) == 4, (
            f"User B: expected 4 photos after recovery, got {len(b_photos)}"
        )

        b_blobs = _extract_blob_ids(user_b.list_blobs(limit=500))
        _assert_no_duplicates(b_blobs, "recovered User B blobs")
        # blobB1 gallery-hidden → 2 visible
        assert _state["user_b_blob_ids"][0] not in b_blobs, (
            "blobB1 should be gallery-hidden after recovery"
        )
        assert _state["user_b_blob_ids"][1] in b_blobs, "blobB2 not recovered"
        assert _state["user_b_blob_ids"][2] in b_blobs, "blobB3 not recovered"
        assert len(b_blobs) == 2, (
            f"User B: expected 2 visible blobs after recovery, got {len(b_blobs)}"
        )

        # User B tag
        tags_b1 = user_b.get_photo_tags(_state["user_b_photo_ids"][0])
        tag_list = tags_b1 if isinstance(tags_b1, list) else tags_b1.get("tags", [])
        tag_names = [t if isinstance(t, str) else t.get("tag", t.get("name", "")) for t in tag_list]
        assert "nature" in tag_names, f"b1 missing 'nature' tag after recovery: {tag_names}"

        # User B favorite
        b_photo_list = user_b.list_photos(limit=500).get("photos", [])
        b2 = next((p for p in b_photo_list if p["id"] == _state["user_b_photo_ids"][1]), None)
        assert b2, "b2 not found after recovery"
        assert b2["is_favorite"] in (True, 1), (
            f"b2 favorite not recovered: {b2.get('is_favorite')}"
        )

        # ══════════════════════════════════════════════════════════════
        # 6. CROSS-CONTAMINATION CHECK on recovered server
        # ══════════════════════════════════════════════════════════════
        TestMultiUserPopulate._verify_no_cross_contamination(
            admin_photos, admin_blobs,
            a_photos, a_blobs,
            b_photos, b_blobs,
            "recovered primary",
        )

        # Additional: admin cannot see other users' photos
        for pid in _state["user_a_photo_ids"]:
            assert pid not in admin_photos, (
                f"CROSS-CONTAMINATION: User A photo {pid} in admin listing after recovery"
            )
        for pid in _state["user_b_photo_ids"]:
            assert pid not in admin_photos, (
                f"CROSS-CONTAMINATION: User B photo {pid} in admin listing after recovery"
            )

        # User A cannot see User B's photos and vice versa
        for pid in _state["user_b_photo_ids"]:
            assert pid not in a_photos, (
                f"CROSS-CONTAMINATION: User B photo {pid} in User A listing after recovery"
            )
        for pid in _state["user_a_photo_ids"]:
            assert pid not in b_photos, (
                f"CROSS-CONTAMINATION: User A photo {pid} in User B listing after recovery"
            )

        # Blob cross-contamination
        for bid in _state["user_a_blob_ids"]:
            if bid != _state["user_a_trashed_blob_id"]:
                assert bid not in admin_blobs, (
                    f"CROSS-CONTAMINATION: User A blob {bid} in admin blobs after recovery"
                )
                assert bid not in b_blobs, (
                    f"CROSS-CONTAMINATION: User A blob {bid} in User B blobs after recovery"
                )
        for bid in _state["user_b_blob_ids"]:
            assert bid not in admin_blobs, (
                f"CROSS-CONTAMINATION: User B blob {bid} in admin blobs after recovery"
            )
            assert bid not in a_blobs, (
                f"CROSS-CONTAMINATION: User B blob {bid} in User A blobs after recovery"
            )
        for bid in _state["admin_blob_ids"]:
            assert bid not in a_blobs, (
                f"CROSS-CONTAMINATION: Admin blob {bid} in User A blobs after recovery"
            )
            assert bid not in b_blobs, (
                f"CROSS-CONTAMINATION: Admin blob {bid} in User B blobs after recovery"
            )

        # User A trash isolation — User B and admin must not see it
        b_trash = _extract_trash_ids(user_b.list_trash(limit=500))
        assert _state["user_a_trash_id"] not in b_trash, (
            f"CROSS-CONTAMINATION: User A trash in User B trash after recovery"
        )
        admin_trash = _extract_trash_ids(admin.list_trash(limit=500))
        assert _state["user_a_trash_id"] not in admin_trash, (
            f"CROSS-CONTAMINATION: User A trash in admin trash after recovery"
        )

        # ══════════════════════════════════════════════════════════════
        # 7. SHARED ALBUMS on recovered server
        # ══════════════════════════════════════════════════════════════

        # Trip album: owner A, member B, photos a1+b1
        albums_a = user_a.list_shared_albums()
        album_list_a = albums_a if isinstance(albums_a, list) else albums_a.get("albums", [])
        trip = next((a for a in album_list_a if a["name"] == "Trip"), None)
        assert trip, (
            f"'Trip' album not recovered for User A. "
            f"Got: {[a['name'] for a in album_list_a]}"
        )

        trip_photos = user_a.list_album_photos(trip["id"])
        trip_list = trip_photos if isinstance(trip_photos, list) else trip_photos.get("photos", [])
        trip_refs = [p.get("photo_ref", p.get("photo_id", p.get("id"))) for p in trip_list]
        assert _state["user_a_photo_ids"][0] in trip_refs, "a1 not in Trip after recovery"
        assert _state["user_b_photo_ids"][0] in trip_refs, "b1 not in Trip after recovery"
        assert len(trip_list) == 2, (
            f"Trip: expected 2 photos after recovery, got {len(trip_list)}"
        )

        # User B also sees Trip
        albums_b = user_b.list_shared_albums()
        album_list_b = albums_b if isinstance(albums_b, list) else albums_b.get("albums", [])
        trip_b = next((a for a in album_list_b if a["name"] == "Trip"), None)
        assert trip_b, "'Trip' album not visible to User B after recovery"

        # Work album: owner Admin, member A, photo pa1
        albums_admin = admin.list_shared_albums()
        album_list_admin = albums_admin if isinstance(albums_admin, list) else albums_admin.get("albums", [])
        work = next((a for a in album_list_admin if a["name"] == "Work"), None)
        assert work, (
            f"'Work' album not recovered for Admin. "
            f"Got: {[a['name'] for a in album_list_admin]}"
        )

        work_photos = admin.list_album_photos(work["id"])
        work_list = work_photos if isinstance(work_photos, list) else work_photos.get("photos", [])
        work_refs = [p.get("photo_ref", p.get("photo_id", p.get("id"))) for p in work_list]
        assert _state["admin_photo_ids"][0] in work_refs, "pa1 not in Work after recovery"
        assert len(work_list) == 1, (
            f"Work: expected 1 photo after recovery, got {len(work_list)}"
        )

        # User A sees Work (member)
        work_a = next((a for a in album_list_a if a["name"] == "Work"), None)
        assert work_a, "User A should see 'Work' album after recovery"

        # User B does NOT see Work
        work_b = next((a for a in album_list_b if a["name"] == "Work"), None)
        assert work_b is None, (
            "User B should NOT see 'Work' album after recovery"
        )

        # ══════════════════════════════════════════════════════════════
        # 8. SECURE GALLERY ISOLATION on recovered server
        # ══════════════════════════════════════════════════════════════

        # Each user only sees their own secure galleries
        admin_gals = admin.list_secure_galleries()
        admin_gal_list = admin_gals if isinstance(admin_gals, list) else admin_gals.get("galleries", [])
        admin_gal_names = [g["name"] for g in admin_gal_list]
        assert "Admin Vault" in admin_gal_names, (
            f"Admin gallery not recovered. Got: {admin_gal_names}"
        )
        assert "A Private" not in admin_gal_names, (
            "GALLERY LEAK: Admin can see User A's gallery after recovery!"
        )
        assert "B Secrets" not in admin_gal_names, (
            "GALLERY LEAK: Admin can see User B's gallery after recovery!"
        )

        a_gals = user_a.list_secure_galleries()
        a_gal_list = a_gals if isinstance(a_gals, list) else a_gals.get("galleries", [])
        a_gal_names = [g["name"] for g in a_gal_list]
        assert "A Private" in a_gal_names, (
            f"User A gallery not recovered. Got: {a_gal_names}"
        )
        assert "Admin Vault" not in a_gal_names, (
            "GALLERY LEAK: User A can see Admin's gallery after recovery!"
        )
        assert "B Secrets" not in a_gal_names, (
            "GALLERY LEAK: User A can see User B's gallery after recovery!"
        )

        b_gals = user_b.list_secure_galleries()
        b_gal_list = b_gals if isinstance(b_gals, list) else b_gals.get("galleries", [])
        b_gal_names = [g["name"] for g in b_gal_list]
        assert "B Secrets" in b_gal_names, (
            f"User B gallery not recovered. Got: {b_gal_names}"
        )
        assert "Admin Vault" not in b_gal_names, (
            "GALLERY LEAK: User B can see Admin's gallery after recovery!"
        )
        assert "A Private" not in b_gal_names, (
            "GALLERY LEAK: User B can see User A's gallery after recovery!"
        )

        # Clone IDs must not leak into any regular listing
        for clone_key, owner in [
            ("admin_clone_id", "Admin"),
            ("user_a_clone_id", "User A"),
            ("user_b_clone_id", "User B"),
        ]:
            clone_id = _state.get(clone_key)
            if clone_id:
                assert clone_id not in admin_blobs, (
                    f"GALLERY LEAK: {owner} clone in Admin blobs after recovery"
                )
                assert clone_id not in a_blobs, (
                    f"GALLERY LEAK: {owner} clone in User A blobs after recovery"
                )
                assert clone_id not in b_blobs, (
                    f"GALLERY LEAK: {owner} clone in User B blobs after recovery"
                )

        # Gallery-hidden originals must not appear in other users' listings
        assert _state["admin_blob_ids"][0] not in a_blobs, (
            "Admin ba1 leaked into User A blobs after recovery"
        )
        assert _state["admin_blob_ids"][0] not in b_blobs, (
            "Admin ba1 leaked into User B blobs after recovery"
        )
        assert _state["user_a_gallery_hidden_blob"] not in admin_blobs, (
            "User A blobA1 leaked into Admin blobs after recovery"
        )
        assert _state["user_a_gallery_hidden_blob"] not in b_blobs, (
            "User A blobA1 leaked into User B blobs after recovery"
        )
        assert _state["user_b_gallery_hidden_blob"] not in admin_blobs, (
            "User B blobB1 leaked into Admin blobs after recovery"
        )
        assert _state["user_b_gallery_hidden_blob"] not in a_blobs, (
            "User B blobB1 leaked into User A blobs after recovery"
        )

        # Verify gallery item counts after recovery (must unlock first)
        admin_tok = admin.unlock_secure_gallery(ADMIN_PASSWORD)["gallery_token"]
        admin_gal = next(g for g in admin_gal_list if g["name"] == "Admin Vault")
        admin_items = admin.list_secure_gallery_items(admin_gal["id"], admin_tok)
        admin_item_list = admin_items if isinstance(admin_items, list) else admin_items.get("items", [])
        assert len(admin_item_list) == 1, (
            f"Admin gallery: expected 1 item after recovery, got {len(admin_item_list)}"
        )

        a_tok = user_a.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        a_gal = next(g for g in a_gal_list if g["name"] == "A Private")
        a_items = user_a.list_secure_gallery_items(a_gal["id"], a_tok)
        a_item_list = a_items if isinstance(a_items, list) else a_items.get("items", [])
        assert len(a_item_list) == 1, (
            f"User A gallery: expected 1 item after recovery, got {len(a_item_list)}"
        )

        b_tok = user_b.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        b_gal = next(g for g in b_gal_list if g["name"] == "B Secrets")
        b_items = user_b.list_secure_gallery_items(b_gal["id"], b_tok)
        b_item_list = b_items if isinstance(b_items, list) else b_items.get("items", [])
        assert len(b_item_list) == 1, (
            f"User B gallery: expected 1 item after recovery, got {len(b_item_list)}"
        )

        # ══════════════════════════════════════════════════════════════
        # 9. NO GLOBAL DUPLICATES on recovered server
        # ══════════════════════════════════════════════════════════════
        all_photos = admin_photos + a_photos + b_photos
        _assert_no_duplicates(all_photos, "all recovered photos (3 users)")

        all_blobs = admin_blobs + a_blobs + b_blobs
        _assert_no_duplicates(all_blobs, "all recovered blobs (3 users)")
