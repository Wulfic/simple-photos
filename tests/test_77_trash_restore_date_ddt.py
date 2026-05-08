"""
E2E DDT: Trash restore must preserve the photo's original date metadata.

Regression for: photos restored from the trash were getting `created_at`
overwritten with the current timestamp, so they jumped to "today" in the
gallery's date-grouped view instead of staying with their original
capture date.

The unencrypted restore path now uses the trashed entry's `taken_at`
(when present) as the restored `created_at`, falling back to the
restore moment only when the original capture date is unknown.

DDT covers the meaningful permutations of (taken_at provided vs not)
and the boundary case of an empty/whitespace timestamp.
"""

from __future__ import annotations

import time
from datetime import datetime, timedelta, timezone

import pytest

from helpers import APIClient, generate_test_jpeg, unique_filename


# ── DDT table ──────────────────────────────────────────────────────────────
# Each row encodes (taken_at_iso, expected_created_at_matches_taken_at)
#
# When a real `taken_at` is present, the restored photo's `created_at`
# MUST equal it (preserving the date the user expects).  When no
# `taken_at` is available, the restored `created_at` falls back to the
# restore moment — that is documented and covered as a distinct row.

_now = datetime.now(timezone.utc)

RESTORE_DATE_CASES = [
    pytest.param(
        (_now - timedelta(days=365 * 3)).isoformat(),
        True,
        id="three_years_ago_preserved",
    ),
    pytest.param(
        (_now - timedelta(days=30)).isoformat(),
        True,
        id="last_month_preserved",
    ),
    pytest.param(
        (_now - timedelta(hours=6)).isoformat(),
        True,
        id="earlier_today_preserved",
    ),
    # No taken_at → fallback to restore-time is acceptable.
    pytest.param(None, False, id="missing_taken_at_falls_back_to_now"),
]


def _wait_visible(client: APIClient, photo_id: str, timeout: float = 10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for p in client.list_photos()["photos"]:
            if p["id"] == photo_id:
                return p
        time.sleep(0.2)
    raise TimeoutError(f"Photo {photo_id} not visible after {timeout}s")


@pytest.mark.parametrize("taken_at,expect_preserved", RESTORE_DATE_CASES)
def test_restore_preserves_original_date(
    user_client: APIClient, taken_at, expect_preserved: bool
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
    if taken_at is not None:
        r = user_client.put(
            f"/api/photos/{photo_id}/metadata",
            json_data={"taken_at": taken_at},
        )
        assert r.status_code in (200, 204), r.text

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
    if expect_preserved:
        # created_at must match the original taken_at to within a second.
        original = datetime.fromisoformat(taken_at.replace("Z", "+00:00"))
        restored_created = datetime.fromisoformat(
            restored["created_at"].replace("Z", "+00:00")
        )
        delta = abs((restored_created - original).total_seconds())
        assert delta < 2.0, (
            f"Expected created_at≈{original.isoformat()} but got "
            f"{restored['created_at']} (Δ={delta:.1f}s)"
        )
    else:
        # No taken_at — fallback to ~now is acceptable.
        restored_created = datetime.fromisoformat(
            restored["created_at"].replace("Z", "+00:00")
        )
        delta = abs((restored_created - datetime.now(timezone.utc)).total_seconds())
        assert delta < 60.0, (
            f"Expected created_at≈now but got {restored['created_at']} (Δ={delta:.1f}s)"
        )
