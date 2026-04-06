"""
Test 07: Tags & Search — add tags, remove tags, list tags, search by tag.
"""

import pytest
from helpers import APIClient, unique_filename


class TestTagCRUD:
    """Tag creation, listing, and deletion."""

    def test_add_tag(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        r = user_client.add_tag(photo["photo_id"], "vacation")
        assert r.status_code == 201

    def test_add_multiple_tags(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.add_tag(photo["photo_id"], "nature")
        user_client.add_tag(photo["photo_id"], "sunset")
        user_client.add_tag(photo["photo_id"], "landscape")

        data = user_client.get_photo_tags(photo["photo_id"])
        assert "tags" in data
        assert set(data["tags"]) >= {"nature", "sunset", "landscape"}

    def test_remove_tag(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.add_tag(photo["photo_id"], "removeme")

        r = user_client.remove_tag(photo["photo_id"], "removeme")
        assert r.status_code == 204

        data = user_client.get_photo_tags(photo["photo_id"])
        assert "removeme" not in data["tags"]

    def test_list_all_tags(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.add_tag(photo["photo_id"], "unique_tag_test")

        data = user_client.list_tags()
        assert "tags" in data
        assert "unique_tag_test" in data["tags"]

    def test_add_duplicate_tag(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.add_tag(photo["photo_id"], "duptag")
        r = user_client.add_tag(photo["photo_id"], "duptag")
        # Should either succeed silently or return conflict
        assert r.status_code in (200, 201, 409)

    def test_tag_user_isolation(self, user_client, second_user_client):
        """Tags should be per-user."""
        p1 = user_client.upload_photo(unique_filename())
        user_client.add_tag(p1["photo_id"], "user1only")

        tags2 = second_user_client.list_tags()
        assert "user1only" not in tags2.get("tags", [])


class TestSearch:
    """Search photos by tag/metadata."""

    def test_search_by_tag(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.add_tag(photo["photo_id"], "searchable_beach")

        data = user_client.search("searchable_beach")
        assert "results" in data
        ids = [r["id"] for r in data["results"]]
        assert photo["photo_id"] in ids

    def test_search_no_results(self, user_client):
        data = user_client.search("nonexistent_tag_xyz_12345")
        assert "results" in data
        assert len(data["results"]) == 0

    def test_search_limit(self, user_client):
        # Upload and tag several photos
        for i in range(3):
            p = user_client.upload_photo(unique_filename())
            user_client.add_tag(p["photo_id"], "batch_search")

        data = user_client.search("batch_search", limit=1)
        assert len(data["results"]) <= 1

    def test_search_user_isolation(self, user_client, second_user_client):
        """Search should only return the current user's photos."""
        p1 = user_client.upload_photo(unique_filename())
        user_client.add_tag(p1["photo_id"], "isolation_search")

        results = second_user_client.search("isolation_search")
        ids = [r["id"] for r in results["results"]]
        assert p1["photo_id"] not in ids
