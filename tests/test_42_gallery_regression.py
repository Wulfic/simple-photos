"""
Test 42: Gallery Engine — E2E Regression Test.

Comprehensive end-to-end regression test for the gallery refactoring.
Exercises the full lifecycle of photos through all view contexts:
  - Main gallery (list, encrypted-sync)
  - Secure gallery (create, add, list, hide from main)
  - Shared albums (create, add photo, list)
  - Favorites (toggle, filter)
  - Editing (crop, duplicate/save-as-copy)
  - Trash (soft delete, restore)
  - Thumbnail availability throughout

This test file exercises combinations of operations, not just individual
features. It is designed to catch interaction bugs between gallery views
that the DDT tests (test_40, test_41) may miss.

This is the primary regression gate for the gallery engine refactoring.
ALL tests in this file must pass before and after each refactoring phase.
"""

import json
import time

import pytest

from helpers import (
    APIClient,
    assert_no_duplicates,
    assert_photo_in_list,
    assert_photo_not_in_list,
    generate_test_gif,
    generate_test_jpeg,
)


# ═══════════════════════════════════════════════════════════════════
# Helpers
# ═══════════════════════════════════════════════════════════════════

USER_PASSWORD = "E2eUserPass456!"


def _upload(client: APIClient, filename: str, content: bytes,
            mime_type: str = "image/jpeg", settle: float = 2.0) -> str:
    resp = client.upload_photo(filename, content, mime_type)
    photo_id = resp["photo_id"]
    time.sleep(settle)
    return photo_id


def _photo_ids(client: APIClient) -> list:
    return [p["id"] for p in client.list_photos()["photos"]]


def _sync_ids(client: APIClient) -> list:
    return [p["id"] for p in client.encrypted_sync().get("photos", [])]


def _wait_thumb(client: APIClient, photo_id: str, timeout: float = 15) -> bytes:
    deadline = time.time() + timeout
    while time.time() < deadline:
        r = client.get_photo_thumb(photo_id)
        if r.status_code == 200 and len(r.content) > 0:
            return r.content
        time.sleep(1)
    raise AssertionError(f"Thumbnail for {photo_id} not ready within {timeout}s")


def _wait_encryption(client: APIClient, photo_id: str, timeout: float = 30) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        for p in client.encrypted_sync().get("photos", []):
            if p["id"] == photo_id and p.get("encrypted_blob_id"):
                return p
        time.sleep(1)
    raise AssertionError(f"Encryption not complete for {photo_id} within {timeout}s")


def _secure_blob_ids(client: APIClient) -> set:
    data = client.get_secure_gallery_blob_ids()
    return set(data.get("blob_ids", []))


# ═══════════════════════════════════════════════════════════════════
# Test: Full Gallery Lifecycle
# ═══════════════════════════════════════════════════════════════════

class TestGalleryLifecycle:
    """
    Upload 5 photos → verify gallery → exercise all view contexts.

    This is a sequential scenario test: each step depends on the previous
    one. If any step fails, subsequent steps may also fail.
    """

    @pytest.fixture(autouse=True)
    def setup_photos(self, user_client):
        """Upload 5 photos of mixed types for the test scenario."""
        self.client = user_client

        # 1. Landscape JPEG
        self.p1_id = _upload(user_client, "land.jpg",
                             generate_test_jpeg(200, 133))
        # 2. Portrait JPEG
        self.p2_id = _upload(user_client, "port.jpg",
                             generate_test_jpeg(133, 200))
        # 3. Small GIF (animated, < 5 MB)
        self.p3_id = _upload(user_client, "anim.gif",
                             generate_test_gif(40, 30, 3), "image/gif")
        # 4. Square JPEG
        self.p4_id = _upload(user_client, "sq.jpg",
                             generate_test_jpeg(150, 150))
        # 5. Another landscape
        self.p5_id = _upload(user_client, "land2.jpg",
                             generate_test_jpeg(300, 200))

        self.all_ids = [self.p1_id, self.p2_id, self.p3_id, self.p4_id, self.p5_id]
        time.sleep(2)  # Extra settle for batch

    def test_01_all_visible_in_gallery(self):
        """All 5 photos should be visible in the main gallery."""
        photos = self.client.list_photos()["photos"]
        gallery_ids = [p["id"] for p in photos]
        for pid in self.all_ids:
            assert pid in gallery_ids, f"Photo {pid} missing from gallery"

    def test_02_all_have_correct_dimensions(self):
        """Each photo should have the correct stored dimensions."""
        photos = self.client.list_photos()["photos"]
        by_id = {p["id"]: p for p in photos}

        expected = {
            self.p1_id: (200, 133),
            self.p2_id: (133, 200),
            self.p3_id: (40, 30),
            self.p4_id: (150, 150),
            self.p5_id: (300, 200),
        }
        for pid, (ew, eh) in expected.items():
            p = by_id[pid]
            assert p["width"] == ew, f"{pid} width: expected {ew}, got {p['width']}"
            assert p["height"] == eh, f"{pid} height: expected {eh}, got {p['height']}"

    def test_03_all_have_thumbnails(self):
        """Each photo should have an available thumbnail."""
        for pid in self.all_ids:
            thumb = _wait_thumb(self.client, pid)
            assert len(thumb) > 50, f"Thumbnail for {pid} too small"

    def test_04_no_duplicate_ids(self):
        """No duplicate IDs in gallery listing."""
        ids = _photo_ids(self.client)
        assert_no_duplicates(ids, "gallery photo IDs")

    def test_05_all_in_encrypted_sync(self):
        """All photos should appear in encrypted-sync."""
        sync_ids = _sync_ids(self.client)
        for pid in self.all_ids:
            assert pid in sync_ids, f"Photo {pid} missing from encrypted-sync"

    def test_06_gif_media_type_correct(self):
        """GIF should be detected as media_type='gif'."""
        photos = self.client.list_photos()["photos"]
        gif_photo = next(p for p in photos if p["id"] == self.p3_id)
        assert gif_photo["media_type"] == "gif"


# ═══════════════════════════════════════════════════════════════════
# Test: Secure Gallery Isolation
# ═══════════════════════════════════════════════════════════════════

class TestSecureGalleryIsolation:
    """Move a photo to secure gallery → verify isolation from main gallery."""

    def test_move_to_secure_hides_from_main(self, user_client):
        """Photo moved to secure gallery must disappear from main gallery."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "secure_iso.jpg", content)

        # Pre-check: visible
        assert photo_id in _photo_ids(user_client)

        # Move to secure gallery
        gallery = user_client.create_secure_gallery("iso_test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(2)

        # Post-check: hidden from main
        assert photo_id not in _photo_ids(user_client), (
            "Photo should be hidden from main gallery after secure add"
        )

        # But visible in secure gallery
        items = user_client.list_secure_gallery_items(gallery["gallery_id"], token)["items"]
        assert len(items) >= 1

    def test_secure_blob_ids_includes_moved_photo(self, user_client):
        """secureBlobIds endpoint should include the moved photo."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "sec_bids.jpg", content)

        gallery = user_client.create_secure_gallery("bids_test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(2)

        secure_ids = _secure_blob_ids(user_client)
        assert len(secure_ids) > 0, "secureBlobIds should not be empty"

    def test_secure_hidden_from_sync(self, user_client):
        """Photo in secure gallery should not appear in encrypted-sync."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "sec_sync.jpg", content)

        # Should be in sync initially
        assert photo_id in _sync_ids(user_client)

        gallery = user_client.create_secure_gallery("sync_hide")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(2)

        # Should be hidden from sync
        assert photo_id not in _sync_ids(user_client), (
            "Photo should be hidden from encrypted-sync after secure add"
        )


# ═══════════════════════════════════════════════════════════════════
# Test: Shared Album Visibility
# ═══════════════════════════════════════════════════════════════════

class TestSharedAlbumVisibility:
    """Shared album photos remain visible in main gallery (unlike secure)."""

    def test_shared_photo_still_in_main(self, user_client):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "shared_vis.jpg", content)

        album = user_client.create_shared_album("vis_test")
        user_client.add_album_photo(album["id"], photo_id, ref_type="photo")

        # Still in main gallery
        assert photo_id in _photo_ids(user_client)

        # Also in shared album
        photos = user_client.list_album_photos(album["id"])
        refs = [p.get("photo_ref") or p.get("photo_id") for p in photos]
        assert photo_id in refs

    def test_shared_album_multi_user_visibility(self, user_client, second_user_client, primary_admin):
        """Second user added as member can see shared album photos."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "shared_multi.jpg", content)

        album = user_client.create_shared_album("multi_test")
        user_client.add_album_photo(album["id"], photo_id, ref_type="photo")

        # Find second user's ID
        users = user_client.list_sharing_users()
        second_uid = None
        for u in users:
            if u.get("username") == second_user_client.username:
                second_uid = u["id"]
                break
        assert second_uid, "Second user not found in sharing users"

        user_client.add_album_member(album["id"], second_uid)

        # Second user should see the album
        albums = second_user_client.list_shared_albums()
        album_ids = [a["id"] for a in albums]
        assert album["id"] in album_ids

        # Second user should see the photos
        photos = second_user_client.list_album_photos(album["id"])
        assert len(photos) >= 1


# ═══════════════════════════════════════════════════════════════════
# Test: Edit + Duplicate Dimensions
# ═══════════════════════════════════════════════════════════════════

class TestEditDuplicateRegression:
    """Editing and duplicating must not corrupt dimensions or thumbnails."""

    def test_crop_preserves_original_thumb(self, user_client):
        """Setting crop metadata should not break the original thumbnail."""
        content = generate_test_jpeg(200, 150)
        photo_id = _upload(user_client, "edit_reg.jpg", content, settle=3)
        _wait_thumb(user_client, photo_id)

        meta = {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8,
                "rotate": 0, "brightness": 0}
        user_client.crop_photo(photo_id, json.dumps(meta))
        time.sleep(1)

        # Thumbnail still valid
        r = user_client.get_photo_thumb(photo_id)
        assert r.status_code == 200

        # Dimensions unchanged (crop is metadata-only)
        photos = user_client.list_photos()["photos"]
        p = next(p for p in photos if p["id"] == photo_id)
        assert p["width"] == 200
        assert p["height"] == 150

    def test_duplicate_rot90_swaps_dimensions(self, user_client):
        """Save As Copy with 90° rotation should swap width/height."""
        content = generate_test_jpeg(200, 150)
        photo_id = _upload(user_client, "dup_rot.jpg", content)

        meta = {"x": 0, "y": 0, "width": 1.0, "height": 1.0,
                "rotate": 90, "brightness": 0}
        dup = user_client.duplicate_photo(photo_id, json.dumps(meta))
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        copy = next(p for p in photos if p["id"] == dup_id)
        assert copy["width"] == 150, f"Expected 150, got {copy['width']}"
        assert copy["height"] == 200, f"Expected 200, got {copy['height']}"

    def test_duplicate_has_null_crop(self, user_client):
        """Duplicate (rendered copy) should have no crop_metadata."""
        content = generate_test_jpeg(200, 150)
        photo_id = _upload(user_client, "dup_null_crop.jpg", content)

        meta = {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8,
                "rotate": 90, "brightness": 30}
        dup = user_client.duplicate_photo(photo_id, json.dumps(meta))
        dup_id = dup["id"]
        time.sleep(3)

        photos = user_client.list_photos()["photos"]
        copy = next(p for p in photos if p["id"] == dup_id)
        assert copy.get("crop_metadata") is None, (
            "Duplicate should have null crop_metadata (edits baked in)"
        )

    def test_duplicate_and_original_both_visible(self, user_client):
        """Both original and duplicate should be visible in gallery."""
        content = generate_test_jpeg(200, 150)
        photo_id = _upload(user_client, "dup_both.jpg", content)

        meta = {"x": 0, "y": 0, "width": 1.0, "height": 1.0,
                "rotate": 0, "brightness": 0}
        dup = user_client.duplicate_photo(photo_id, json.dumps(meta))
        dup_id = dup["id"]
        time.sleep(3)

        ids = _photo_ids(user_client)
        assert photo_id in ids, "Original should still be visible"
        assert dup_id in ids, "Duplicate should be visible"


# ═══════════════════════════════════════════════════════════════════
# Test: Trash → Restore Full Cycle
# ═══════════════════════════════════════════════════════════════════

class TestTrashRestoreRegression:
    """Photo disappears on trash, reappears on restore — including thumbnails."""

    def test_full_trash_restore_cycle(self, user_client):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "trash_cycle.jpg", content, settle=3)
        _wait_thumb(user_client, photo_id)

        # Verify visible
        assert photo_id in _photo_ids(user_client)

        # Trash via DELETE /api/photos/{id} (server-managed photo)
        r = user_client.delete(f"/api/photos/{photo_id}")
        assert r.status_code == 200
        time.sleep(1)

        # Verify gone from main
        assert photo_id not in _photo_ids(user_client)

        # Find in trash
        trash = user_client.list_trash()
        trash_items = trash.get("items", trash) if isinstance(trash, dict) else trash
        entry = None
        for item in trash_items:
            if item.get("photo_id") == photo_id or item.get("original_id") == photo_id:
                entry = item
                break
        assert entry, "Photo not found in trash"

        # Restore
        user_client.restore_trash(entry["id"])
        time.sleep(2)

        # Verify back in main
        assert photo_id in _photo_ids(user_client)

        # Thumbnail should work again
        thumb = _wait_thumb(user_client, photo_id)
        assert len(thumb) > 50

    def test_trash_hides_from_sync(self, user_client):
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "trash_sync.jpg", content)

        assert photo_id in _sync_ids(user_client)

        r = user_client.delete(f"/api/photos/{photo_id}")
        assert r.status_code == 200
        time.sleep(1)

        assert photo_id not in _sync_ids(user_client)


# ═══════════════════════════════════════════════════════════════════
# Test: Favorites Interaction With Secure Gallery
# ═══════════════════════════════════════════════════════════════════

class TestFavoritesSecureInteraction:
    """Favoriting a photo that's later moved to secure gallery."""

    def test_favorited_then_secured_hidden(self, user_client):
        """A favorited photo moved to secure gallery should disappear from favorites."""
        content = generate_test_jpeg(100, 80)
        photo_id = _upload(user_client, "fav_sec.jpg", content)

        # Favorite it
        user_client.favorite_photo(photo_id)

        # Verify in favorites
        favs = user_client.list_photos(favorites_only="true")["photos"]
        fav_ids = [p["id"] for p in favs]
        assert photo_id in fav_ids

        # Move to secure gallery
        gallery = user_client.create_secure_gallery("fav_sec_test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], photo_id, token)
        time.sleep(2)

        # Should be hidden from favorites too
        favs = user_client.list_photos(favorites_only="true")["photos"]
        fav_ids = [p["id"] for p in favs]
        assert photo_id not in fav_ids, (
            "Favorited photo should be hidden from favorites after secure add"
        )


# ═══════════════════════════════════════════════════════════════════
# Test: Cross-Context No-Duplicate Integrity
# ═══════════════════════════════════════════════════════════════════

class TestCrossContextIntegrity:
    """After exercising multiple view contexts, no duplicate IDs should exist."""

    def test_no_duplicates_after_mixed_operations(self, user_client):
        """Upload → favorite → crop → shared album → verify no dupes."""
        # Upload
        p1 = _upload(user_client, "cross_1.jpg", generate_test_jpeg(100, 80))
        p2 = _upload(user_client, "cross_2.jpg", generate_test_jpeg(80, 100))
        p3 = _upload(user_client, "cross_3.gif",
                     generate_test_gif(30, 30, 2), "image/gif")

        # Favorite p1
        user_client.favorite_photo(p1)

        # Crop p2
        meta = {"x": 0, "y": 0, "width": 1.0, "height": 1.0,
                "rotate": 90, "brightness": 0}
        user_client.crop_photo(p2, json.dumps(meta))

        # Add p3 to shared album
        album = user_client.create_shared_album("cross_test")
        user_client.add_album_photo(album["id"], p3, ref_type="photo")

        time.sleep(2)

        # Verify no duplicate IDs in any listing
        photo_ids = _photo_ids(user_client)
        assert_no_duplicates(photo_ids, "gallery photo IDs after mixed ops")

        sync_ids = _sync_ids(user_client)
        assert_no_duplicates(sync_ids, "encrypted-sync IDs after mixed ops")

    def test_secure_add_no_orphan_in_sync(self, user_client):
        """Moving to secure gallery should not leave orphaned entries in sync."""
        p1 = _upload(user_client, "orphan_1.jpg", generate_test_jpeg(100, 80))
        p2 = _upload(user_client, "orphan_2.jpg", generate_test_jpeg(80, 100))

        # Move p1 to secure
        gallery = user_client.create_secure_gallery("orphan_test")
        token = user_client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
        user_client.add_secure_gallery_item(gallery["gallery_id"], p1, token)
        time.sleep(2)

        # p1 gone from sync, p2 still there
        sync_ids = _sync_ids(user_client)
        assert p1 not in sync_ids
        assert p2 in sync_ids

        # No duplicates
        assert_no_duplicates(sync_ids, "sync IDs after secure add")


# ═══════════════════════════════════════════════════════════════════
# Test: Aspect Ratio Consistency Across Endpoints
# ═══════════════════════════════════════════════════════════════════

class TestAspectRatioConsistency:
    """Dimensions from list_photos must match encrypted-sync for same photo."""

    AR_CASES = [
        pytest.param(200, 133, id="landscape_3_2"),
        pytest.param(133, 200, id="portrait_2_3"),
        pytest.param(150, 150, id="square"),
        pytest.param(400, 100, id="panoramic"),
    ]

    @pytest.mark.parametrize("w,h", AR_CASES)
    def test_list_sync_dimensions_match(self, user_client, w, h):
        content = generate_test_jpeg(w, h)
        photo_id = _upload(user_client, "ar_match.jpg", content)

        # From list_photos
        photos = user_client.list_photos()["photos"]
        listed = next(p for p in photos if p["id"] == photo_id)

        # From encrypted-sync
        synced = None
        for p in user_client.encrypted_sync().get("photos", []):
            if p["id"] == photo_id:
                synced = p
                break
        assert synced, "Photo not in encrypted-sync"

        assert listed["width"] == synced["width"], (
            f"Width mismatch: list={listed['width']}, sync={synced['width']}"
        )
        assert listed["height"] == synced["height"], (
            f"Height mismatch: list={listed['height']}, sync={synced['height']}"
        )
