"""
Test 94: Multi-album secure membership (Z1).

A photo used to be allowed in **at most one** secure album — enforced server-side
in ``add_gallery_item`` with a 409, and the reason the cross-album ``/move``
endpoint exists at all.  Z1 relaxes that: a photo may now live in several secure
albums, sharing ONE clone blob.

Everything here drives the real HTTP handlers, because that is the only place the
three interesting arms live.  The unit tests in ``gallery/secure.rs`` pin the
primitives (``existing_memberships``, ``clone_is_shared``, ``collapse_by_clone``)
but cannot reach the handler wiring — the handlers need a full ``AppState``.

The three arms, and what breaks if each is wrong:

* **same-album add** must still 409.  "At most once per *album*" is the half of
  the old invariant that survives; losing it puts the photo in one album twice.
* **cross-album add** must ADOPT the existing clone, not make a second one.  A
  second clone doubles storage, spends another decrypt+encrypt pass, and — being
  physically a different file — leaves an edit in one album unable to reach the
  other.
* **removal** must drop only the membership row while another album still points
  at the clone.  The destruction path deletes the clone blob, the clone ``photos``
  row, its encrypted blobs and its thumbnails; running it early blanks the photo
  in every other album while leaving their rows intact.  That is silent data
  loss, and it reads as corruption rather than as a deletion the user asked for.
"""

import pytest

from helpers import APIClient, generate_random_bytes
from conftest import USER_PASSWORD


def _unlock(client) -> str:
    return client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]


def _add(client, gallery_id: str, blob_id: str, token: str):
    """Raw add — returns the Response so a 409 can be asserted rather than raised."""
    return client.post(
        f"/api/galleries/secure/{gallery_id}/items",
        json_data={"blob_id": blob_id},
        headers={"x-gallery-token": token},
    )


def _items(client, gallery_id: str, token: str):
    return client.list_secure_gallery_items(gallery_id, token).get("items", [])


def _all_items(client, token: str):
    """The aggregate feed the secure smart albums are derived from."""
    r = client.get("/api/galleries/secure/items", headers={"x-gallery-token": token})
    r.raise_for_status()
    return r.json().get("items", [])


@pytest.fixture
def two_albums_and_a_photo(user_client):
    """A photo secured into album A, plus an empty album B.

    Byte-distinct content: ingest deduplicates on content hash, so two uploads of
    identical bytes collapse into one row and the "two photos" in a test become
    one (the trap test_93's fixture hit).  ``generate_random_bytes`` is distinct
    per call, and the assertion below refuses to let that silently change.
    """
    token = _unlock(user_client)
    album_a = user_client.create_secure_gallery("Z1 Album A")["gallery_id"]
    album_b = user_client.create_secure_gallery("Z1 Album B")["gallery_id"]
    assert album_a != album_b

    blob = user_client.upload_blob("photo", content=generate_random_bytes(4096))
    blob_id = blob["blob_id"]

    r = _add(user_client, album_a, blob_id, token)
    assert r.status_code == 201, f"precondition: first add must succeed, got {r.status_code}"
    first = r.json()
    assert not first.get("adopted"), "the FIRST add clones; it cannot be an adoption"

    return {
        "token": token,
        "album_a": album_a,
        "album_b": album_b,
        "blob_id": blob_id,
        "clone_blob_id": first["new_blob_id"],
        "item_a": first["item_id"],
    }


class TestMultiAlbumMembership:
    """The core Z1 property: one photo, several secure albums."""

    def test_a_photo_can_be_added_to_two_secure_albums(self, user_client, two_albums_and_a_photo):
        f = two_albums_and_a_photo

        r = _add(user_client, f["album_b"], f["blob_id"], f["token"])
        assert r.status_code == 201, (
            f"second album add must succeed under Z1 — got {r.status_code} "
            f"{r.text!r}. A 409 here is the pre-Z1 one-secure-album invariant."
        )

        assert len(_items(user_client, f["album_a"], f["token"])) == 1
        assert len(_items(user_client, f["album_b"], f["token"])) == 1

    def test_the_second_add_adopts_the_clone_instead_of_making_another(
        self, user_client, two_albums_and_a_photo
    ):
        """The property that makes multi-album membership cheap.

        Asserted on the returned clone id rather than on a byte count, so it
        survives any refactor of how the clone is produced.
        """
        f = two_albums_and_a_photo

        r = _add(user_client, f["album_b"], f["blob_id"], f["token"])
        assert r.status_code == 201
        second = r.json()

        assert second["new_blob_id"] == f["clone_blob_id"], (
            "the second album must reuse the FIRST album's clone; a different "
            "id means a second clone was produced — double storage, and an edit "
            "in one album can never reach the other"
        )
        assert second.get("adopted") is True
        assert second["item_id"] != f["item_a"], "each album gets its own membership row"

    def test_adding_to_the_same_album_twice_is_still_refused(
        self, user_client, two_albums_and_a_photo
    ):
        """The surviving half of the old invariant.

        Without this, Z1 would have traded "one album only" for "the same album
        as many times as you like", which is strictly worse than either rule.
        """
        f = two_albums_and_a_photo

        r = _add(user_client, f["album_a"], f["blob_id"], f["token"])
        assert r.status_code == 409, f"expected 409 for a same-album re-add, got {r.status_code}"
        assert len(_items(user_client, f["album_a"], f["token"])) == 1

    def test_moving_into_an_album_that_already_holds_the_photo_is_refused(
        self, user_client, two_albums_and_a_photo
    ):
        """Multi-membership makes a same-album duplicate reachable via /move.

        Refused rather than merged: silently dropping the source row would be a
        destructive reading of a request the user made as a move.
        """
        f = two_albums_and_a_photo
        assert _add(user_client, f["album_b"], f["blob_id"], f["token"]).status_code == 201

        r = user_client.move_secure_gallery_item(f["album_a"], f["item_a"], f["album_b"])
        assert r.status_code == 409, f"expected 409 moving into an album that has it, got {r.status_code}"

        # Both memberships survive the refusal — a rejected move changes nothing.
        assert len(_items(user_client, f["album_a"], f["token"])) == 1
        assert len(_items(user_client, f["album_b"], f["token"])) == 1

    def test_a_move_to_an_album_without_the_photo_still_works(
        self, user_client, two_albums_and_a_photo
    ):
        """Vacuity guard for the test above: a /move that always 409s would
        satisfy it while breaking the #31 and #43 pickers outright."""
        f = two_albums_and_a_photo

        r = user_client.move_secure_gallery_item(f["album_a"], f["item_a"], f["album_b"])
        assert r.status_code in (200, 204), f"an ordinary move must still work, got {r.status_code}"

        assert len(_items(user_client, f["album_a"], f["token"])) == 0
        assert len(_items(user_client, f["album_b"], f["token"])) == 1


class TestRemovalDoesNotDestroySharedBytes:
    """The data-loss guard — the reason this feature needed a refcount."""

    def test_removing_from_one_album_leaves_the_photo_intact_in_the_other(
        self, user_client, two_albums_and_a_photo
    ):
        f = two_albums_and_a_photo
        r = _add(user_client, f["album_b"], f["blob_id"], f["token"])
        assert r.status_code == 201
        item_b = r.json()["item_id"]

        # Precondition: the bytes are actually fetchable before the removal, or
        # the post-removal fetch below proves nothing.
        before = user_client.get(
            f"/api/blobs/{f['clone_blob_id']}", headers={"x-gallery-token": f["token"]}
        )
        assert before.status_code == 200, (
            f"precondition: the clone must be servable first, got {before.status_code}"
        )

        rm = user_client.remove_secure_gallery_item(f["album_a"], f["item_a"])
        assert rm.status_code in (200, 204)

        assert len(_items(user_client, f["album_a"], f["token"])) == 0, "removed from A"
        items_b = _items(user_client, f["album_b"], f["token"])
        assert len(items_b) == 1, "album B must still hold the photo"
        assert items_b[0]["id"] == item_b

        after = user_client.get(
            f"/api/blobs/{f['clone_blob_id']}", headers={"x-gallery-token": f["token"]}
        )
        assert after.status_code == 200, (
            "the shared clone must survive a removal from one album — a 404 here "
            "is the data-loss bug: album B still lists the item, but its bytes "
            "are gone, so the tile renders blank"
        )
        assert len(after.content) == len(before.content)

    def test_the_photo_stays_hidden_from_the_main_gallery_while_any_membership_remains(
        self, user_client, two_albums_and_a_photo
    ):
        """Un-hiding on the first removal would surface a photo the user still
        has in a secure album — the privacy-shaped half of the same bug."""
        f = two_albums_and_a_photo
        assert _add(user_client, f["album_b"], f["blob_id"], f["token"]).status_code == 201

        user_client.remove_secure_gallery_item(f["album_a"], f["item_a"])

        hidden = set(user_client.get_secure_gallery_blob_ids().get("blob_ids", []))
        assert f["clone_blob_id"] in hidden, (
            "still secured in album B, so the main gallery must keep hiding it"
        )

    def test_removing_the_last_membership_still_reclaims_the_clone(
        self, user_client, two_albums_and_a_photo
    ):
        """Vacuity guard for both tests above.

        A refcount that reads "shared" unconditionally satisfies every assertion
        in this class while making the destruction path unreachable — so every
        secure-album removal would leak its clone's bytes forever.
        """
        f = two_albums_and_a_photo

        rm = user_client.remove_secure_gallery_item(f["album_a"], f["item_a"])
        assert rm.status_code in (200, 204)

        gone = user_client.get(
            f"/api/blobs/{f['clone_blob_id']}", headers={"x-gallery-token": f["token"]}
        )
        assert gone.status_code == 404, (
            f"the last membership must reclaim the clone, got {gone.status_code} — "
            "otherwise the refcount never lets anything be deleted"
        )

        hidden = set(user_client.get_secure_gallery_blob_ids().get("blob_ids", []))
        assert f["clone_blob_id"] not in hidden, "and the original returns to the gallery"


class TestAggregateFeedCollapsesMemberships:
    """The aggregate feed is not just a listing — the secure smart albums are
    derived from it, so a duplicated row becomes a double-counted tile."""

    def test_a_photo_in_two_albums_is_one_tile_carrying_both(
        self, user_client, two_albums_and_a_photo
    ):
        f = two_albums_and_a_photo
        assert _add(user_client, f["album_b"], f["blob_id"], f["token"]).status_code == 201

        tiles = _all_items(user_client, f["token"])
        mine = [t for t in tiles if t.get("blob_id") == f["clone_blob_id"]]

        assert len(mine) == 1, (
            f"one photo in two albums must be ONE tile in the aggregate feed, "
            f"got {len(mine)} — duplicates here double-count every secure smart album"
        )

        gallery_ids = {g["id"] for g in mine[0].get("galleries", [])}
        assert gallery_ids == {f["album_a"], f["album_b"]}, (
            "the tile must carry every album it is in, so a client can route a "
            "'remove' — with N memberships, 'which album' is a real question"
        )
        assert mine[0]["gallery_id"] == f["album_a"], (
            "the representative is the OLDEST membership, so the tile's added_at "
            "is when the photo was secured, not when it was filed into album B"
        )

    def test_a_photo_in_one_album_is_unchanged(self, user_client, two_albums_and_a_photo):
        """The no-op property: before anyone uses multi-album membership the feed
        must be exactly what it was.  Without this, collapsing everything to a
        single tile passes the test above while destroying the feed."""
        f = two_albums_and_a_photo

        tiles = _all_items(user_client, f["token"])
        mine = [t for t in tiles if t.get("blob_id") == f["clone_blob_id"]]

        assert len(mine) == 1
        assert [g["id"] for g in mine[0].get("galleries", [])] == [f["album_a"]]
        assert mine[0]["gallery_id"] == f["album_a"]
        assert mine[0]["gallery_name"] == "Z1 Album A"
