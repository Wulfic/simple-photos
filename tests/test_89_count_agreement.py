"""
Test 89: Count agreement — the summary, a full sync walk, and the badge agree.

This is the E2E half of #42. Three definitions of "how many items are in this
library" had silently diverged, and two independent defects kept them apart:

  1. The keyset cursor was derived from the PEEKED (limit+1)-th row, which the
     next page's strict predicate then excluded — so exactly one photo vanished
     per page boundary, on every client, forever. On the live box that was 29
     photos over a 14,874-row library.
  2. Clients counted their own truncated local mirror in preference to the
     server's aggregate.

Neither had a test. The Rust unit tests in `gallery/sync.rs` now cover the
cursor arithmetic in isolation; this file asserts the property end-to-end,
over HTTP, against a real server — because the unit test cannot catch a
regression in the handler, the query, or the response shape.

The load-bearing trick is **small page limits**. At the production limit of
500 a library of 12 photos has zero page boundaries and the off-by-one is
invisible; at limit=1 the same library has 11 of them.
"""

import pytest

from helpers import generate_test_jpeg, unique_filename


# Enough rows that small limits produce many page boundaries, small enough to
# stay fast. At limit=1 this is 11 chances to drop a row.
PHOTO_COUNT = 12


@pytest.fixture
def counted_library(user_client):
    """A fresh user's library with PHOTO_COUNT distinct photos.

    Dimensions vary per photo so every upload has a distinct content hash:
    `generate_test_jpeg` is deterministic, so uploading the same bytes twice
    dedups to a single row server-side and the test would silently assert
    almost nothing.
    """
    ids = []
    for i in range(PHOTO_COUNT):
        data = user_client.upload_photo(
            unique_filename("jpg"),
            generate_test_jpeg(width=2 + i, height=2),
        )
        ids.append(data["photo_id"])

    assert len(set(ids)) == PHOTO_COUNT, (
        "uploads deduplicated — the fixture is not producing distinct content, "
        "so every assertion below would be vacuous"
    )
    return user_client, ids


class TestCountAgreement:
    """The summary, the sync walk, and the smart-album badges must agree."""

    def test_summary_total_matches_upload_count(self, counted_library):
        client, ids = counted_library
        summary = client.photos_summary()

        assert summary["total"] == PHOTO_COUNT
        # No bursts in this fixture, so every row is its own tile.
        assert summary["collapsed_total"] == PHOTO_COUNT
        assert summary["smart_photos"] == PHOTO_COUNT

    @pytest.mark.parametrize("limit", [1, 2, 3, 5, PHOTO_COUNT - 1, PHOTO_COUNT, PHOTO_COUNT + 1])
    def test_full_pagination_returns_every_row_exactly_once(self, counted_library, limit):
        """The #42 regression, at every interesting page boundary.

        Parameterised over limits that divide the library evenly, unevenly,
        exactly, and not at all — the even-division case has a boundary after
        the final page and was its own distinct way to lose a row.
        """
        client, ids = counted_library
        records = client.encrypted_sync_all(limit=limit)
        got = [r["id"] for r in records]

        missing = set(ids) - set(got)
        assert not missing, (
            f"limit={limit}: rows were never returned by ANY page: {sorted(missing)} "
            "— this is the keyset cursor off-by-one (#42)"
        )
        assert len(got) == len(set(got)), (
            f"limit={limit}: a row was returned by more than one page: "
            f"{len(got)} records for {len(set(got))} distinct ids"
        )
        assert len(got) == PHOTO_COUNT

    def test_pagination_agrees_with_the_summary(self, counted_library):
        """The badge and the grid must not be able to disagree.

        The badge reads `/photos/summary`; the grid is populated by walking
        `encrypted-sync`. If those two ever diverge the user sees a count that
        does not match what they can scroll to — which is the literal bug report.
        """
        client, _ = counted_library
        summary = client.photos_summary()
        walked = client.encrypted_sync_all(limit=3)

        assert len(walked) == summary["total"], (
            f"summary says {summary['total']} rows, a full sync walk returns "
            f"{len(walked)} — badge and grid disagree"
        )

    def test_page_size_does_not_change_the_result(self, counted_library):
        """Whatever the client picks for `limit` must not alter what it sees.

        A defect that depends on page size is exactly how this survived: the
        production limit of 500 hid it on every library smaller than that.
        """
        client, _ = counted_library
        by_limit = {
            limit: sorted(r["id"] for r in client.encrypted_sync_all(limit=limit))
            for limit in (1, 4, 500)
        }

        assert by_limit[1] == by_limit[4] == by_limit[500], (
            "the set of returned photos depends on the page size: "
            + ", ".join(f"limit={k} -> {len(v)} rows" for k, v in by_limit.items())
        )

    def test_favorites_count_tracks_the_summary(self, counted_library):
        """A write must move the summary, not just the rows.

        The summary is TTL-cached server-side; if a favourite toggle does not
        invalidate it, badges go stale in a way that looks exactly like the
        original count bug.
        """
        client, ids = counted_library
        assert client.photos_summary()["smart_favorites"] == 0

        client.favorite_photo(ids[0])

        summary = _eventually(
            lambda: client.photos_summary(),
            lambda s: s["smart_favorites"] == 1,
        )
        assert summary["smart_favorites"] == 1
        assert summary["favorites"] == 1
        # Favouriting must not change how many items exist.
        assert summary["total"] == PHOTO_COUNT


def _eventually(fetch, predicate, attempts: int = 20, delay: float = 0.5):
    """Poll `fetch` until `predicate` holds, then return the value.

    The summary is TTL-cached (15 s by default), so a freshly-written change is
    allowed to take a moment to surface — but not forever. Returns the last
    value either way so the caller's assertion produces a useful diff.
    """
    import time

    value = None
    for _ in range(attempts):
        value = fetch()
        if predicate(value):
            return value
        time.sleep(delay)
    return value
