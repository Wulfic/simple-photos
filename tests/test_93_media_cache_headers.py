"""
Test 93: B6 — `Cache-Control` on media routes, read off the wire.

`todo.md` (B6) states the requirement and the reason in one line: "pin it with an
E2E that reads the header **off the wire**. Every one of the 17 sites is
'correct' when read in the handler; the defect only exists after the middleware
runs, which is why unit tests have never seen it."

That is the whole point of this module. `security.rs` used to stamp
`Cache-Control: no-store, no-cache, must-revalidate` on **every** `/api/`
response with a blanket `insert`, overwriting whatever the handler had set. Read
in `photos/serve.rs`, a thumbnail declares `private, max-age=86400`; measured on
the wire it arrived as `no-store`. Seventeen handler sites across four files were
dead, and with them the ETag machinery behind them — `no-store` forbids storing
the response at all, so there is nothing left to revalidate with. Every tile in a
scrolled grid was re-fetched and re-decrypted on every visit, and #49's
swap-quality-keep-playing picker had no bytes to keep.

The fix is a route allowlist (`http_utils::is_cacheable_media_route`) plus a
per-item confidentiality verdict returned by `require_secure_access`. Both halves
need proving from outside the process, and they pull in opposite directions,
which is why this file asserts both:

  1. ordinary media really is cacheable now (the bug), **and**
  2. the JSON API and secure-gallery media really are not (the reason the
     blanket rule existed in the first place).

Assertion 2 is not decoration. Without it, "delete the middleware" passes every
test in group 1 while writing refresh tokens and decrypted secure photos into a
browser's on-disk cache.

## The secure-gate half

`TestSourceFileIsGatedLikeEveryOtherMediaRoute` is not a caching test. It covers
a confidentiality hole found while classifying these routes:
`/api/photos/{id}/source-file` took no `GalleryToken` and never called
`require_secure_access`, unlike `file` / `web` / `thumb` / `motion-video`.
Securing a photo hides its `photos` row but never deletes it and never clears
`source_path`, so an account session alone downloaded the **original,
unconverted** source of a photo sitting in a secure album. The route had no E2E
coverage at all before this file.
"""

import time

import pytest

from conftest import ADMIN_PASSWORD
from helpers import APIClient, generate_test_jpeg, generate_test_tiff, random_username

USER_PASSWORD = "E2eUserPass456!"

# The two values the handlers set, spelled out rather than imported, so a change
# to either is a deliberate edit to this file and not a silent agreement.
MEDIA_CACHE_1D = "private, max-age=86400"
BLOB_CACHE_IMMUTABLE = "private, max-age=31536000, immutable"
NO_STORE = "no-store, no-cache, must-revalidate"


def _cache_control(response) -> str:
    return response.headers.get("Cache-Control", "<absent>")


def _tiff(color: bytes) -> bytes:
    """`generate_test_tiff()` recoloured, so two fixtures are byte-distinct.

    The helper takes no parameters and returns identical bytes on every call, so
    two uploads of it deduplicate into a single `photos` row (see the fixture).
    Its pixel strip is the final `2 * 2 * 3` bytes of the file and every IFD
    offset points *before* it, so swapping the colour leaves a fully valid TIFF
    with a different content hash.

    `generate_test_tiff_with_exif` looks like the obvious alternative and is
    not: it returns **JPEG** bytes intended for a `.tiff` filename, and the
    upload endpoint answers 500 for that combination.
    """
    return generate_test_tiff()[:-12] + color * 4


def _await_converted(admin: APIClient, stem: str, timeout: float = 120.0) -> str:
    """Photo id of the row `run_conversion_pass` registered for `stem`.

    A deferred upload answers `202 {"status": "queued"}` and carries **no photo
    id** — the row does not exist yet, and when it appears it is a *new* row for
    the converted output (`{stem}.jpg`), not the `.tiff` that was posted. So the
    id has to be discovered by filename after the pass runs, and re-triggering
    the autoscan each poll is how the production cadence behaves anyway.
    """
    deadline = time.time() + timeout
    seen = []
    while time.time() < deadline:
        photos = admin.list_photos(limit=500).get("photos", [])
        seen = [p.get("filename", "") for p in photos]
        for p in photos:
            name = p.get("filename", "")
            if name.rsplit(".", 1)[0] == stem:
                return p["id"]
        try:
            admin.admin_trigger_autoscan()
        except Exception:
            pass
        time.sleep(2)

    pytest.fail(
        f"the conversion pass never registered a photo for {stem!r} within "
        f"{timeout:.0f}s. Without it `/source-file` 404s and the secure-gate "
        f"tests below would pass vacuously. Filenames seen: {seen}"
    )


# ── Fixture ──────────────────────────────────────────────────────────


@pytest.fixture(scope="module")
def media(primary_server, primary_admin):
    """One user with: an ordinary photo, a converted photo, and a secured photo.

    Module-scoped — every test below is a header read against the same three
    rows, and the secure-gallery setup costs an unlock round-trip.
    """
    client = APIClient(primary_server.base_url)
    username = random_username("cachehdr_")
    primary_admin.admin_create_user(username, USER_PASSWORD, role="user")
    client.login(username, USER_PASSWORD)

    # ── Every fixture photo must be byte-DISTINCT ────────────────────────
    #
    # Not a style preference. Ingest deduplicates on content hash, so two
    # uploads of identical bytes collapse into **one** `photos` row with one id.
    # The first draft of this fixture called `generate_test_jpeg(64, 64)` and
    # `generate_test_tiff()` twice each; the ordinary photo and the secured
    # photo came back as the same row, so securing one secured the other and the
    # "ordinary media is cacheable" tests failed with 401. The dedup is correct
    # behaviour — the fixture was wrong — and it fails in the *safe* direction
    # (a test that breaks), which is the only reason it was caught.
    ordinary = client.upload_photo(
        filename="cache_ordinary.jpg",
        content=generate_test_jpeg(width=64, height=64),
        mime_type="image/jpeg",
    )
    secured = client.upload_photo(
        filename="cache_secured.jpg",
        content=generate_test_jpeg(width=48, height=48),
        mime_type="image/jpeg",
    )

    # A `.tiff` filename is what puts these through `conversion_target`, so the
    # server keeps the upload as `source_path` — which is the thing
    # `/source-file` serves and the thing the secure gate must protect. BMP
    # would NOT work here: it is absent from `conversion_target`'s image arm, so
    # `/source-file` would 404 for everyone and the gate test would prove
    assert len({ordinary["photo_id"], secured["photo_id"]}) == 2, (
        "fixture photos deduplicated into one row — every assertion below would "
        "then be about the wrong photo"
    )

    gallery = client.create_secure_gallery("Cache Header Vault")
    token = client.unlock_secure_gallery(USER_PASSWORD)["gallery_token"]
    # A server-side photo is secured by its **photo id**: `add_gallery_item`
    # resolves the canonical original identity, and for a server-side photo that
    # is the id itself.
    client.add_secure_gallery_item(gallery["gallery_id"], secured["photo_id"], token)

    # ── The converted pair, which must go through a different door ───────
    #
    # `/source-file` serves `photos.source_path`, and **the upload endpoint
    # never writes that column**. An ordinary upload of a convertible file takes
    # `upload.rs`'s *inline* branch: it converts and registers, but records no
    # source. Only `run_conversion_pass` (`ingest.rs`) keeps the original and
    # sets `source_path`, and the one way to reach it over HTTP is a **deferred**
    # upload — which `upload.rs` gates on `X-Defer-Conversion` **and admin**,
    # because the pass attributes new photos to the admin user.
    #
    # So these two are uploaded as admin, and every assertion about them is made
    # as admin. Using the regular user here is what produced a 404 on the first
    # run, and a 404 would have made the gate test pass while proving nothing.
    for name, colour in (
        ("cache_src_open.tiff", b"\xff\x00\x00"),
        ("cache_src_secret.tiff", b"\x00\x00\xff"),
    ):
        primary_admin.upload_photo(
            filename=name,
            content=_tiff(colour),
            mime_type="image/tiff",
            extra_headers={"X-Defer-Conversion": "1"},
        )

    converted_id = _await_converted(primary_admin, "cache_src_open")
    secured_converted_id = _await_converted(primary_admin, "cache_src_secret")

    admin_gallery = primary_admin.create_secure_gallery("Cache Header Admin Vault")
    admin_token = primary_admin.unlock_secure_gallery(ADMIN_PASSWORD)["gallery_token"]
    primary_admin.add_secure_gallery_item(
        admin_gallery["gallery_id"], secured_converted_id, admin_token
    )

    return {
        "client": client,
        "admin": primary_admin,
        "ordinary_id": ordinary["photo_id"],
        "secured_id": secured["photo_id"],
        "converted_id": converted_id,
        "secured_converted_id": secured_converted_id,
        "token": token,
        "admin_token": admin_token,
    }


# ── The B6 defect ────────────────────────────────────────────────────


class TestOrdinaryMediaIsCacheableOnTheWire:
    """The regression, measured where it actually happened.

    On the pre-fix tree every assertion in this class reports
    `no-store, no-cache, must-revalidate`.
    """

    def test_thumb_keeps_its_handler_header(self, media):
        r = media["client"].get(f"/api/photos/{media['ordinary_id']}/thumb")
        assert r.status_code == 200
        assert _cache_control(r) == MEDIA_CACHE_1D, (
            "the middleware overwrote serve_thumbnail's Cache-Control — this is B6"
        )

    def test_file_keeps_its_handler_header(self, media):
        r = media["client"].get(f"/api/photos/{media['ordinary_id']}/file")
        assert r.status_code == 200
        assert _cache_control(r) == MEDIA_CACHE_1D

    def test_web_keeps_its_handler_header(self, media):
        r = media["client"].get(f"/api/photos/{media['ordinary_id']}/web")
        assert r.status_code == 200
        assert _cache_control(r) == MEDIA_CACHE_1D

    def test_the_etag_handshake_is_alive_again(self, media):
        """The payoff, and the part a header check alone does not prove.

        `no-store` forbids storing the response, so the ETag the handler sends is
        unusable by construction — the client has nothing to revalidate. This
        drives the full conditional round-trip: fetch, keep the ETag, re-fetch
        with `If-None-Match`, and require a 304 whose own `Cache-Control` still
        permits storage. A 304 that said `no-store` would tell the client to
        discard the very copy it just revalidated.
        """
        client = media["client"]
        first = client.get(f"/api/photos/{media['ordinary_id']}/thumb")
        assert first.status_code == 200
        etag = first.headers.get("ETag")
        assert etag, "serve_thumbnail must send an ETag for revalidation to exist"

        second = client.get(
            f"/api/photos/{media['ordinary_id']}/thumb",
            headers={"If-None-Match": etag},
        )
        assert second.status_code == 304, (
            f"expected a conditional hit, got {second.status_code} — the ETag "
            "round-trip is what caching buys and it must survive the middleware"
        )
        assert _cache_control(second) == MEDIA_CACHE_1D


class TestJsonIsStillNeverStored:
    """The vacuity guard, and the one with teeth.

    Deleting the middleware's `Cache-Control` insert entirely would satisfy every
    assertion in the class above. These two are what stops that, and the second
    is a token-bearing response.
    """

    def test_photo_listing_is_no_store(self, media):
        r = media["client"].get("/api/photos", params={"limit": 1})
        assert r.status_code == 200
        assert _cache_control(r) == NO_STORE

    def test_the_secure_item_listing_is_no_store(self, media):
        r = media["client"].get(
            "/api/galleries/secure/items",
            headers={"x-gallery-token": media["token"]},
        )
        assert r.status_code == 200
        assert _cache_control(r) == NO_STORE, (
            "this response enumerates the contents of a secure album; it must "
            "never be written to disk"
        )


class TestSecureMediaIsNeverStored:
    """Secure media rides the *same routes* as ordinary media — only the unlock
    token distinguishes it — so the allowlist alone cannot protect it. The
    handler picks `no-store` from `require_secure_access`'s verdict, and these
    read that decision off the wire.
    """

    @pytest.mark.parametrize("kind", ["thumb", "file", "web"])
    def test_secure_photo_media_is_no_store(self, media, kind):
        r = media["client"].get(
            f"/api/photos/{media['secured_id']}/{kind}",
            headers={"x-gallery-token": media["token"]},
        )
        assert r.status_code == 200, (
            f"precondition: the unlocked fetch must succeed, got {r.status_code} — "
            "a 401 here would make the header assertion vacuous"
        )
        assert _cache_control(r) == NO_STORE, (
            f"/{kind} of a secured photo was marked cacheable: {_cache_control(r)!r}. "
            "A browser cache entry outlives both the unlock token and the session."
        )

    def test_the_secure_no_store_is_byte_identical_to_the_default(self, media):
        """Two spellings of "do not store" are different on the wire. If secure
        media used its own wording, an intermediary that understood one and not
        the other would cache it.
        """
        secure = media["client"].get(
            f"/api/photos/{media['secured_id']}/thumb",
            headers={"x-gallery-token": media["token"]},
        )
        default = media["client"].get("/api/photos", params={"limit": 1})
        assert _cache_control(secure) == _cache_control(default) == NO_STORE


class TestSourceFileIsGatedLikeEveryOtherMediaRoute:
    """The confidentiality hole found while classifying these routes.

    `/api/photos/{id}/source-file` served the **original, unconverted** file of
    any owned photo with no `GalleryToken` and no `require_secure_access` call,
    while every sibling route gated it. The route had no E2E coverage at all.
    """

    def test_an_ordinary_converted_photo_still_serves_its_source(self, media):
        """Precondition, and it runs first on purpose.

        If conversion never happened, `source_path` is NULL and this endpoint
        404s for everyone — which would make the 401 assertion below pass for
        entirely the wrong reason. This is the same vacuity trap `test_92`
        records, and it is not hypothetical: the first draft of this fixture
        uploaded the TIFFs as a regular user, took `upload.rs`'s inline branch
        (which converts but never records `source_path`), and got a 404 here.
        """
        r = media["admin"].get(f"/api/photos/{media['converted_id']}/source-file")
        assert r.status_code == 200, (
            "no source file was recorded, so there is nothing to protect and "
            "the gate test below would be vacuous"
        )
        assert len(r.content) > 0

    def test_a_secured_photos_source_requires_the_unlock_token(self, media):
        """**The hole, verbatim.** Returns 200 and the plaintext original on the
        pre-fix tree, because the handler took no token and never called
        `require_secure_access`."""
        r = media["admin"].get(
            f"/api/photos/{media['secured_converted_id']}/source-file"
        )
        assert r.status_code == 401, (
            f"expected 401, got {r.status_code}: an account session alone "
            "downloaded the unconverted original of a photo in a secure album"
        )

    def test_the_token_still_opens_it(self, media):
        """The other half — a gate that refuses everyone is not a fix."""
        r = media["admin"].get(
            f"/api/photos/{media['secured_converted_id']}/source-file",
            headers={"x-gallery-token": media["admin_token"]},
        )
        assert r.status_code == 200
        assert len(r.content) > 0

    def test_and_the_unlocked_source_is_not_cacheable(self, media):
        r = media["admin"].get(
            f"/api/photos/{media['secured_converted_id']}/source-file",
            headers={"x-gallery-token": media["admin_token"]},
        )
        assert _cache_control(r) == NO_STORE

    def test_an_ordinary_photos_source_is_cacheable(self, media):
        """The cacheability half of the same route — `/source-file` is on the
        allowlist, so an unsecured source must keep its handler header."""
        r = media["admin"].get(f"/api/photos/{media['converted_id']}/source-file")
        assert _cache_control(r) == MEDIA_CACHE_1D
