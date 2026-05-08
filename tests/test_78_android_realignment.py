"""
Android realignment DDT — verifies that every API endpoint the Android
client newly consumes (Sessions 1-5 of the realignment) returns sane
shapes / status codes for an authenticated user.

These tests do *not* exercise feature semantics — that is covered by the
feature-specific E2E suites elsewhere. Here we only assert:

  * Endpoint exists (no 404 / 405)
  * Auth token is honored (no 401)
  * Response is JSON when documented as such

Each row in the parametrize table is one endpoint surface. Adding a new
row is the canonical way to extend the contract guard.
"""
from __future__ import annotations

import pytest


# ── DDT table ────────────────────────────────────────────────────────────────
#
# Each row: (method, path, expected_status_set, description)
#
# We accept any 2xx status as "success" because some endpoints return 200
# and others 204. Some endpoints return 404 when the underlying resource
# does not exist yet (e.g. AI before any photos are scanned) — those cases
# add 404 to the allowed set. We never accept 401 / 405.

ANDROID_API_CONTRACT = [
    # AI ────────────────────────────────────────────────────────────
    pytest.param("GET",  "/api/ai/status",          {200},      id="ai_status"),
    pytest.param("GET",  "/api/ai/faces",           {200},      id="ai_faces_list"),
    pytest.param("GET",  "/api/ai/objects",         {200},      id="ai_objects_list"),
    pytest.param("GET",  "/api/ai/pets",            {200},      id="ai_pets_list"),

    # Geo ───────────────────────────────────────────────────────────
    pytest.param("GET",  "/api/settings/geo",       {200},      id="geo_settings"),
    pytest.param("GET",  "/api/geo/countries",      {200},      id="geo_countries"),
    pytest.param("GET",  "/api/geo/locations",      {200},      id="geo_locations"),
    pytest.param("GET",  "/api/geo/map",            {200},      id="geo_map"),
    pytest.param("GET",  "/api/geo/timeline",       {200},      id="geo_timeline"),
    pytest.param("GET",  "/api/geo/memories",       {200},      id="geo_memories"),
    pytest.param("GET",  "/api/geo/trips",          {200},      id="geo_trips"),

    # Export ────────────────────────────────────────────────────────
    pytest.param("GET",  "/api/export/status",      {200, 404}, id="export_status"),
    pytest.param("GET",  "/api/export/files",       {200},      id="export_files"),

    # Activity / processing status ──────────────────────────────────
    pytest.param("GET",  "/api/status/activity",    {200},      id="status_activity"),
    pytest.param("GET",  "/api/admin/conversion-status", {200, 403}, id="admin_conversion_status"),

    # Setup / discovery ─────────────────────────────────────────────
    pytest.param("GET",  "/api/discover/info",      {200},      id="discover_info"),
]


@pytest.mark.parametrize("method,path,allowed_statuses", ANDROID_API_CONTRACT)
def test_android_endpoint_contract(user_client, method, path, allowed_statuses):
    """Every endpoint the Android app consumes is reachable with a
    user token and returns a JSON body with one of the allowed statuses.
    """
    response = user_client.get(path) if method == "GET" else (
        user_client.post(path) if method == "POST" else None
    )
    assert response is not None, f"unsupported method {method}"

    assert response.status_code in allowed_statuses, (
        f"{method} {path} returned {response.status_code}: "
        f"{response.text[:200]}"
    )

    # 200 responses must be JSON
    if response.status_code == 200:
        ctype = response.headers.get("content-type", "")
        assert "application/json" in ctype, (
            f"{method} {path} returned 200 but content-type={ctype!r}"
        )
