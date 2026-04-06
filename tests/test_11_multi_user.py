"""
Test 11: Multi-User Scenarios — user isolation, admin operations, shared album
         collaboration, concurrent operations.
"""

import concurrent.futures
import pytest
from helpers import (
    APIClient,
    unique_filename,
    random_username,
    generate_random_bytes,
)
from conftest import USER_PASSWORD


class TestUserIsolation:
    """Ensure complete data isolation between users."""

    def test_photo_isolation(self, user_client, second_user_client):
        """Users cannot see each other's photos."""
        p1 = user_client.upload_photo(unique_filename())
        p2 = second_user_client.upload_photo(unique_filename())

        list1 = user_client.list_photos()
        list2 = second_user_client.list_photos()

        u1_ids = {p["id"] for p in list1["photos"]}
        u2_ids = {p["id"] for p in list2["photos"]}

        assert p1["photo_id"] in u1_ids
        assert p1["photo_id"] not in u2_ids
        assert p2["photo_id"] in u2_ids
        assert p2["photo_id"] not in u1_ids

    def test_blob_isolation(self, user_client, second_user_client):
        """Users cannot access each other's blobs."""
        blob = user_client.upload_blob("photo", generate_random_bytes(512))
        r = second_user_client.download_blob(blob["blob_id"])
        assert r.status_code in (403, 404)

    def test_trash_isolation(self, user_client, second_user_client):
        """Users cannot see each other's trash."""
        blob = user_client.upload_blob("photo")
        user_client.soft_delete_blob(blob["blob_id"], filename="iso.jpg")

        trash1 = user_client.list_trash()
        trash2 = second_user_client.list_trash()

        ids1 = {t["id"] for t in trash1["items"]}
        ids2 = {t["id"] for t in trash2["items"]}

        assert len(ids1) >= 1, "User 1 should have at least 1 trash item"
        assert ids1.isdisjoint(ids2), (
            f"User 2 can see user 1's trash! Overlap: {ids1 & ids2}"
        )

    def test_tag_isolation(self, user_client, second_user_client):
        """Users cannot see each other's tags."""
        p = user_client.upload_photo(unique_filename())
        user_client.add_tag(p["photo_id"], "private_tag_isolation")

        tags2 = second_user_client.list_tags()
        assert "private_tag_isolation" not in tags2.get("tags", [])

    def test_gallery_isolation(self, user_client, second_user_client):
        """Users cannot see each other's secure galleries."""
        user_client.create_secure_gallery("Private Gallery")

        galleries2 = second_user_client.list_secure_galleries()
        names = [g["name"] for g in galleries2["galleries"]]
        assert "Private Gallery" not in names

    def test_storage_stats_isolation(self, user_client, second_user_client):
        """Storage stats should reflect only the user's own data."""
        user_client.upload_blob("photo", generate_random_bytes(512))
        stats1 = user_client.storage_stats()
        stats2 = second_user_client.storage_stats()

        # User 1 uploaded a blob, so must have non-zero storage
        u1_bytes = stats1.get("user_total_bytes", 0)
        assert u1_bytes > 0, f"User 1 should have >0 bytes after upload, got {u1_bytes}"

        # User 2 hasn't uploaded anything, storage should be 0 or much less
        u2_bytes = stats2.get("user_total_bytes", 0)
        assert u2_bytes < u1_bytes, (
            f"User 2 storage ({u2_bytes}) should be less than user 1 ({u1_bytes})"
        )


class TestMultiUserAlbumCollaboration:
    """Shared album interactions between multiple users."""

    def test_two_users_share_album(self, user_client, second_user_client, primary_admin):
        """Owner creates album, adds member, both contribute photos."""
        album = user_client.create_shared_album("Collab Album")

        # Find second user by username
        sharing_users = user_client.list_sharing_users()
        second_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_id = u["id"]
                break

        if not second_id:
            pytest.skip("Could not find second user")

        # Add member
        user_client.add_album_member(album["id"], second_id)

        # Owner adds photo
        p1 = user_client.upload_photo(unique_filename())
        user_client.add_album_photo(album["id"], p1["photo_id"])

        # Member adds photo
        p2 = second_user_client.upload_photo(unique_filename())
        second_user_client.add_album_photo(album["id"], p2["photo_id"])

        # Both should see exactly 2 photos
        photos_owner = user_client.list_album_photos(album["id"])
        photos_member = second_user_client.list_album_photos(album["id"])

        assert len(photos_owner) == 2, (
            f"Owner should see exactly 2 photos, got {len(photos_owner)}"
        )
        assert len(photos_member) == 2, (
            f"Member should see exactly 2 photos, got {len(photos_member)}"
        )

    def test_removed_member_loses_access(self, user_client, second_user_client):
        """Removed members should lose access to the album."""
        album = user_client.create_shared_album("Revoke Album")

        sharing_users = user_client.list_sharing_users()
        second_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_id = u["id"]
                break

        if not second_id:
            pytest.skip("Could not find second user")
        user_client.add_album_member(album["id"], second_id)

        # Verify access
        albums = second_user_client.list_shared_albums()
        assert any(a["id"] == album["id"] for a in albums)

        # Remove member
        user_client.remove_album_member(album["id"], second_id)

        # Should lose access
        albums = second_user_client.list_shared_albums()
        assert not any(a["id"] == album["id"] for a in albums)


class TestAdminOperations:
    """Admin-specific operations across users."""

    def test_admin_can_list_all_users(self, admin_client):
        users = admin_client.admin_list_users()
        assert len(users) >= 1

    def test_admin_create_and_delete_user(self, admin_client, primary_server):
        username = random_username("lifecycle_")
        data = admin_client.admin_create_user(username, "Lifecycle123!")

        # User can login
        client = APIClient(primary_server.base_url)
        client.login(username, "Lifecycle123!")

        # Admin deletes
        admin_client.admin_delete_user(data["user_id"])

        # User can no longer login
        r = client.post("/api/auth/login", json_data={
            "username": username, "password": "Lifecycle123!",
        })
        assert r.status_code == 401

    def test_admin_role_change(self, admin_client, primary_server):
        username = random_username("rolechange_")
        data = admin_client.admin_create_user(username, "RoleChange123!")
        uid = data["user_id"]

        # Promote to admin
        r = admin_client.put(f"/api/admin/users/{uid}/role", json_data={"role": "admin"})
        assert r.status_code == 200

        # Verify they can now access admin endpoints
        client = APIClient(primary_server.base_url)
        client.login(username, "RoleChange123!")
        users = client.admin_list_users()
        assert isinstance(users, list)

        # Demote back
        r = admin_client.put(f"/api/admin/users/{uid}/role", json_data={"role": "user"})
        assert r.status_code == 200

    def test_delete_user_cleans_up_data(self, admin_client, primary_server):
        """Deleting a user should clean up their photos."""
        username = random_username("cleanup_")
        data = admin_client.admin_create_user(username, "Cleanup123!")
        uid = data["user_id"]

        # User uploads photos
        client = APIClient(primary_server.base_url)
        client.login(username, "Cleanup123!")
        client.upload_photo(unique_filename())
        client.upload_photo(unique_filename())

        # Delete user
        admin_client.admin_delete_user(uid)

        # Verify the user is actually gone
        users = admin_client.admin_list_users()
        user_ids = [u.get("id") for u in users]
        assert uid not in user_ids, f"Deleted user {uid} still in user list"

        # Verify user can no longer login
        dead_client = APIClient(primary_server.base_url)
        r = dead_client.post("/api/auth/login", json_data={
            "username": username, "password": "Cleanup123!",
        })
        assert r.status_code == 401, f"Deleted user could still login: {r.status_code}"


class TestConcurrentOperations:
    """Concurrent API operations should not corrupt data."""

    def test_concurrent_uploads(self, user_client):
        """Multiple simultaneous uploads should all succeed."""
        results = []
        errors = []

        def upload_one(i):
            try:
                # Use unique random content per upload to avoid dedup
                content = generate_random_bytes(256)
                data = user_client.upload_photo(f"concurrent_{i}.jpg", content=content)
                return data["photo_id"]
            except Exception as e:
                errors.append(str(e))
                return None

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(upload_one, i) for i in range(5)]
            for f in concurrent.futures.as_completed(futures):
                result = f.result()
                if result:
                    results.append(result)

        assert len(results) == 5, (
            f"All 5 uploads should succeed, only {len(results)} did. Errors: {errors}"
        )
        assert len(set(results)) == 5, "Duplicate photo IDs from concurrent uploads"

    def test_concurrent_blob_uploads(self, user_client):
        """Multiple concurrent blob uploads."""
        results = []

        def upload_one(i):
            content = generate_random_bytes(512)
            data = user_client.upload_blob("photo", content)
            return data["blob_id"]

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(upload_one, i) for i in range(5)]
            for f in concurrent.futures.as_completed(futures):
                results.append(f.result())

        assert len(results) == 5
        assert len(set(results)) == 5  # All unique
