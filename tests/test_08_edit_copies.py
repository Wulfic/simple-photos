"""
Test 08: Edit Copies — create, list, delete edit copies (lightweight metadata versions).
"""

import json
import pytest
from helpers import APIClient, unique_filename


class TestEditCopyCRUD:
    """Edit copy creation, listing, and deletion."""

    def test_create_edit_copy(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        edit_meta = json.dumps({"brightness": 1.2, "contrast": 0.9})
        data = user_client.create_edit_copy(photo["photo_id"], name="Bright version", edit_metadata=edit_meta)
        assert "id" in data
        # Server returns edit_metadata as a parsed JSON object
        expected = json.loads(edit_meta)
        actual = data["edit_metadata"]
        if isinstance(actual, str):
            actual = json.loads(actual)
        assert actual == expected

    def test_create_edit_copy_without_name(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        edit_meta = json.dumps({"filter": "sepia"})
        data = user_client.create_edit_copy(photo["photo_id"], edit_metadata=edit_meta)
        assert "id" in data

    def test_list_edit_copies(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        user_client.create_edit_copy(photo["photo_id"], name="V1", edit_metadata='{"v":1}')
        user_client.create_edit_copy(photo["photo_id"], name="V2", edit_metadata='{"v":2}')

        data = user_client.list_edit_copies(photo["photo_id"])
        assert "copies" in data
        assert len(data["copies"]) >= 2
        names = [c.get("name") for c in data["copies"]]
        assert "V1" in names
        assert "V2" in names

    def test_delete_edit_copy(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        copy = user_client.create_edit_copy(photo["photo_id"], edit_metadata='{"x":1}')
        copy_id = copy["id"]

        r = user_client.delete_edit_copy(photo["photo_id"], copy_id)
        assert r.status_code == 200

        data = user_client.list_edit_copies(photo["photo_id"])
        ids = [c["id"] for c in data["copies"]]
        assert copy_id not in ids


class TestEditCopyEdgeCases:
    """Edge cases for edit copies."""

    def test_edit_copy_on_nonexistent_photo(self, user_client):
        """Creating an edit copy on a nonexistent photo should fail."""
        r = user_client.post(
            "/api/photos/nonexistent-id/copies",
            json_data={"edit_metadata": "{}"},
        )
        assert r.status_code == 404

    def test_edit_copy_user_isolation(self, user_client, second_user_client):
        """Users should not access other users' edit copies."""
        photo = user_client.upload_photo(unique_filename())
        user_client.create_edit_copy(photo["photo_id"], edit_metadata='{"x":1}')

        r = second_user_client.get(f"/api/photos/{photo['photo_id']}/copies")
        # Server returns 200 with empty list (copies are filtered by user_id)
        if r.status_code == 200:
            data = r.json()
            assert len(data.get("copies", [])) == 0
        else:
            assert r.status_code in (403, 404)

    def test_edit_copy_preserved_after_favorite(self, user_client):
        """Edit copies should survive metadata changes on the original."""
        photo = user_client.upload_photo(unique_filename())
        user_client.create_edit_copy(photo["photo_id"], name="Before Fav", edit_metadata='{}')

        user_client.favorite_photo(photo["photo_id"])

        copies = user_client.list_edit_copies(photo["photo_id"])
        assert len(copies["copies"]) >= 1
        assert any(c["name"] == "Before Fav" for c in copies["copies"])

    def test_multiple_edit_copies_per_photo(self, user_client):
        photo = user_client.upload_photo(unique_filename())
        for i in range(5):
            user_client.create_edit_copy(
                photo["photo_id"],
                name=f"Copy {i}",
                edit_metadata=json.dumps({"version": i}),
            )

        copies = user_client.list_edit_copies(photo["photo_id"])
        assert len(copies["copies"]) >= 5

    def test_edit_copy_complex_metadata(self, user_client):
        """Edit metadata can contain complex JSON."""
        photo = user_client.upload_photo(unique_filename())
        meta = json.dumps({
            "adjustments": {
                "brightness": 1.5,
                "contrast": 0.8,
                "saturation": 1.2,
                "temperature": 5500,
            },
            "crop": {"x": 10, "y": 20, "width": 300, "height": 400},
            "filters": ["sharpen", "vignette"],
            "rotation": 90,
        })
        data = user_client.create_edit_copy(photo["photo_id"], name="Complex", edit_metadata=meta)
        actual = data["edit_metadata"]
        if isinstance(actual, str):
            actual = json.loads(actual)
        assert actual == json.loads(meta)
