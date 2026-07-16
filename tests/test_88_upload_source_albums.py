"""E2E: Takeout album capture on the browser-upload path.

`POST /api/photos/upload` used to record no album data at all, so any Google
Takeout imported through the browser's "Local Upload" lost 100% of its albums —
the album name is the *folder* name, and a browser upload only ever sends loose
bytes with no folder structure.

The web client now derives the album from the picked folder structure
(`web/src/utils/uploadAlbums.ts`, mirroring `server/src/import/sidecar.rs`) and
declares it per file via `X-Source-Album` / `X-Source-Album-Title`. These tests
pin the server half of that contract: the headers are percent-decoded, sanitised,
re-checked against the non-album folder rules (the header is client-supplied and
therefore untrusted), and recorded so `/api/photos/source-albums` — the endpoint
both clients rebuild album manifests from — returns them.
"""

import hashlib
from urllib.parse import quote

import pytest

from conftest import USER_PASSWORD
from helpers import APIClient, generate_test_jpeg, random_username


def _source_album_id(name: str, source: str = "google_takeout") -> str:
    """The album id both clients derive — `"src-" + sha256("<source> <name>")`.
    Computed here from the spec, independently of the server's implementation."""
    return "src-" + hashlib.sha256(f"{source} {name}".encode()).hexdigest()


def _album_headers(album: str, title: str = None) -> dict:
    """The headers the web client sends. Percent-encoded because header values
    are bytes: a non-Latin-1 album name can't go in a raw header at all."""
    h = {"X-Source-Album": quote(album)}
    if title is not None:
        h["X-Source-Album-Title"] = quote(title)
    return h


def _albums_by_name(client) -> dict:
    r = client.get("/api/photos/source-albums")
    r.raise_for_status()
    return {a["name"]: a for a in r.json()["albums"]}


@pytest.fixture(scope="module")
def uploader(primary_server, primary_admin):
    """A dedicated non-admin user, so this module's albums can be asserted in
    isolation from every other test's photos (source-albums is per-user).

    Module-scoped — unlike the function-scoped `user_client` — because these
    tests build one library across an ordered sequence of uploads.
    """
    username = random_username("albumup_")
    primary_admin.admin_create_user(username, USER_PASSWORD, role="user")
    client = APIClient(primary_server.base_url)
    client.login(username, USER_PASSWORD)
    client.username = username
    return client


class TestUploadSourceAlbums:
    def test_upload_with_album_header_records_membership(self, uploader):
        resp = uploader.upload_photo(
            filename="IMG_1.jpg",
            content=generate_test_jpeg(width=21, height=21),
            extra_headers=_album_headers("Trip to Rome"),
        )
        albums = _albums_by_name(uploader)
        assert "Trip to Rome" in albums, (
            "a browser-uploaded Takeout photo must keep its album"
        )
        assert albums["Trip to Rome"]["photo_ids"] == [resp["photo_id"]]
        assert albums["Trip to Rome"]["source"] == "google_takeout"

    def test_upload_without_album_header_records_nothing(self, uploader):
        """The overwhelmingly common case — a normal upload is not an album."""
        before = _albums_by_name(uploader)
        uploader.upload_photo(
            filename="IMG_plain.jpg", content=generate_test_jpeg(width=22, height=22)
        )
        assert _albums_by_name(uploader) == before

    def test_album_title_survives_the_round_trip(self, uploader):
        """The real title is non-ASCII and unrepresentable in a raw header — the
        whole reason both sides percent-encode."""
        title = "Mum & Dad's 40th — 東京"
        uploader.upload_photo(
            filename="IMG_2.jpg",
            content=generate_test_jpeg(width=23, height=23),
            extra_headers=_album_headers("Mum _ Dad_s 40th", title),
        )
        albums = _albums_by_name(uploader)
        assert "Mum _ Dad_s 40th" in albums, "identity stays the mangled folder name"
        assert albums["Mum _ Dad_s 40th"]["title"] == title

    def test_duplicate_upload_still_records_its_album(self, uploader):
        """Takeout ships the SAME bytes in every album folder a photo belongs to,
        so the second copy hash-dedups. If the dedup branch skipped album
        recording, every album would silently lose every member that also lives
        in another album or a date folder — i.e. all of them."""
        content = generate_test_jpeg(width=24, height=24)
        first = uploader.upload_photo(
            filename="IMG_3.jpg",
            content=content,
            extra_headers=_album_headers("Best of 2021"),
        )
        # The same bytes again, this time from a different album's folder.
        second = uploader.upload_photo(
            filename="IMG_3.jpg",
            content=content,
            extra_headers=_album_headers("Holidays"),
        )
        assert second["photo_id"] == first["photo_id"], "the copy must dedup"

        albums = _albums_by_name(uploader)
        assert first["photo_id"] in albums["Best of 2021"]["photo_ids"]
        assert first["photo_id"] in albums["Holidays"]["photo_ids"], (
            "the deduped copy's album membership must still be recorded"
        )

    def test_reupload_is_idempotent(self, uploader):
        """Re-running an interrupted import must not duplicate membership."""
        content = generate_test_jpeg(width=25, height=25)
        headers = _album_headers("Reupload Album")
        uploader.upload_photo(filename="IMG_4.jpg", content=content, extra_headers=headers)
        uploader.upload_photo(filename="IMG_4.jpg", content=content, extra_headers=headers)

        albums = _albums_by_name(uploader)
        assert len(albums["Reupload Album"]["photo_ids"]) == 1

    def test_title_is_filled_in_on_a_later_upload(self, uploader):
        """A membership recorded without a title must still be able to acquire
        one — INSERT OR IGNORE alone would drop it forever."""
        content = generate_test_jpeg(width=26, height=26)
        uploader.upload_photo(
            filename="IMG_6.jpg", content=content, extra_headers=_album_headers("Late Title")
        )
        assert _albums_by_name(uploader)["Late Title"]["title"] is None

        uploader.upload_photo(
            filename="IMG_6.jpg",
            content=content,
            extra_headers=_album_headers("Late Title", "The Real Name"),
        )
        assert _albums_by_name(uploader)["Late Title"]["title"] == "The Real Name"

    @pytest.mark.parametrize("folder", ["Photos from 2021", "Takeout", "Google Photos"])
    def test_googles_non_album_folders_are_rejected(self, uploader, folder):
        """The browser filters these, but the header is client-supplied — a
        malicious or buggy client must not be able to create a date-folder album
        holding the user's entire library."""
        uploader.upload_photo(
            filename=f"IMG_{abs(hash(folder)) % 997}.jpg",
            content=generate_test_jpeg(width=27, height=27),
            extra_headers=_album_headers(folder),
        )
        assert folder not in _albums_by_name(uploader)

    def test_hostile_album_names_are_sanitised(self, uploader):
        """Album names are plaintext server-side and rendered by both clients."""
        uploader.upload_photo(
            filename="IMG_evil.jpg",
            content=generate_test_jpeg(width=28, height=28),
            # Bidi override + control char + padded whitespace.
            extra_headers=_album_headers("  Evil\u202e \u0001  Album  "),
        )
        albums = _albums_by_name(uploader)
        assert "Evil Album" in albums, f"expected sanitised name; got {list(albums)}"

    def test_blank_album_name_is_ignored(self, uploader):
        before = _albums_by_name(uploader)
        uploader.upload_photo(
            filename="IMG_blank.jpg",
            content=generate_test_jpeg(width=29, height=29),
            extra_headers=_album_headers("   "),
        )
        assert _albums_by_name(uploader) == before

    def test_dismissing_an_album_stops_reconstruction_recreating_it(self, uploader):
        """Deleting a reconstructed album used to be impossible: the next rebuild
        recreated it from the untouched server-side membership, on every device.
        A tombstone makes the deletion stick — everywhere, since reconstruction
        reads the filtered list."""
        content = generate_test_jpeg(width=31, height=31)
        resp = uploader.upload_photo(
            filename="IMG_dismiss.jpg",
            content=content,
            extra_headers=_album_headers("Album To Delete"),
        )
        assert "Album To Delete" in _albums_by_name(uploader)

        r = uploader.post(
            "/api/photos/source-albums/dismiss",
            json_data={"album_id": _source_album_id("Album To Delete")},
        )
        r.raise_for_status()
        body = r.json()
        assert body["dismissed"] is True, f"the server must resolve the id; got {body}"
        assert body["name"] == "Album To Delete"

        assert "Album To Delete" not in _albums_by_name(uploader), (
            "a dismissed album must not come back from reconstruction"
        )

        # The photo itself survives — "delete album, keep photos".
        library = {p["id"] for p in uploader.list_photos()["photos"]}
        assert resp["photo_id"] in library, (
            "dismissing an album must never remove its photos from the library"
        )

    def test_dismiss_is_idempotent_and_scoped_to_one_album(self, uploader):
        for _ in range(2):
            r = uploader.post(
                "/api/photos/source-albums/dismiss",
                json_data={"album_id": _source_album_id("Album To Delete")},
            )
            r.raise_for_status()
            assert r.json()["dismissed"] is True

        # Other albums are untouched.
        assert "Trip to Rome" in _albums_by_name(uploader)

    def test_dismissing_a_non_source_album_is_a_no_op(self, uploader):
        """Ordinary user-created albums aren't reconstructed and need no
        tombstone — the client can call this blindly for any `src-` id."""
        r = uploader.post(
            "/api/photos/source-albums/dismiss",
            json_data={"album_id": "src-" + "0" * 64},
        )
        r.raise_for_status()
        assert r.json() == {"dismissed": False, "name": None}

    def test_re_uploading_a_dismissed_albums_photo_does_not_resurrect_it(self, uploader):
        """Re-running an import must not undo the user's deletion."""
        uploader.upload_photo(
            filename="IMG_dismiss.jpg",
            content=generate_test_jpeg(width=31, height=31),
            extra_headers=_album_headers("Album To Delete"),
        )
        assert "Album To Delete" not in _albums_by_name(uploader), (
            "a re-import must not resurrect a deleted album"
        )

    def test_dismiss_cannot_touch_another_users_albums(self, uploader, user_client):
        """Only the caller's own albums are candidates for the id lookup."""
        r = user_client.post(
            "/api/photos/source-albums/dismiss",
            json_data={"album_id": _source_album_id("Trip to Rome")},
        )
        r.raise_for_status()
        assert r.json()["dismissed"] is False, (
            "another user's album must not be resolvable, let alone tombstoned"
        )
        assert "Trip to Rome" in _albums_by_name(uploader), (
            "the owner's album must be unaffected"
        )

    def test_albums_are_per_user(self, uploader, admin_client):
        """source-albums is not admin-gated — each user reads only their own."""
        assert "Trip to Rome" in _albums_by_name(uploader)
        admin_albums = _albums_by_name(admin_client)
        uploader_ids = {
            pid for a in _albums_by_name(uploader).values() for pid in a["photo_ids"]
        }
        admin_ids = {pid for a in admin_albums.values() for pid in a["photo_ids"]}
        assert not (uploader_ids & admin_ids), (
            "one user's album members must never appear in another's"
        )
