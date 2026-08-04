"""
Test 91: #49 video ladder — rendition serving and HTTP Range requests.

`todo.md` (B4) records exactly what was missing: "The ladder arithmetic and the
picker default-per-network-state are unit-tested; the *serving* path with a real
`Range` header is not."

That gap is not cosmetic. `serve_photo` implements `?rendition=<short_edge>` by
**swapping the rung's locator into the local variables and letting every branch
below run unchanged** (`server/src/photos/serve.rs`), so renditions inherit Range
support, chunked streaming and conditional requests rather than reimplementing
them. That is only a saving if the inherited branches really do produce:

  1. the rung's **bytes** — a fallback to the original "works" while defeating the
     entire point of asking for 1080p;
  2. the rung's **length** — `Content-Range: bytes a-b/TOTAL` with the original's
     total makes every player's seek bar wrong and its final range request a 416;
  3. the rung's **cache identity** — `etag_id` is `{photo_id}.r{short_edge}`
     precisely so a client holding the 4K original is not handed a 304 for the
     1080p rung. A shared ETag serves the wrong quality out of the browser cache
     forever, and no unit test on the pure `rendition_serve_target` can see it.

None of the three is reachable from a unit test: the first needs a real encode,
the second needs a real `Range` header, and the third needs a real HTTP cache
handshake. Hence this module.

The fixture drives the actual pipeline — upload an oversized H.264 source, kick
`POST /api/admin/photos/auto-scan` (which awaits the conversion pass and *then*
calls `generate_rungs_after_scan`), and wait for `renditions` to appear on the
sync feed. It asserts the ladder produced something before any test runs, because
"no rungs" would otherwise make every assertion below pass vacuously — the trap
`todo.md` records this repo falling into repeatedly.
"""

import json
import os
import subprocess
import tempfile
import time

import pytest

from helpers import APIClient, random_username

# ── Environment guards ───────────────────────────────────────────────


def _ffmpeg_available() -> bool:
    try:
        subprocess.run(["ffmpeg", "-version"], capture_output=True, timeout=5)
        subprocess.run(["ffprobe", "-version"], capture_output=True, timeout=5)
        return True
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


pytestmark = pytest.mark.skipif(
    not _ffmpeg_available(),
    reason="ffmpeg/ffprobe not installed — the ladder cannot produce a rung without them",
)

USER_PASSWORD = "E2eUserPass456!"

# The source shape. The short edge must clear `ladder::rung_threshold(1080)` =
# 1188, so 1440 is the smallest round number that earns a 1080p rung; 1080 or
# 1088 would sit inside `TIER_TOLERANCE` and correctly produce no ladder at all.
SOURCE_WIDTH = 2560
SOURCE_HEIGHT = 1440
RUNG_SHORT_EDGE = 1080
RUNG_WIDTH = 1920
RUNG_HEIGHT = 1080

# The sweep runs behind a scan + a full conversion pass on a shared session
# server, and the encode itself is real. Generous, and a failure here is a real
# finding rather than flake — see `_wait_for_rung`.
RUNG_TIMEOUT_SECS = 240.0


def _generate_oversized_h264(width: int, height: int, seconds: float = 2.0) -> bytes:
    """A genuinely browser-native H.264/yuv420p source above the 1080p tier.

    `testsrc2` rather than a flat colour: a static frame compresses to almost
    nothing, and a rendition of a few hundred bytes makes the Range assertions
    below meaningless (there is nothing to slice).
    """
    fd, path = tempfile.mkstemp(suffix=".mp4")
    os.close(fd)
    try:
        subprocess.run(
            [
                "ffmpeg", "-y",
                "-f", "lavfi", "-i", f"testsrc2=s={width}x{height}:r=15:d={seconds}",
                "-c:v", "libx264", "-preset", "ultrafast", "-profile:v", "high",
                "-pix_fmt", "yuv420p",
                "-movflags", "+faststart",
                path,
            ],
            capture_output=True, timeout=120, check=True,
        )
        with open(path, "rb") as fh:
            return fh.read()
    finally:
        if os.path.exists(path):
            os.unlink(path)


def _probe_dimensions(data: bytes) -> tuple:
    """(width, height) of the first video stream in `data`.

    Written to a temp file rather than piped: ffprobe on a pipe cannot seek, and
    a `+faststart` MP4 read from stdin reports nothing useful on some builds.
    """
    fd, path = tempfile.mkstemp(suffix=".mp4")
    os.close(fd)
    try:
        with open(path, "wb") as fh:
            fh.write(data)
        out = subprocess.run(
            [
                "ffprobe", "-v", "error",
                "-select_streams", "v:0",
                "-show_entries", "stream=width,height",
                "-of", "json", path,
            ],
            capture_output=True, timeout=60, check=True, text=True,
        ).stdout
        streams = json.loads(out).get("streams", [])
        assert streams, "ffprobe found no video stream in the served bytes"
        return streams[0]["width"], streams[0]["height"]
    finally:
        if os.path.exists(path):
            os.unlink(path)


def _sync_row(client: APIClient, photo_id: str):
    """The photo's row on the sync feed — the only place `renditions` is published.

    One page is enough and deliberately so: `encrypted-sync` is user-scoped and
    this module's fixture owns a freshly created user with two photos.
    """
    for p in client.encrypted_sync(limit=500).get("photos", []):
        if p["id"] == photo_id:
            return p
    return None


def _wait_for_rung(client: APIClient, admin: APIClient, photo_id: str) -> list:
    """Drive the pipeline until the ladder has recorded a rung for `photo_id`.

    The autoscan trigger is re-sent on each poll rather than once: the sweep
    stands down entirely (`should_defer_sweep`) while an encryption migration or
    a conversion pass is running, and on a shared session server another module's
    upload can be holding either. One trigger is a race; re-triggering is how the
    hourly production cadence behaves anyway.
    """
    deadline = time.time() + RUNG_TIMEOUT_SECS
    last_seen = None
    while time.time() < deadline:
        row = _sync_row(client, photo_id)
        if row is not None:
            last_seen = row
            rungs = row.get("renditions") or []
            if any(r.get("short_edge") == RUNG_SHORT_EDGE for r in rungs):
                return rungs
        try:
            admin.admin_trigger_autoscan()
        except Exception:
            pass
        time.sleep(3)

    pytest.fail(
        "the ladder never produced a 1080p rung for a "
        f"{SOURCE_WIDTH}x{SOURCE_HEIGHT} H.264 source within {RUNG_TIMEOUT_SECS:.0f}s. "
        "This is the #49 sweep failing, not a fixture problem: check the server log "
        "for `[LADDER]`. Last sync row: "
        f"{ {k: last_seen.get(k) for k in ('width', 'height', 'encrypted_blob_id', 'renditions')} if last_seen else None }"
    )


# ── Fixture ──────────────────────────────────────────────────────────


@pytest.fixture(scope="module")
def ladder(primary_server, primary_admin):
    """An uploaded oversized video that has actually been through the sweep.

    Module-scoped: producing it costs a real 1440p encode plus a scan, and every
    test below reads the same rung.
    """
    client = APIClient(primary_server.base_url)
    username = random_username("ladder_")
    primary_admin.admin_create_user(username, USER_PASSWORD, role="user")
    client.login(username, USER_PASSWORD)

    source = _generate_oversized_h264(SOURCE_WIDTH, SOURCE_HEIGHT)
    upload = client.upload_photo(
        filename="ladder_source.mp4",
        content=source,
        mime_type="video/mp4",
    )
    photo_id = upload["photo_id"]

    rungs = _wait_for_rung(client, primary_admin, photo_id)

    # The vacuity guard. Everything below asserts something *about* the rung, so
    # an empty or degenerate ladder must stop the module here rather than let a
    # dozen assertions pass against nothing.
    rung = next(r for r in rungs if r["short_edge"] == RUNG_SHORT_EDGE)
    assert rung["size_bytes"] > 0, f"the 1080p rung recorded no bytes: {rung}"
    assert (rung["width"], rung["height"]) == (RUNG_WIDTH, RUNG_HEIGHT), (
        f"ladder::rung_dimensions should turn {SOURCE_WIDTH}x{SOURCE_HEIGHT} into "
        f"{RUNG_WIDTH}x{RUNG_HEIGHT}, got {rung['width']}x{rung['height']}"
    )
    assert not rung["is_source"], "the 1080p downscale must not be flagged as the source"

    return {
        "client": client,
        "photo_id": photo_id,
        "rungs": rungs,
        "rung": rung,
        "source_bytes": source,
    }


def _get(ladder, *, rendition=None, headers=None):
    params = {"rendition": rendition} if rendition is not None else None
    return ladder["client"].get(
        f"/api/photos/{ladder['photo_id']}/file", params=params, headers=headers
    )


# =====================================================================
# 1. The rung's BYTES — not a silent fallback to the original
# =====================================================================


class TestRenditionServesTheRung:
    def test_rendition_serves_the_downscale_not_the_original(self, ladder):
        """`?rendition=1080` must return 1920x1080 pixels.

        The strongest available statement of "the rung, not the original": it is
        decided by decoding the served bytes, so it survives any refactor of how
        the locator is chosen. A fallback to the original would return
        2560x1440 here and 200 OK everywhere else.
        """
        r = _get(ladder, rendition=RUNG_SHORT_EDGE)
        assert r.status_code == 200, r.text[:400]
        assert _probe_dimensions(r.content) == (RUNG_WIDTH, RUNG_HEIGHT)

    def test_the_original_is_untouched_by_the_ladder(self, ladder):
        """No selector = the original, at full resolution.

        The counterpart to the test above: the two must not have converged on
        one file. Without this, a bug that served the *rung* for every request
        would pass the whole rest of the module.
        """
        r = _get(ladder)
        assert r.status_code == 200, r.text[:400]
        assert _probe_dimensions(r.content) == (SOURCE_WIDTH, SOURCE_HEIGHT)

    def test_rendition_is_served_as_video_mp4(self, ladder):
        """The ladder always encodes H.264 in MP4, whatever the source container.

        `ServeTarget.content_type` exists for this: 10 videos in the live library
        are `.mov`, and serving their downscale under the source's
        `video/quicktime` hands the player bytes that do not match the type it
        was promised.
        """
        r = _get(ladder, rendition=RUNG_SHORT_EDGE)
        assert r.headers["Content-Type"] == "video/mp4"


class TestVideoIsNotCompressedOnTheWire:
    """Found by this module, in a surface the B4 item never named.

    Every video response — original *and* rung — was being gzipped by the global
    `CompressionLayer`. `DefaultPredicate` declines `image/*` and nothing else,
    so `video/mp4` sailed through it while the JPEG next to it did not.

    The wasted CPU is the boring half. The functional half is that a compressed
    body is a *transformed* body, so the layer drops `Content-Length` and
    `Accept-Ranges: bytes` and switches to `Transfer-Encoding: chunked` —
    deleting the two headers `serve_photo` sets specifically to advertise
    seeking, on the serving path of a feature whose entire purpose is swapping
    quality mid-playback.

    Unreachable from a unit test by construction: the middleware only exists in
    the assembled router, and the header loss happens on the wire.
    """

    def test_a_video_rendition_is_not_gzipped(self, ladder):
        r = _get(ladder, rendition=RUNG_SHORT_EDGE)
        assert r.headers.get("Content-Encoding", "identity") == "identity", (
            "the 1080p rung came back gzipped; H.264 is already entropy-coded, "
            "so this is pure CPU for no saving — and it costs the two headers "
            "asserted below"
        )

    def test_a_video_rendition_advertises_seek_support(self, ladder):
        r = _get(ladder, rendition=RUNG_SHORT_EDGE)
        assert r.headers.get("Accept-Ranges") == "bytes", (
            "serve_photo sets Accept-Ranges: bytes; if it is missing here the "
            "compression layer stripped it and the client is told the video "
            "cannot be seeked"
        )
        assert r.headers.get("Content-Length") == str(len(r.content))

    def test_the_original_video_is_not_gzipped_either(self, ladder):
        """The same defect, on the path that carries multi-GB 4K originals.

        Worth its own case rather than folding into the rung: the rung is a few
        megabytes, the original is what makes the wasted compression expensive
        on a real library.
        """
        r = _get(ladder)
        assert r.headers.get("Content-Encoding", "identity") == "identity"
        assert r.headers.get("Accept-Ranges") == "bytes"

    def test_json_responses_are_still_compressed(self, ladder):
        """The fix must not have been bought by disabling compression outright.

        `/api/photos` is text, repetitive, and the reason the layer is mounted.
        Without this assertion, `compress_when(|_| false)` passes everything
        above.
        """
        r = ladder["client"].get("/api/photos")
        assert r.status_code == 200
        # Either encoding is correct — the layer negotiates from Accept-Encoding,
        # and `requests` offers br only when a brotli backend is installed.
        # Pinning "gzip" alone would fail on a machine that has one.
        assert r.headers.get("Content-Encoding") in ("gzip", "br"), (
            "JSON must still be compressed — the media bypass is by content "
            f"type, not a blanket opt-out (got {r.headers.get('Content-Encoding')!r})"
        )


# =====================================================================
# 3. The rung's LENGTH — Range requests
# =====================================================================


class TestRenditionRangeRequests:
    """The half `todo.md` names explicitly: a real `Range` header.

    Every assertion here is about the *total* in `Content-Range`, because that
    is the field a locator swap gets wrong: the rung's bytes with the original's
    length is exactly what "swap the locator, inherit the branch" would produce
    if `size_bytes` were not swapped with it.
    """

    def test_a_range_returns_206_with_the_rungs_own_total(self, ladder):
        full = _get(ladder, rendition=RUNG_SHORT_EDGE)
        assert full.status_code == 200
        total = len(full.content)

        original_total = len(_get(ladder).content)
        assert total != original_total, (
            "precondition: the rung and the original must differ in length, or "
            f"the total below proves nothing (both {total})"
        )

        r = _get(ladder, rendition=RUNG_SHORT_EDGE, headers={"Range": "bytes=64-1087"})
        assert r.status_code == 206, r.text[:400]
        assert r.headers["Content-Range"] == f"bytes 64-1087/{total}", (
            "Content-Range must report the RENDITION's length. The original's "
            f"length is {original_total}; reporting that makes every player's "
            "seek bar wrong and its last range request a 416."
        )
        assert len(r.content) == 1024
        assert r.content == full.content[64:1088], (
            "the partial bytes must be the rung's own, at the requested offset"
        )

    def test_an_open_ended_range_reaches_the_true_end_of_the_rung(self, ladder):
        """`bytes=N-` is what a player issues to finish a file.

        Served against the original's length it either over-reads or truncates,
        and the truncation is silent — the player simply reports a corrupt tail.
        """
        full = _get(ladder, rendition=RUNG_SHORT_EDGE)
        total = len(full.content)
        start = total - 512

        r = _get(ladder, rendition=RUNG_SHORT_EDGE, headers={"Range": f"bytes={start}-"})
        assert r.status_code == 206, r.text[:400]
        assert r.headers["Content-Range"] == f"bytes {start}-{total - 1}/{total}"
        assert r.content == full.content[start:]

    def test_a_range_past_the_end_of_the_rung_is_416(self, ladder):
        """The boundary that distinguishes the two lengths most sharply.

        The rung is smaller than the original, so a request starting at the
        rung's own length is unsatisfiable — but would be perfectly satisfiable
        against the original's. A 206 here means the handler is measuring the
        wrong file.
        """
        total = len(_get(ladder, rendition=RUNG_SHORT_EDGE).content)
        r = _get(ladder, rendition=RUNG_SHORT_EDGE, headers={"Range": f"bytes={total}-"})
        assert r.status_code == 416, (
            f"expected 416 for a range starting at the rung's length ({total}), "
            f"got {r.status_code}"
        )

    def test_ranges_reassemble_into_the_whole_rung(self, ladder):
        """Chunked-blob seeking decrypts only the overlapping frames.

        A frame-boundary error is invisible in a single mid-file range (the
        slice still looks like bytes) and shows up only when the pieces are
        reassembled — which is what a player actually does.
        """
        full = _get(ladder, rendition=RUNG_SHORT_EDGE).content
        total = len(full)
        step = max(total // 4, 1)

        assembled = b""
        pos = 0
        while pos < total:
            end = min(pos + step, total) - 1
            r = _get(
                ladder,
                rendition=RUNG_SHORT_EDGE,
                headers={"Range": f"bytes={pos}-{end}"},
            )
            assert r.status_code == 206, f"range {pos}-{end}: {r.status_code}"
            assembled += r.content
            pos = end + 1

        assert assembled == full, "ranges did not reassemble into the rung"
        assert _probe_dimensions(assembled) == (RUNG_WIDTH, RUNG_HEIGHT)


# =====================================================================
# 4. The rung's CACHE IDENTITY
# =====================================================================


class TestRenditionCacheIdentity:
    """`etag_id = "{photo_id}.r{short_edge}"` — the reason it is not `photo_id`.

    A shared ETag is the worst of the three failure modes here because it is
    *persistent*: the browser keeps serving the wrong quality from its own cache
    long after the server is fixed.
    """

    def test_the_rung_and_the_original_do_not_share_an_etag(self, ladder):
        original = _get(ladder).headers["ETag"]
        rung = _get(ladder, rendition=RUNG_SHORT_EDGE).headers["ETag"]
        assert original and rung
        assert original != rung, (
            f"the original and the 1080p rung both claim ETag {original}"
        )

    def test_the_originals_etag_does_not_validate_the_rung(self, ladder):
        """The cache handshake, which is where a shared ETag actually bites.

        A client that has the 4K original cached sends its ETag when it asks for
        the 1080p rung. Answering 304 tells it to play the 4K bytes it already
        has — the exact fallback the handler documents itself as refusing.
        """
        original_etag = _get(ladder).headers["ETag"]
        r = _get(
            ladder,
            rendition=RUNG_SHORT_EDGE,
            headers={"If-None-Match": original_etag},
        )
        assert r.status_code == 200, (
            "the original's ETag must not validate the rung — a 304 here hands "
            "the client 4K bytes it asked to avoid"
        )
        assert _probe_dimensions(r.content) == (RUNG_WIDTH, RUNG_HEIGHT)

    def test_the_rungs_own_etag_still_validates(self, ladder):
        """...and the distinction must not have been bought by breaking caching.

        Without this, `etag_id = uuid()` would pass the test above.
        """
        etag = _get(ladder, rendition=RUNG_SHORT_EDGE).headers["ETag"]
        r = _get(
            ladder, rendition=RUNG_SHORT_EDGE, headers={"If-None-Match": etag}
        )
        assert r.status_code == 304, f"expected 304 for a matching ETag, got {r.status_code}"


# =====================================================================
# 5. An unoffered rung is a 404, never a fallback
# =====================================================================


class TestUnknownRungIsNotAFallback:
    def test_a_rung_that_does_not_exist_is_404(self, ladder):
        """`serve_photo` documents this: "An unknown or unproduced rung is a 404,
        never a silent fallback to the original."

        720 is the realistic case, not a nonsense number — it is a tier a future
        ladder could add and a stale client could already be asking for.
        """
        for short_edge in (720, 4320, 0, -1):
            r = _get(ladder, rendition=short_edge)
            assert r.status_code == 404, (
                f"?rendition={short_edge} returned {r.status_code}; a client that "
                "asked for a quality it was never offered must not be handed the "
                "original as if it were that quality"
            )

    def test_a_still_image_has_no_rungs_to_ask_for(self, ladder):
        """The degenerate case, and the majority of any library.

        Stills never gain `video_renditions` rows, so every selector must 404 —
        including the one that happens to be a real tier for videos.
        """
        from helpers import generate_test_jpeg

        client = ladder["client"]
        upload = client.upload_photo(
            filename="not_a_video.jpg",
            content=generate_test_jpeg(width=64, height=64),
            mime_type="image/jpeg",
        )
        still_id = upload["photo_id"]

        r = client.get(
            f"/api/photos/{still_id}/file", params={"rendition": RUNG_SHORT_EDGE}
        )
        assert r.status_code == 404, (
            f"a still returned {r.status_code} for ?rendition={RUNG_SHORT_EDGE}"
        )
        # ...but the still itself still serves, so the 404 above is about the
        # selector and not about the photo being unreachable.
        assert client.get(f"/api/photos/{still_id}/file").status_code == 200


# =====================================================================
# 6. The DTO and the serving path agree
# =====================================================================


class TestLadderDtoMatchesWhatIsServed:
    """`RenditionDto.size_bytes` is what a client uses to decide whether the
    swap is worth it on a metered connection. If it disagrees with the bytes the
    server will actually send, the picker is advertising a file that does not
    exist.
    """

    def test_advertised_size_matches_the_served_length(self, ladder):
        served = len(_get(ladder, rendition=RUNG_SHORT_EDGE).content)
        assert ladder["rung"]["size_bytes"] == served, (
            f"the picker advertises {ladder['rung']['size_bytes']} bytes; the "
            f"server sends {served}"
        )

    def test_every_advertised_rung_is_actually_servable(self, ladder):
        """`list_renditions` filters unproduced rows out precisely so a picker
        never offers a quality that 404s. This asserts the two ends agree.
        """
        rungs = ladder["rungs"]
        assert rungs, "vacuity guard: there must be at least one rung to check"
        for rung in rungs:
            r = _get(ladder, rendition=rung["short_edge"])
            assert r.status_code == 200, (
                f"rung {rung['short_edge']} is offered by the API but serves "
                f"{r.status_code}"
            )
            assert _probe_dimensions(r.content) == (rung["width"], rung["height"]), (
                f"rung {rung['short_edge']} advertises "
                f"{rung['width']}x{rung['height']} and served something else"
            )
