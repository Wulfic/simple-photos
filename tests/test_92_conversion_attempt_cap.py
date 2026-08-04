"""
Test 92: no file is transcoded forever (#40, the 3-strike cap's E2E half)

`todo.md` asked for one thing here: "a fixture that always fails must be
attempted **exactly 3 times across 5 real scan passes**". **That test cannot be
built, and the measurement that proves it is below.** What is built instead is
the property the cap actually exists to deliver — *no file is transcoded on
every pass forever* — driven through five real autoscan passes for both paths
that reach it.

── Why "3 times across 5 passes" is unbuildable ─────────────────────────────

Measured on this suite's own server, 2026-08-04, garbage `.mkv` in the storage
root, five `POST /api/admin/photos/auto-scan` passes:

    pass 1..5: media_convert_failure=1  conversion_retired=0  photos row=1

**One attempt, not three, and the cap never fired.** `process_candidate`'s
failure arm registers the ORIGINAL to avoid data loss
([ingest.rs](../server/src/ingest.rs)), so the file lands in `photos.file_path`
and every later pass skips it via `existing_set` before the skip cache is even
consulted. A strike is charged once and nothing ever spends the other two.

Every other failing route is terminal in one pass too: the hash-dedup arms on
both the success and failure sides record a terminal `hash_duplicate`
deliberately (spending three transcodes to re-derive a deterministic answer
would be its own bug, and it would retire the file citing a reason that is not
true). That leaves the cap with exactly two live consumers — the DB-error path
at `ingest.rs`'s "Failed to register converted photo", and a pass interrupted
between the charge and the registration. **Neither is reachable from an E2E
without a fault-injection seam in the server**, and a knob that makes a release
binary drop photos on demand is a worse thing to own than an untested branch.
The cap's arithmetic and SQL stay pinned by the unit + DB tests
(`photos/scan_skip.rs`, `photos/register.rs`); this file pins the outcome.

**Do not "fix" this file by asserting a count of 3.** It went red for exactly
that reason once already.

── The vacuity traps, because there are two and they point opposite ways ─────

1. "Assert it is attempted only once" passes for the WRONG REASON on the
   registered path — it stops because a `photos` row exists, not because
   anything capped it. `test_it_stops_because_it_registered_...` asserts the row
   is there, so a future change that stops registering failures cannot leave
   this file green.
2. The no-row path is the one B2's correction block calls the most expensive
   loop in the issue (the Takeout library: same bytes in the date folder and in
   every album folder, so every album copy was fully transcoded and the output
   discarded, on every pass, forever). A test of it is vacuous unless it first
   proves the row really is absent — otherwise it is just `existing_set` again
   wearing a different hat. `test_the_duplicate_leaves_no_photos_row` is that
   precondition and it runs before the count assertions.
"""

import json
import os

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    unique_filename,
    _ffmpeg_available,
)


pytestmark = pytest.mark.skipif(
    not _ffmpeg_available(),
    reason="ffmpeg not installed — a conversion failure cannot be provoked",
)

#: Enough passes to tell "capped" from "loops forever" without making the
#: fixture slow. Each pass is a full walk + transcode attempt of the fixture.
SCAN_PASSES = 5


def _audit(admin: APIClient, event_type: str, needle: str) -> list:
    """Audit rows of `event_type` whose details mention `needle`.

    Filenames are `unique_filename()`-generated, so the needle cannot collide
    with another test's fixture on this session-scoped server.
    """
    r = admin.get(
        "/api/admin/audit-logs", params={"event_type": event_type, "limit": 300}
    )
    assert r.status_code == 200, f"audit-logs returned {r.status_code}: {r.text[:300]}"
    return [e for e in r.json()["logs"] if needle in e.get("details", "")]


def _photo_rows(admin: APIClient, filename: str) -> list:
    r = admin.get("/api/photos", params={"limit": 500})
    assert r.status_code == 200, f"/api/photos returned {r.status_code}"
    return [p for p in r.json().get("photos", []) if p.get("filename") == filename]


def _scan(admin: APIClient) -> None:
    """One real autoscan pass, awaited to completion.

    `POST /api/admin/photos/auto-scan` awaits the conversion pass itself, but
    the audit rows are written by `log_background` (a fire-and-forget spawn),
    so the conversion-status poll is kept as the settling point.
    """
    admin.admin_trigger_autoscan()
    admin.wait_for_conversion(timeout=90)


@pytest.fixture(scope="module")
def unconvertible_pair(primary_admin: APIClient, primary_server) -> dict:
    """Drive both paths to the cap, once, and hand back the observed state.

    Ordering is deliberate and NOT a coincidence of the walk: `registered` is
    planted and scanned **alone** first, so it is the copy that wins the
    hash-dedup race and registers. Planting both in one pass leaves which copy
    registers up to directory-iteration order, and the two assertions below
    want to name a specific file.

    The bytes are random, and `conversion_target()` dispatches on extension
    with no magic-byte gate ahead of it, so a `.mkv` full of noise reaches
    ffmpeg and fails there rather than being rejected at the door.
    """
    payload = generate_random_bytes(4096)
    registered = unique_filename("mkv")
    duplicate = unique_filename("mkv")

    with open(os.path.join(primary_server.storage_root, registered), "wb") as f:
        f.write(payload)
    _scan(primary_admin)

    # Same bytes, different path — the Takeout shape.
    with open(os.path.join(primary_server.storage_root, duplicate), "wb") as f:
        f.write(payload)
    for _ in range(SCAN_PASSES):
        _scan(primary_admin)

    return {"registered": registered, "duplicate": duplicate, "passes": SCAN_PASSES}


class TestTheFixtureReallyFails:
    """If the fixture converts, every count below is measuring nothing."""

    def test_the_first_copy_failed_conversion(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        name = unconvertible_pair["registered"]
        failures = _audit(primary_admin, "media_convert_failure", name)

        assert failures, (
            f"No media_convert_failure for {name}. Either ffmpeg now succeeds on "
            "random bytes or the file never entered the conversion queue — "
            "either way this whole file is asserting nothing."
        )
        details = json.loads(failures[0]["details"])
        assert details.get("error"), (
            f"Failure row carries no error text, so it is not actionable: {details}"
        )


class TestTheRegisteredOriginalIsNotRetranscoded:
    """A failing conversion registers its original and is then left alone."""

    def test_transcoded_once_across_five_passes(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        """The property #40 is about: not "3 attempts", but "not forever"."""
        name = unconvertible_pair["registered"]
        failures = _audit(primary_admin, "media_convert_failure", name)

        assert len(failures) == 1, (
            f"{name} was transcoded {len(failures)} times across "
            f"{unconvertible_pair['passes'] + 1} passes. One attempt is the whole "
            "point — more than one means the file is back in the forever-loop #40 "
            "closed."
        )

    def test_it_stops_because_it_registered_not_because_of_the_cap(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        """The vacuity guard for the test above.

        `existing_set` is what silences this file, and it is consulted before
        the skip cache. Asserting the count alone would stay green if the
        registration were dropped and the cap took over instead — a different
        mechanism, and one that loses the bytes the failure arm exists to keep
        (issue #1: "reported size lower than actual").
        """
        name = unconvertible_pair["registered"]

        rows = _photo_rows(primary_admin, name)
        assert len(rows) == 1, (
            f"Expected exactly one photos row for {name}, got {len(rows)}. The "
            "failure arm registers the ORIGINAL so the bytes are not lost; "
            "without that row the single-attempt count above proves nothing."
        )

        retired = _audit(primary_admin, "conversion_retired", name)
        assert not retired, (
            f"{name} registered successfully but was announced as retired: "
            f"{[e['details'] for e in retired]}. A file that is in the library "
            "must not be reported in the Server Logs tab as abandoned."
        )


class TestTheNoRowDuplicateIsNotRetranscoded:
    """The Takeout loop: same bytes, second path, no row to skip it next time."""

    def test_the_duplicate_leaves_no_photos_row(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        """Precondition, and the reason the next test is not vacuous.

        This is the only path in the walk that runs a transcode and leaves
        nothing in `photos`. If a row ever appears here, `existing_set` starts
        doing the work and the count below stops testing the skip cache at all.
        """
        name = unconvertible_pair["duplicate"]
        rows = _photo_rows(primary_admin, name)

        assert rows == [], (
            f"{name} registered a photos row ({len(rows)}), so it is no longer "
            "the no-row path this class exists to cover. The dedup arm is "
            "supposed to return without registering."
        )

    def test_transcoded_once_across_five_passes(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        """Terminal after one transcode, with no row and no cap involved."""
        name = unconvertible_pair["duplicate"]
        failures = _audit(primary_admin, "media_convert_failure", name)

        assert len(failures) == 1, (
            f"{name} was transcoded {len(failures)} times across "
            f"{unconvertible_pair['passes']} passes with no photos row to stop "
            "it. This is the loop B2 measured on the Takeout library — every "
            "album copy of the same bytes, fully transcoded, output discarded, "
            "on every pass forever."
        )

    def test_it_is_recorded_as_a_duplicate_not_as_a_retirement(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        """Terminal at ZERO strikes, deliberately — so it must not say "retired".

        The verdict is deterministic, so it is recorded as `hash_duplicate`
        rather than spending three transcodes to reach it three times. A
        `conversion_retired` row here would be the audit trail naming a reason
        that is not true, which is the #45 complaint restated.
        """
        name = unconvertible_pair["duplicate"]
        retired = _audit(primary_admin, "conversion_retired", name)

        assert not retired, (
            f"{name} is a deterministic duplicate but was announced as retired "
            f"after repeated failures: {[e['details'] for e in retired]}"
        )


class TestRetryFailedIsScopedToConversionFailures:
    """`retry-failed` re-admits strikes; it must not re-admit dead ends."""

    def test_a_hash_duplicate_survives_retry_failed(
        self, primary_admin: APIClient, unconvertible_pair: dict
    ):
        """POST /api/admin/conversion/retry-failed, then one more real pass.

        The endpoint deletes `conversion_failed` rows only. Widening it to every
        skip reason would re-admit the whole Takeout duplicate set and re-hash
        the library on the next pass — the disk thrash migration 031 removed.
        Nothing but an E2E can show that, because the scoping lives in a `WHERE`
        clause and the consequence lives one full scan pass later.
        """
        duplicate = unconvertible_pair["duplicate"]
        registered = unconvertible_pair["registered"]

        r = primary_admin.post("/api/admin/conversion/retry-failed")
        assert r.status_code == 200, f"retry-failed returned {r.status_code}"
        cleared = r.json()["cleared"]
        assert cleared >= 1, (
            "retry-failed cleared nothing, so the pass below cannot distinguish "
            "'scoped correctly' from 'did nothing at all'. Expected at least the "
            f"{registered!r} conversion_failed row."
        )

        _scan(primary_admin)

        assert len(_audit(primary_admin, "media_convert_failure", duplicate)) == 1, (
            f"retry-failed re-admitted the hash_duplicate row for {duplicate} — "
            "it is scoped to conversion_failed precisely so a Takeout library's "
            "duplicate set is not re-transcoded on the next pass."
        )
        assert _photo_rows(primary_admin, duplicate) == [], (
            f"{duplicate} registered after retry-failed; it is still a duplicate "
            "of a photo that is still in the library."
        )
