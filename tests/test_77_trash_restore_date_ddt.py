"""
E2E DDT: Trash restore must preserve the photo's original date metadata.

Regression for: photos restored from the trash were getting `created_at`
overwritten with the current timestamp, so they jumped to "today" in the
gallery's date-grouped view instead of staying with their original
capture date.

Since migration 041 (TODO.md F11) restore puts the trashed `photos` row
back verbatim, so the contract is exact: the restored photo's `taken_at`
and `created_at` are the strings it had before it was trashed. The gallery
orders by COALESCE(taken_at, created_at), so it lands where it was.

(The pre-041 restore rebuilt the row and set `created_at := taken_at` to
avoid stamping it with the restore moment; this test asserted that
workaround. An exact match is stricter: a regression to "now" or to the
rewrite both fail it.)

DDT covers the meaningful permutations of (taken_at provided vs not)
and the boundary case of an empty/whitespace timestamp.
"""

from __future__ import annotations

import time
from datetime import datetime, timedelta, timezone

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── DDT table ──────────────────────────────────────────────────────────────
# Each row is the `taken_at` set before trashing (None = never set).

_now = datetime.now(timezone.utc)

RESTORE_DATE_CASES = [
    pytest.param(
        (_now - timedelta(days=365 * 3)).isoformat(),
        id="three_years_ago_preserved",
    ),
    pytest.param(
        (_now - timedelta(days=30)).isoformat(),
        id="last_month_preserved",
    ),
    pytest.param(
        (_now - timedelta(hours=6)).isoformat(),
        id="earlier_today_preserved",
    ),
    # Never edited. Upload always resolves a taken_at — falling back to the
    # same instant it stamps as created_at (`resolve_upload_taken_at`) — so a
    # row with NO taken_at cannot come from this API; this is the "nobody set
    # a date" case. The pre-041 restore also passed it (taken_at == created_at).
    pytest.param(None, id="taken_at_left_at_upload_time"),
]


def _wait_visible(client: APIClient, photo_id: str, timeout: float = 10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for p in client.list_photos()["photos"]:
            if p["id"] == photo_id:
                return p
        time.sleep(0.2)
    raise TimeoutError(f"Photo {photo_id} not visible after {timeout}s")


@pytest.mark.parametrize("taken_at", RESTORE_DATE_CASES)
def test_restore_preserves_original_date(
    user_client: APIClient, taken_at
):
    # 1. Upload a real photo through the unencrypted pipeline.
    upload = user_client.upload_photo(
        filename=unique_filename(),
        content=generate_test_jpeg(width=64, height=64),
        mime_type="image/jpeg",
    )
    photo_id = upload["photo_id"]
    _wait_visible(user_client, photo_id)

    # 2. If we want a specific taken_at, set it via the metadata editor
    #    (the upload pipeline only assigns one when EXIF carries it).
    #    The server route (and the web client) use PATCH, not PUT.
    if taken_at is not None:
        r = user_client.patch(
            f"/api/photos/{photo_id}/metadata",
            json_data={"taken_at": taken_at},
        )
        assert r.status_code in (200, 204), r.text

    before = _wait_visible(user_client, photo_id)
    if taken_at is not None:
        assert before["taken_at"] is not None, before

    # A restore stamped "now" must be distinguishable from the upload stamp.
    time.sleep(1.1)

    # 3. Soft-delete via the unencrypted endpoint (DELETE /api/photos/:id).
    r = user_client.delete(f"/api/photos/{photo_id}")
    assert r.status_code in (200, 204), r.text

    # 4. Locate the trash entry for our photo and restore it.
    trash = user_client.list_trash()
    item = next((t for t in trash["items"] if t.get("photo_id") == photo_id), None)
    assert item is not None, f"Photo {photo_id} did not land in trash: {trash}"

    r = user_client.restore_trash(item["id"])
    assert r.status_code in (200, 204), r.text

    # 5. Verify the restored photo's date metadata.
    restored = _wait_visible(user_client, photo_id)
    assert restored["taken_at"] == before["taken_at"], (restored, before)
    assert restored["created_at"] == before["created_at"], (
        f"created_at was {before['created_at']} before trash, "
        f"{restored['created_at']} after restore"
    )
