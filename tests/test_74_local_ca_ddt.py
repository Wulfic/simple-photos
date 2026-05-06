"""
Test 74: Self-signed Local CA — input validation (DDT).

Exercises `POST /api/admin/ssl/local-ca` with `dry_run=true` across a
parametrised matrix of inputs to confirm the server validates the label
and `extra_hosts` list **before** mutating the filesystem.

Every row uses `dry_run=true`, so no PEM files are written and no audit
events are logged for "real" provisioning — this file is safe to run
repeatedly without producing side effects on the test instance.

A handful of positive control rows (`accepts_*`) confirm well-formed
requests return `200 OK` with `dry_run=true` echoed back.
"""

import pytest

from helpers import APIClient


# ── Cases ────────────────────────────────────────────────────────────────────
# Each row: (payload_overrides, expected_status, expected_substring_in_message_lower)
LOCAL_CA_INPUT_CASES = [
    # ── Acceptance rows (200) ─────────────────────────────────────────
    pytest.param(
        {},
        200, "", id="accepts_default_inputs",
    ),
    pytest.param(
        {"label": "Simple Photos Local CA — kitchen-NAS"},
        200, "", id="accepts_unicode_label",
    ),
    pytest.param(
        {"extra_hosts": ["photos.local", "192.168.1.50", "10.0.0.5", "::1"]},
        200, "", id="accepts_mixed_dns_and_ip_extras",
    ),
    pytest.param(
        {"label": "x" * 128},
        200, "", id="accepts_label_at_max_length",
    ),
    pytest.param(
        {"extra_hosts": [f"host-{i}.local" for i in range(32)]},
        200, "", id="accepts_max_extra_hosts",
    ),

    # ── Rejection rows (400) ──────────────────────────────────────────
    pytest.param(
        {"label": "x" * 129},
        400, "label", id="rejects_label_over_max_length",
    ),
    pytest.param(
        {"label": "control\x07char"},
        400, "control", id="rejects_label_with_control_char",
    ),
    pytest.param(
        {"label": "null\x00byte"},
        400, "control", id="rejects_label_with_null_byte",
    ),
    pytest.param(
        {"extra_hosts": [f"host-{i}.local" for i in range(33)]},
        400, "32", id="rejects_too_many_extra_hosts",
    ),
    pytest.param(
        {"extra_hosts": [""]},
        400, "extra host", id="rejects_empty_extra_host",
    ),
    pytest.param(
        {"extra_hosts": ["x" * 254]},
        400, "extra host", id="rejects_extra_host_over_253_chars",
    ),
    pytest.param(
        {"extra_hosts": ["bad host with spaces"]},
        400, "invalid characters", id="rejects_extra_host_with_spaces",
    ),
    pytest.param(
        {"extra_hosts": ["bad\x00null"]},
        400, "invalid characters", id="rejects_extra_host_with_null_byte",
    ),
    pytest.param(
        {"extra_hosts": ["control\x01char"]},
        400, "invalid characters", id="rejects_extra_host_with_control_char",
    ),
]


@pytest.mark.parametrize(
    "payload_extra,expected_status,expected_msg_kw",
    LOCAL_CA_INPUT_CASES,
)
def test_local_ca_input_validation(
    primary_admin: APIClient,
    payload_extra: dict,
    expected_status: int,
    expected_msg_kw: str,
):
    """Every input combination must either succeed (dry-run 200) or be
    rejected with a 4xx and a clear, user-readable error message that
    mentions which field was wrong."""
    payload = {"dry_run": True}
    payload.update(payload_extra)

    res = primary_admin.post("/api/admin/ssl/local-ca", json_data=payload)
    assert res.status_code == expected_status, (
        f"Expected {expected_status} for payload {payload!r} but got "
        f"{res.status_code}: {res.text}"
    )
    if expected_status == 200:
        body = res.json()
        # Server must honour dry_run — no real PEM files written.
        assert body.get("dry_run") is True, body
        assert body.get("success") is True, body
        # Dry-run intentionally returns empty fingerprint/hosts so callers
        # can't confuse the validation reply with a successful issue.
        assert body.get("fingerprint_sha256") == "", body
        assert body.get("hosts") == [], body
    else:
        assert expected_msg_kw.lower() in res.text.lower(), (
            f"Expected error message to mention {expected_msg_kw!r} but got: {res.text}"
        )


def test_local_ca_requires_admin_auth(primary_admin: APIClient, primary_server):
    """Unauthenticated callers must be rejected on both the provision and
    bundle-download endpoints — never serve cert artifacts to anonymous
    clients."""
    anon = APIClient(primary_server.base_url)
    res = anon.post("/api/admin/ssl/local-ca", json_data={"dry_run": True})
    assert res.status_code in (401, 403), res.text

    res = anon.get("/api/admin/ssl/local-ca/bundle")
    assert res.status_code in (401, 403), res.text
