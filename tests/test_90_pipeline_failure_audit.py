"""
Test 90: Pipeline failure auditing (#45)

Before this, the ingest/convert/encrypt failure paths emitted only
`tracing::warn!` — which goes to the process log, not the `audit_log` table
the Server Logs tab reads. The success path audited, the failure path did not,
so the one question a user actually asks ("*which file* failed?") was the one
question the UI could not answer.

Covers:
  - A failed conversion writes a `media_convert_failure` row to the audit log
  - That row is reachable through the admin logs endpoint (not just the DB)
  - Its details name the offending file and carry the error — "a conversion
    failed" alone is not actionable
  - `failures_only=true` filters server-side, so a failure buried under a pile
    of newer successes is still found
  - `failures_only=true` returns no success events
"""

import json
import time

import pytest
from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    unique_filename,
    _ffmpeg_available,
)


pytestmark = pytest.mark.skipif(
    not _ffmpeg_available(),
    reason="ffmpeg not installed — conversion failure cannot be provoked",
)


def _fetch_audit_logs(admin_client: APIClient, **params) -> list:
    """GET /api/admin/audit-logs with arbitrary query params."""
    r = admin_client.get("/api/admin/audit-logs", params=params)
    assert r.status_code == 200, f"audit-logs returned {r.status_code}: {r.text[:300]}"
    return r.json()["logs"]


def _wait_for_audit_event(admin_client: APIClient, event_type: str, needle: str,
                          timeout: float = 20.0) -> dict:
    """Poll the audit log until an event of `event_type` mentioning `needle` appears.

    The failure paths audit via `log_background` — a fire-and-forget spawn, so
    the row is not guaranteed to exist by the time the HTTP response lands.
    Polling is the honest way to wait for it; a bare sleep either flakes or
    wastes time.
    """
    deadline = time.time() + timeout
    last_seen = []
    while time.time() < deadline:
        logs = _fetch_audit_logs(admin_client, event_type=event_type, limit=100)
        last_seen = logs
        for entry in logs:
            if needle in entry.get("details", ""):
                return entry
        time.sleep(0.5)

    pytest.fail(
        f"No '{event_type}' audit row mentioning '{needle}' within {timeout}s. "
        f"Saw {len(last_seen)} rows of that type: "
        f"{[e.get('details', '')[:80] for e in last_seen[:5]]}"
    )


def _provoke_conversion_failure(user_client: APIClient) -> str:
    """Upload a file whose extension routes it to ffmpeg but whose bytes are junk.

    `conversion_target()` dispatches on extension alone, with no magic-byte
    gate ahead of it, so a `.mkv` full of random bytes reaches the transcoder
    and fails there rather than being rejected at the door.
    """
    filename = unique_filename("mkv")
    r = user_client.post(
        "/api/photos/upload",
        data=generate_random_bytes(4096),
        headers={
            **user_client._auth_headers(),
            "X-Filename": filename,
            "X-Mime-Type": "video/x-matroska",
            "Content-Type": "application/octet-stream",
        },
    )
    assert r.status_code >= 400, (
        f"Expected the corrupt .mkv upload to fail, got {r.status_code}. "
        "If conversion now succeeds on garbage input, this test is no longer "
        "provoking the path it claims to."
    )
    return filename


class TestConversionFailureIsAudited:
    """A failed conversion must leave a trail a human can actually read."""

    def test_failure_lands_in_audit_and_is_reachable_via_api(
        self, user_client: APIClient, admin_client: APIClient
    ):
        """Corrupt upload → media_convert_failure row, fetchable over HTTP."""
        filename = _provoke_conversion_failure(user_client)

        entry = _wait_for_audit_event(
            admin_client, "media_convert_failure", filename
        )

        assert entry["event_type"] == "media_convert_failure"

        details = json.loads(entry["details"])
        assert details["filename"] == filename, (
            "The audit row must name the file that failed — that is the whole "
            f"point of #45. Got: {details}"
        )
        assert details.get("error"), (
            f"Audit row carries no error text, so it is not actionable: {details}"
        )
        assert details.get("origin") == "upload", (
            f"Expected origin=upload for the upload path, got: {details}"
        )

    def test_failures_only_finds_a_failure_buried_under_newer_successes(
        self, user_client: APIClient, admin_client: APIClient
    ):
        """The filter is server-side, so paging cannot hide the failure.

        This is the regression that justifies filtering on the server: a
        client-side filter over the fetched page reports "no failures" whenever
        the newest N events happen to be something else, which is a worse
        answer than no filter at all because it looks authoritative.
        """
        filename = _provoke_conversion_failure(user_client)
        _wait_for_audit_event(admin_client, "media_convert_failure", filename)

        # Bury it under newer, unrelated audit activity.
        for _ in range(8):
            user_client.upload_photo(unique_filename("jpg"), generate_test_jpeg())

        # A small page of *unfiltered* recent events should now be dominated by
        # the successes above — this is the situation a client-side filter gets
        # wrong. Not asserted as a hard precondition (background tasks can also
        # log), it just sets up the contrast.
        recent = _fetch_audit_logs(admin_client, limit=5)
        assert len(recent) > 0

        # Server-side, with the same small page size, the failure is still found.
        failures = _fetch_audit_logs(admin_client, failures_only="true", limit=5)
        assert any(filename in e.get("details", "") for e in failures), (
            "failures_only=true did not surface the conversion failure with "
            f"limit=5. Returned: {[e['event_type'] for e in failures]}"
        )

    def test_failures_only_returns_no_success_events(
        self, user_client: APIClient, admin_client: APIClient
    ):
        """The filter must not leak successes into a failures-only view."""
        _provoke_conversion_failure(user_client)
        user_client.upload_photo(unique_filename("jpg"), generate_test_jpeg())

        failures = _fetch_audit_logs(admin_client, failures_only="true", limit=100)
        assert len(failures) > 0, "Expected at least the conversion failure"

        # Mirrors FAILURE_EVENTS in server/src/audit.rs.
        allowed = {
            "media_convert_failure",
            "import_failure",
            "encryption_failure",
            "thumbnail_failure",
            "login_failure",
            "totp_login_failure",
            "rate_limited",
            "account_locked",
        }
        leaked = sorted({e["event_type"] for e in failures} - allowed)
        assert not leaked, f"failures_only leaked non-failure events: {leaked}"

    def test_failure_is_also_reachable_without_the_filter(
        self, user_client: APIClient, admin_client: APIClient
    ):
        """The filter is a convenience, not the only way to see the row.

        Guards against a future 'optimization' that hides failure events from
        the default view.
        """
        filename = _provoke_conversion_failure(user_client)
        _wait_for_audit_event(admin_client, "media_convert_failure", filename)

        unfiltered = _fetch_audit_logs(admin_client, limit=100)
        assert any(
            e["event_type"] == "media_convert_failure"
            and filename in e.get("details", "")
            for e in unfiltered
        ), "The failure row is invisible in an unfiltered listing"
