"""
Test 05: Shared Albums — CRUD, membership, photo management, permissions.
"""

import pytest
from helpers import APIClient, unique_filename, generate_random_bytes


class TestSharedAlbumCRUD:
    """Create, list, and delete shared albums."""

    def test_create_album(self, user_client):
        data = user_client.create_shared_album("My Album")
        assert "id" in data
        assert data["name"] == "My Album"

    def test_list_albums(self, user_client):
        user_client.create_shared_album("List Test Album")
        albums = user_client.list_shared_albums()
        assert isinstance(albums, list)
        assert any(a["name"] == "List Test Album" for a in albums)

    def test_delete_album(self, user_client):
        album = user_client.create_shared_album("To Delete")
        r = user_client.delete_shared_album(album["id"])
        assert r.status_code == 204

        albums = user_client.list_shared_albums()
        assert not any(a["id"] == album["id"] for a in albums)

    def test_create_multiple_albums(self, user_client):
        a1 = user_client.create_shared_album("Album One")
        a2 = user_client.create_shared_album("Album Two")
        assert a1["id"] != a2["id"]

        albums = user_client.list_shared_albums()
        names = [a["name"] for a in albums]
        assert "Album One" in names
        assert "Album Two" in names


class TestSharedAlbumMembers:
    """Album membership management."""

    def test_add_member(self, user_client, second_user_client, primary_admin):
        album = user_client.create_shared_album("Shared Album")

        # Find second user by username in sharing users list
        sharing_users = user_client.list_sharing_users()
        second_user_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_user_id = u["id"]
                break

        if not second_user_id:
            pytest.skip("Could not find second user")

        data = user_client.add_album_member(album["id"], second_user_id)
        assert "member_id" in data

    def test_list_members(self, user_client, second_user_client, primary_admin):
        album = user_client.create_shared_album("Member List Album")

        sharing_users = user_client.list_sharing_users()
        second_user_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_user_id = u["id"]
                break

        if second_user_id:
            user_client.add_album_member(album["id"], second_user_id)

        members = user_client.list_album_members(album["id"])
        assert isinstance(members, list)
        assert len(members) >= 1

    def test_remove_member(self, user_client, second_user_client, primary_admin):
        album = user_client.create_shared_album("Remove Member Album")

        sharing_users = user_client.list_sharing_users()
        second_user_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_user_id = u["id"]
                break

        if second_user_id:
            user_client.add_album_member(album["id"], second_user_id)
            r = user_client.remove_album_member(album["id"], second_user_id)
            assert r.status_code == 204

    def test_non_owner_cannot_add_member(self, user_client, second_user_client):
        album = user_client.create_shared_album("Owner Only Album")

        # Try to add members as non-owner
        sharing_users = second_user_client.list_sharing_users()
        if sharing_users:
            r = second_user_client.post(
                f"/api/sharing/albums/{album['id']}/members",
                json_data={"user_id": sharing_users[0]["id"]},
            )
            assert r.status_code in (403, 404)

    def test_non_owner_cannot_delete_album(self, user_client, second_user_client):
        album = user_client.create_shared_album("Undeletable Album")
        r = second_user_client.delete_shared_album(album["id"])
        assert r.status_code in (403, 404)


class TestSharedAlbumPhotos:
    """Adding and removing photos from shared albums."""

    def test_add_photo_to_album(self, user_client):
        album = user_client.create_shared_album("Photo Album")
        photo = user_client.upload_photo(unique_filename())

        data = user_client.add_album_photo(album["id"], photo["photo_id"])
        assert "photo_id" in data

    def test_list_album_photos(self, user_client):
        album = user_client.create_shared_album("List Photos Album")
        # Use different content to avoid dedup giving the same photo_id
        p1 = user_client.upload_photo(unique_filename(), content=generate_random_bytes(256))
        p2 = user_client.upload_photo(unique_filename(), content=generate_random_bytes(256))

        user_client.add_album_photo(album["id"], p1["photo_id"])
        user_client.add_album_photo(album["id"], p2["photo_id"])

        photos = user_client.list_album_photos(album["id"])
        assert isinstance(photos, list)
        assert len(photos) >= 2

    def test_remove_photo_from_album(self, user_client):
        album = user_client.create_shared_album("Remove Photo Album")
        photo = user_client.upload_photo(unique_filename())

        added = user_client.add_album_photo(album["id"], photo["photo_id"])
        photo_entry_id = added["photo_id"]

        r = user_client.remove_album_photo(album["id"], photo_entry_id)
        assert r.status_code == 204

        photos = user_client.list_album_photos(album["id"])
        refs = [p["photo_ref"] for p in photos]
        assert photo["photo_id"] not in refs

    def test_add_duplicate_photo_is_idempotent(self, user_client):
        """Server uses INSERT OR IGNORE — duplicates are silently accepted."""
        album = user_client.create_shared_album("No Dup Album")
        photo = user_client.upload_photo(unique_filename())

        user_client.add_album_photo(album["id"], photo["photo_id"])
        r = user_client.post(
            f"/api/sharing/albums/{album['id']}/photos",
            json_data={"photo_ref": photo["photo_id"], "ref_type": "photo"},
        )
        assert r.status_code == 201

        # Should still only have one entry
        photos = user_client.list_album_photos(album["id"])
        refs = [p["photo_ref"] for p in photos]
        assert refs.count(photo["photo_id"]) == 1

    def test_member_can_add_photos(self, user_client, second_user_client, primary_admin):
        """Album members can add their own photos."""
        album = user_client.create_shared_album("Collaborative Album")

        # Find second user's ID from the sharing user list
        sharing_users = user_client.list_sharing_users()
        second_user_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_user_id = u["id"]
                break

        if not second_user_id:
            pytest.skip("Could not find second user in sharing users list")

        user_client.add_album_member(album["id"], second_user_id)

        # Second user uploads and adds a photo
        photo = second_user_client.upload_photo(unique_filename())
        data = second_user_client.add_album_photo(album["id"], photo["photo_id"])
        assert "photo_id" in data

    def test_member_can_view_album(self, user_client, second_user_client, primary_admin):
        """Album members can list photos."""
        album = user_client.create_shared_album("Viewable Album")
        photo = user_client.upload_photo(unique_filename())
        user_client.add_album_photo(album["id"], photo["photo_id"])

        # Find second user's ID
        sharing_users = user_client.list_sharing_users()
        second_user_id = None
        for u in sharing_users:
            if u["username"] == second_user_client.username:
                second_user_id = u["id"]
                break

        if not second_user_id:
            pytest.skip("Could not find second user")

        user_client.add_album_member(album["id"], second_user_id)

        # Member should see the album in their list
        albums = second_user_client.list_shared_albums()
        album_ids = [a["id"] for a in albums]
        assert album["id"] in album_ids


class TestSharedAlbumEdgeCases:
    """Edge cases for shared albums."""

    def test_delete_album_with_photos(self, user_client):
        """Deleting an album should not delete the photos themselves."""
        album = user_client.create_shared_album("Delete With Photos")
        photo = user_client.upload_photo(unique_filename())
        user_client.add_album_photo(album["id"], photo["photo_id"])

        user_client.delete_shared_album(album["id"])

        # Photo should still exist
        photos = user_client.list_photos()
        ids = [p["id"] for p in photos["photos"]]
        assert photo["photo_id"] in ids

    def test_remove_photo_from_album(self, user_client):
        """Removing a photo from the album should work cleanly."""
        album = user_client.create_shared_album("Photo Remove Album")
        photo = user_client.upload_photo(unique_filename())
        added = user_client.add_album_photo(album["id"], photo["photo_id"])

        # Remove uses the album-internal photo entry ID, not the photo_ref
        entry_id = added["photo_id"]
        r = user_client.remove_album_photo(album["id"], entry_id)
        assert r.status_code in (200, 204)

        # Album photo list should no longer include the photo
        photos = user_client.list_album_photos(album["id"])
        ids = [p["id"] for p in photos]
        assert entry_id not in ids
