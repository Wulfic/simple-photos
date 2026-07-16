"""E2E: Google Takeout album backfill.

Album membership is only captured at import time, and only since Jul 2026. Every
library imported before that — including anything uploaded through the browser,
which never records albums at all — has no `photo_source_albums` rows, so clients
reconstruct partial or empty albums. No other path repairs it: a re-scan skips
files that are already registered *before* the album-recording code can run.

`POST /api/admin/import/google-photos/backfill-albums` is that repair. These tests
build a synthetic Takeout export containing the shapes that actually break
importers — an album folder whose photos are duplicated into `Photos from YYYY`,
a `(1)` duplicate counter with its displaced sidecar, an `-edited` pair, a legacy
`.json` sidecar name, an album-level `metadata.json`, and a plain non-Takeout
folder — then assert the recovered membership through `/api/photos/source-albums`,
the same endpoint the web and Android clients rebuild their albums from.

The library is seeded via `/api/photos/upload` precisely because that path records
no album data, which is exactly the broken pre-fix state the backfill must repair.
"""

import pytest

from helpers import generate_test_jpeg


# A real per-photo sidecar: `photoTakenTime` is what marks a directory as a
# genuine Takeout export (the `is_takeout` gate) rather than a user folder.
PHOTO_SIDECAR = b'{"title":"%s","photoTakenTime":{"timestamp":"1494963474"}}'

# The album's real Google Photos name, deliberately unlike its folder name and
# unrepresentable in a raw HTTP header — proving the title survives end to end.
ROME_TITLE = "Trip to Rome — 東京 & back"
ALBUM_METADATA = (
    '{"title":"%s","access":"protected"}' % ROME_TITLE
).encode()


def _sidecar_for(name: str) -> bytes:
    return PHOTO_SIDECAR % name.encode()


def _write(path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)


@pytest.fixture(scope="module")
def takeout_tree(tmp_path_factory):
    """A synthetic Takeout export laid out the way Google actually ships one.

    Deliberately lives outside the server's storage root so the autoscan can't
    register these files itself — the backfill must work purely by matching the
    already-uploaded photos by content hash.

    Returns `(root, photos)` where `photos` maps a logical name to its bytes.
    """
    root = tmp_path_factory.mktemp("takeout")
    gphotos = root / "Takeout" / "Google Photos"

    # Distinct dimensions → distinct bytes → distinct content hashes, so the
    # upload path's hash-dedup can't silently collapse two logical photos.
    photos = {
        "IMG_1.jpg": generate_test_jpeg(width=8, height=8),
        "IMG_2(1).jpg": generate_test_jpeg(width=9, height=9),
        "IMG_5.jpg": generate_test_jpeg(width=10, height=10),
        "IMG_5-edited.jpg": generate_test_jpeg(width=11, height=11),
        "IMG_7.jpg": generate_test_jpeg(width=12, height=12),
        "IMG_8.jpg": generate_test_jpeg(width=13, height=13),
        "IMG_9.jpg": generate_test_jpeg(width=14, height=14),
    }

    rome = gphotos / "Trip to Rome"
    # Album-level metadata.json — must never be mistaken for a photo sidecar, and
    # carries the album's REAL title. Deliberately different from the folder name:
    # Takeout mangles folder names (special characters → "_", truncation), so the
    # title is the only faithful name, while the folder name stays the identity
    # key clients derive the album id from.
    _write(rome / "metadata.json", ALBUM_METADATA)
    # A plain album member.
    _write(rome / "IMG_1.jpg", photos["IMG_1.jpg"])
    _write(rome / "IMG_1.jpg.supplemental-metadata.json", _sidecar_for("IMG_1.jpg"))
    # THE classic gotcha: "IMG_2(1).jpg" pairs with "IMG_2.jpg(1).json".
    _write(rome / "IMG_2(1).jpg", photos["IMG_2(1).jpg"])
    _write(rome / "IMG_2.jpg(1).json", _sidecar_for("IMG_2.jpg"))
    # An edited/original pair: the import keeps "-edited" and drops the original.
    _write(rome / "IMG_5.jpg", photos["IMG_5.jpg"])
    _write(rome / "IMG_5-edited.jpg", photos["IMG_5-edited.jpg"])
    _write(rome / "IMG_5.jpg.supplemental-metadata.json", _sidecar_for("IMG_5.jpg"))
    # In the album on disk but never imported → a genuine, reportable gap.
    _write(rome / "IMG_9.jpg", photos["IMG_9.jpg"])
    _write(rome / "IMG_9.jpg.supplemental-metadata.json", _sidecar_for("IMG_9.jpg"))

    # A second album, using the legacy sidecar naming.
    birthday = gphotos / "Birthday"
    _write(birthday / "IMG_7.jpg", photos["IMG_7.jpg"])
    _write(birthday / "IMG_7.jpg.json", _sidecar_for("IMG_7.jpg"))

    # Takeout duplicates every album member into its date folder. Same bytes, so
    # it dedups to the same photo — and a date folder is never an album.
    date_folder = gphotos / "Photos from 2021"
    _write(date_folder / "IMG_1.jpg", photos["IMG_1.jpg"])
    _write(date_folder / "IMG_1.jpg.supplemental-metadata.json", _sidecar_for("IMG_1.jpg"))

    # A plain user folder: media, but no Google sidecars → never an album.
    plain = root / "Vacation Photos"
    _write(plain / "IMG_8.jpg", photos["IMG_8.jpg"])

    return root, photos


@pytest.fixture(scope="module")
def seeded_library(primary_admin, takeout_tree):
    """Upload the Takeout photos the way a pre-fix import left them: real photo
    rows, zero album membership. `IMG_5.jpg` (the unedited original) and
    `IMG_9.jpg` are deliberately NOT uploaded.

    Seeds once per module (hence `primary_admin`, the session-scoped client,
    rather than the function-scoped `admin_client` alias) so the ordered
    before/after assertions below share one library.

    Returns a map of logical name → photo id.
    """
    _root, photos = takeout_tree
    ids = {}
    for name in ["IMG_1.jpg", "IMG_2(1).jpg", "IMG_5-edited.jpg", "IMG_7.jpg", "IMG_8.jpg"]:
        resp = primary_admin.upload_photo(filename=name, content=photos[name])
        ids[name] = resp["photo_id"]
    return ids


def _albums_by_name(admin_client) -> dict:
    r = admin_client.get("/api/photos/source-albums")
    r.raise_for_status()
    return {a["name"]: a for a in r.json()["albums"]}


def _backfill(admin_client, path) -> dict:
    r = admin_client.post(
        "/api/admin/import/google-photos/backfill-albums",
        json_data={"path": str(path)},
    )
    r.raise_for_status()
    return r.json()


class TestTakeoutAlbumBackfill:
    def test_uploaded_library_starts_with_no_album_membership(
        self, admin_client, seeded_library
    ):
        """The bug being repaired: uploading a Takeout export records no albums."""
        albums = _albums_by_name(admin_client)
        for name in ["Trip to Rome", "Birthday"]:
            assert name not in albums, (
                f"'{name}' should not exist before the backfill — if it does, this "
                "test can't prove the backfill is what created it"
            )

    def test_backfill_recovers_album_membership(
        self, admin_client, takeout_tree, seeded_library
    ):
        root, _photos = takeout_tree
        result = _backfill(admin_client, root)

        assert result["albums_seen"] == 2, (
            "only 'Trip to Rome' and 'Birthday' are albums — 'Photos from 2021' is "
            f"a date folder and 'Vacation Photos' has no sidecars; got {result}"
        )
        assert result["albums_recorded"] == 4, f"expected 4 memberships, got {result}"
        assert result["photos_matched"] == 4, f"got {result}"
        assert result["shadowed_skipped"] == 1, (
            f"IMG_5.jpg's '-edited' sibling was imported instead; got {result}"
        )
        assert result["photos_unmatched"] == 1, (
            f"IMG_9.jpg is in the album on disk but was never imported; got {result}"
        )
        assert result["errors_total"] == 0, f"got {result['errors']}"

    def test_recovered_albums_are_readable_by_clients(
        self, admin_client, seeded_library
    ):
        """The payload clients actually rebuild their manifests from."""
        albums = _albums_by_name(admin_client)

        assert "Trip to Rome" in albums
        assert "Birthday" in albums
        assert albums["Trip to Rome"]["source"] == "google_takeout"

        rome = set(albums["Trip to Rome"]["photo_ids"])
        assert rome == {
            seeded_library["IMG_1.jpg"],
            seeded_library["IMG_2(1).jpg"],
            seeded_library["IMG_5-edited.jpg"],
        }, "the album must contain exactly its imported members"

        assert albums["Birthday"]["photo_ids"] == [seeded_library["IMG_7.jpg"]]

    def test_albums_carry_their_real_title_keyed_by_folder_name(
        self, admin_client, seeded_library
    ):
        """Takeout mangles album folder names; the real title lives only in the
        album's metadata.json. Clients display the title but must keep keying on
        the folder name — the deterministic album id derives from it, so re-keying
        would orphan and duplicate every album already on a device."""
        albums = _albums_by_name(admin_client)

        assert albums["Trip to Rome"]["title"] == ROME_TITLE, (
            "the real title must be read from the album's metadata.json"
        )
        assert "Trip to Rome" in albums, "identity stays the folder name"

        # An album with no metadata.json (older exports) has no title — clients
        # then fall back to the folder name.
        assert albums["Birthday"]["title"] is None, (
            f"expected no title for a metadata-less album; got {albums['Birthday']}"
        )

    def test_non_album_folders_never_become_albums(self, admin_client, seeded_library):
        """The `is_takeout` gate and the date-folder rule, from the client's view."""
        albums = _albums_by_name(admin_client)

        assert "Vacation Photos" not in albums, (
            "a plain user folder with no Google sidecars must never become an album"
        )
        assert "Photos from 2021" not in albums, "date folders are not albums"
        assert "Google Photos" not in albums and "Takeout" not in albums, (
            "container folders are not albums"
        )

        # IMG_8 lives only in the plain folder, so it belongs to no album at all.
        all_members = {pid for a in albums.values() for pid in a["photo_ids"]}
        assert seeded_library["IMG_8.jpg"] not in all_members

    def test_backfill_is_idempotent(self, admin_client, takeout_tree, seeded_library):
        """Safe to re-run against a live library — the whole point of INSERT OR
        IGNORE. A second pass must add nothing and duplicate nothing."""
        root, _photos = takeout_tree
        before = _albums_by_name(admin_client)

        result = _backfill(admin_client, root)

        assert result["albums_recorded"] == 0, (
            f"a re-run must record nothing new; got {result}"
        )
        assert result["photos_matched"] == 4, (
            "the photos are still found, they're just already recorded"
        )
        assert _albums_by_name(admin_client) == before, "membership must not change"

    def test_backfill_rejects_bad_paths(self, admin_client, takeout_tree):
        root, _photos = takeout_tree

        traversal = admin_client.post(
            "/api/admin/import/google-photos/backfill-albums",
            json_data={"path": str(root / ".." / "etc")},
        )
        assert traversal.status_code == 400, "path traversal must be rejected"

        missing = admin_client.post(
            "/api/admin/import/google-photos/backfill-albums",
            json_data={"path": str(root / "does-not-exist")},
        )
        assert missing.status_code == 400, "an unresolvable path must be rejected"

        not_a_dir = admin_client.post(
            "/api/admin/import/google-photos/backfill-albums",
            json_data={"path": str(root / "Vacation Photos" / "IMG_8.jpg")},
        )
        assert not_a_dir.status_code == 400, "a file path must be rejected"

    def test_backfill_requires_admin(self, user_client, takeout_tree):
        """It reads arbitrary server-side directories — admin only."""
        root, _photos = takeout_tree
        resp = user_client.post(
            "/api/admin/import/google-photos/backfill-albums",
            json_data={"path": str(root)},
        )
        assert resp.status_code == 403, f"non-admin must be refused, got {resp.status_code}"
