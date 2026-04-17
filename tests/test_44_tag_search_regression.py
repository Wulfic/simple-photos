"""
Test 44: Tag Search E2E Regression Tests.

End-to-end tests that verify the complete tag + search lifecycle:
  - Upload → Tag → Search → Verify found → Remove tag → Verify gone
  - Multi-user isolation: user A's tags invisible to user B's search
  - Bulk tagging: many photos with the same tag, search returns all
  - Tag + filename combined search: both fields contribute
  - Re-tag after delete: add tag, remove, re-add, search still works
  - Tag list endpoint consistency after CRUD operations
  - Search result metadata: tags array present and correct

These tests catch regressions from search handler changes, tag storage
modifications, and client ↔ server sync issues.
"""

import pytest

from helpers import APIClient, unique_filename


# ══════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════

def _upload(client: APIClient) -> str:
    data = client.upload_photo(unique_filename())
    return data["photo_id"]


# ══════════════════════════════════════════════════════════════════════
# Regression: Full Tag Lifecycle
# ══════════════════════════════════════════════════════════════════════

class TestTagLifecycle:
    """Full lifecycle: add → search → remove → search."""

    def test_add_search_remove_search(self, user_client):
        """End-to-end: add tag, find via search, remove tag, not found."""
        photo_id = _upload(user_client)
        tag = "lifecycle_regression"

        # Add tag
        r = user_client.add_tag(photo_id, tag)
        assert r.status_code == 201

        # Search finds it
        results = user_client.search(tag)
        ids = [r["id"] for r in results["results"]]
        assert photo_id in ids

        # Verify tag on photo
        photo_tags = user_client.get_photo_tags(photo_id)
        assert tag in photo_tags["tags"]

        # Remove tag
        r = user_client.remove_tag(photo_id, tag)
        assert r.status_code == 204

        # Search does NOT find it
        results = user_client.search(tag)
        ids = [r["id"] for r in results["results"]]
        assert photo_id not in ids

        # Tag removed from photo
        photo_tags = user_client.get_photo_tags(photo_id)
        assert tag not in photo_tags["tags"]

    def test_retag_after_removal(self, user_client):
        """Remove a tag and re-add it — photo should be searchable again."""
        photo_id = _upload(user_client)
        tag = "retag_regression"

        user_client.add_tag(photo_id, tag)
        user_client.remove_tag(photo_id, tag)

        # Re-add
        user_client.add_tag(photo_id, tag)
        results = user_client.search(tag)
        ids = [r["id"] for r in results["results"]]
        assert photo_id in ids


# ══════════════════════════════════════════════════════════════════════
# Regression: Multi-User Isolation
# ══════════════════════════════════════════════════════════════════════

class TestUserIsolation:
    """Tags and search results are per-user."""

    def test_tag_invisible_to_other_user_search(self, user_client, second_user_client):
        """User A tags a photo; user B searching same tag finds nothing."""
        photo_id = _upload(user_client)
        tag = "isolation_regression"

        user_client.add_tag(photo_id, tag)

        # User B search
        results = second_user_client.search(tag)
        ids = [r["id"] for r in results["results"]]
        assert photo_id not in ids

    def test_tag_list_isolation(self, user_client, second_user_client):
        """User A's tags do not appear in user B's tag list."""
        photo_id = _upload(user_client)
        tag = "isolated_list_tag"

        user_client.add_tag(photo_id, tag)

        a_tags = user_client.list_tags()["tags"]
        b_tags = second_user_client.list_tags()["tags"]
        assert tag in a_tags
        assert tag not in b_tags


# ══════════════════════════════════════════════════════════════════════
# Regression: Bulk Tag Search
# ══════════════════════════════════════════════════════════════════════

class TestBulkTagging:
    """Multiple photos tagged the same — all found by search."""

    def test_bulk_same_tag(self, user_client):
        """5 photos with the same tag → search returns all 5."""
        tag = "bulk_regression"
        photo_ids = []
        for _ in range(5):
            pid = _upload(user_client)
            user_client.add_tag(pid, tag)
            photo_ids.append(pid)

        results = user_client.search(tag)
        found_ids = [r["id"] for r in results["results"]]
        for pid in photo_ids:
            assert pid in found_ids, f"Photo {pid} not found in bulk search"

    def test_one_removed_others_remain(self, user_client):
        """Remove tag from one photo; others still searchable."""
        tag = "partial_remove_regression"
        pids = []
        for _ in range(3):
            pid = _upload(user_client)
            user_client.add_tag(pid, tag)
            pids.append(pid)

        # Remove from first
        user_client.remove_tag(pids[0], tag)

        results = user_client.search(tag)
        found_ids = [r["id"] for r in results["results"]]
        assert pids[0] not in found_ids
        assert pids[1] in found_ids
        assert pids[2] in found_ids


# ══════════════════════════════════════════════════════════════════════
# Regression: Search Result Metadata
# ══════════════════════════════════════════════════════════════════════

class TestSearchMetadata:
    """Search results carry correct metadata."""

    def test_results_contain_tags_array(self, user_client):
        """Each search result must include a 'tags' array field."""
        photo_id = _upload(user_client)
        user_client.add_tag(photo_id, "meta_tag_a")
        user_client.add_tag(photo_id, "meta_tag_b")

        results = user_client.search("meta_tag_a")
        matching = [r for r in results["results"] if r["id"] == photo_id]
        assert len(matching) == 1
        r = matching[0]
        assert "tags" in r
        assert isinstance(r["tags"], list)
        assert set(r["tags"]) >= {"meta_tag_a", "meta_tag_b"}

    def test_results_contain_standard_fields(self, user_client):
        """Search results contain id, filename, media_type, etc."""
        photo_id = _upload(user_client)
        user_client.add_tag(photo_id, "fields_check_regression")

        results = user_client.search("fields_check_regression")
        matching = [r for r in results["results"] if r["id"] == photo_id]
        assert len(matching) == 1
        r = matching[0]
        for field in ("id", "filename", "media_type", "mime_type", "created_at"):
            assert field in r, f"Missing field '{field}' in search result"


# ══════════════════════════════════════════════════════════════════════
# Regression: Tag List Endpoint
# ══════════════════════════════════════════════════════════════════════

class TestTagListEndpoint:
    """Global tag list reflects CRUD operations."""

    def test_new_tag_appears_in_list(self, user_client):
        """After adding a unique tag, it appears in list_tags."""
        photo_id = _upload(user_client)
        tag = "list_new_regression"
        user_client.add_tag(photo_id, tag)

        all_tags = user_client.list_tags()["tags"]
        assert tag in all_tags

    def test_removed_tag_gone_from_list(self, user_client):
        """After removing the only instance of a tag, it should vanish from list_tags."""
        photo_id = _upload(user_client)
        tag = "list_gone_regression"
        user_client.add_tag(photo_id, tag)
        user_client.remove_tag(photo_id, tag)

        all_tags = user_client.list_tags()["tags"]
        assert tag not in all_tags

    def test_tag_survives_on_second_photo(self, user_client):
        """Tag on two photos, remove from one — tag still in global list."""
        p1 = _upload(user_client)
        p2 = _upload(user_client)
        tag = "survive_regression"
        user_client.add_tag(p1, tag)
        user_client.add_tag(p2, tag)
        user_client.remove_tag(p1, tag)

        all_tags = user_client.list_tags()["tags"]
        assert tag in all_tags


# ══════════════════════════════════════════════════════════════════════
# Regression: Search by Filename Still Works
# ══════════════════════════════════════════════════════════════════════

class TestFilenamSearch:
    """Ensure filename-based search still works alongside tag search."""

    def test_search_by_filename(self, user_client):
        """Upload with known filename, search by filename, photo found."""
        fname = unique_filename()
        data = user_client.upload_photo(fname)
        photo_id = data["photo_id"]

        # Search by the unique part of the filename
        stem = fname.rsplit(".", 1)[0]
        results = user_client.search(stem)
        ids = [r["id"] for r in results["results"]]
        assert photo_id in ids
