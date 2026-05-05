"""E2E + DDT tests for the first-run setup wizard completion gate.

The wizard's contract:

* `GET /api/setup/status` returns both ``setup_complete`` (a user exists) and
  ``wizard_completed`` (admin reached the final "Go to Gallery" step).
* While ``wizard_completed`` is false, every user-data endpoint must respond
  with ``403`` and ``error_code: "wizard_incomplete"`` so the SPA can
  redirect to ``/welcome`` (step 1) instead of showing an error page.
* Endpoints the wizard itself depends on (setup, auth, admin/storage,
  admin/users, admin/port, admin/ssl, admin/backup, downloads) must remain
  reachable while the wizard is incomplete.
* ``POST /api/setup/finalize`` requires admin auth, is idempotent, and flips
  the flag to true.

Each test spins up its own isolated server so we exercise the real
"setup_complete=true / wizard_completed=false" intermediate state — the
shared session-scope ``primary_admin`` fixture finalizes immediately after
login so it can't be used for these checks.
"""
from __future__ import annotations

import os
import socket
import subprocess
import sys
import tempfile
from typing import Iterator

import pytest

sys.path.insert(0, os.path.dirname(__file__))
from helpers import APIClient, wait_for_server  # noqa: E402
from conftest import (  # noqa: E402
    ServerInstance,
    ADMIN_USERNAME,
    ADMIN_PASSWORD,
    USER_PASSWORD,
)


# ── Fixtures ─────────────────────────────────────────────────────────


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture
def fresh_server(server_binary, tmp_path) -> Iterator[ServerInstance]:
    """A brand-new server instance per test \u2014 db is empty, wizard not done."""
    if not server_binary:
        pytest.skip("fresh_server tests require an in-process server build")
    port = _find_free_port()
    server = ServerInstance("wizard_e2e", port, str(tmp_path))
    server.start(server_binary)
    try:
        yield server
    finally:
        server.stop()


@pytest.fixture
def fresh_client(fresh_server) -> APIClient:
    return APIClient(fresh_server.base_url)


@pytest.fixture
def fresh_admin_pre_finalize(fresh_client) -> APIClient:
    """Admin account created + logged in, but wizard NOT yet finalized.

    Note: we call the raw HTTP endpoints here rather than ``client.login``
    because the helper auto-finalizes the wizard for convenience, which is
    exactly what we need to NOT happen in these tests.
    """
    fresh_client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
    r = fresh_client.post(
        "/api/auth/login",
        json_data={"username": ADMIN_USERNAME, "password": ADMIN_PASSWORD},
    )
    r.raise_for_status()
    data = r.json()
    fresh_client.access_token = data["access_token"]
    fresh_client.refresh_token = data.get("refresh_token")
    return fresh_client


# ── E2E: status reporting ───────────────────────────────────────────


class TestStatusFlags:
    def test_status_starts_with_neither_flag_set(self, fresh_client):
        status = fresh_client.setup_status()
        assert status["setup_complete"] is False
        assert status["wizard_completed"] is False

    def test_status_after_init_setup_complete_but_wizard_incomplete(
        self, fresh_client
    ):
        """After setup_init the admin exists but the wizard is mid-flight \u2014
        this is exactly the state the user gets stuck in after a browser crash."""
        fresh_client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        status = fresh_client.setup_status()
        assert status["setup_complete"] is True
        assert status["wizard_completed"] is False

    def test_status_after_finalize_both_flags_true(self, fresh_admin_pre_finalize):
        fresh_admin_pre_finalize.setup_finalize()
        status = fresh_admin_pre_finalize.setup_status()
        assert status["setup_complete"] is True
        assert status["wizard_completed"] is True


# ── DDT: gated endpoints reject pre-finalize ────────────────────────


# (method, path) pairs that *must* return 403 wizard_incomplete while the
# wizard is mid-flight. These are user-data routes only \u2014 the wizard itself
# uses admin/storage, admin/port, admin/ssl, etc. and those must stay open.
GATED_ENDPOINTS = [
    pytest.param("GET", "/api/photos", id="list-photos"),
    pytest.param("GET", "/api/photos/encrypted-sync", id="encrypted-sync"),
    pytest.param("GET", "/api/blobs", id="list-blobs"),
    pytest.param("GET", "/api/galleries/secure", id="list-secure-galleries"),
    pytest.param("GET", "/api/trash", id="list-trash"),
    pytest.param("GET", "/api/sharing/albums", id="list-shared-albums"),
    pytest.param("GET", "/api/tags", id="list-tags"),
    pytest.param("GET", "/api/search", id="search"),
    pytest.param("GET", "/api/ai/status", id="ai-status"),
    pytest.param("GET", "/api/geo/locations", id="geo-locations"),
    pytest.param("GET", "/api/export/status", id="export-status"),
]


@pytest.mark.parametrize("method,path", GATED_ENDPOINTS)
class TestGatedEndpointsRejectPreFinalize:
    def test_returns_403_with_wizard_incomplete_code(
        self, fresh_admin_pre_finalize, method, path
    ):
        client = fresh_admin_pre_finalize
        if method == "GET":
            r = client.get(path)
        elif method == "POST":
            r = client.post(path, json_data={})
        else:
            pytest.fail(f"unsupported method {method}")
        assert r.status_code == 403, (
            f"{method} {path} should be gated while wizard incomplete; "
            f"got {r.status_code}: {r.text[:200]}"
        )
        body = r.json()
        assert body.get("error_code") == "wizard_incomplete", (
            f"{method} {path} 403 must include error_code=wizard_incomplete; "
            f"got {body}"
        )

    def test_endpoint_works_after_finalize(
        self, fresh_admin_pre_finalize, method, path
    ):
        client = fresh_admin_pre_finalize
        client.setup_finalize()
        if method == "GET":
            r = client.get(path)
        else:
            r = client.post(path, json_data={})
        # We don't assert 200 \u2014 some endpoints need extra context (storage,
        # encryption key) to return clean data. What we *do* assert is the
        # wizard gate is no longer the thing rejecting the request.
        if r.status_code == 403:
            body = r.json() if r.headers.get("content-type", "").startswith("application/json") else {}
            assert body.get("error_code") != "wizard_incomplete", (
                f"{method} {path} still gated after finalize: {body}"
            )


# ── DDT: open endpoints work pre-finalize ───────────────────────────


# Routes the wizard depends on. Must respond *without* the wizard_incomplete
# 403 even before finalize.
OPEN_ENDPOINTS_AUTHED = [
    pytest.param("GET", "/api/admin/storage", id="get-storage"),
    pytest.param("GET", "/api/admin/port", id="get-port"),
    pytest.param("GET", "/api/admin/ssl", id="get-ssl"),
    pytest.param("GET", "/api/admin/users", id="list-users"),
    pytest.param("GET", "/api/admin/backup/servers", id="list-backup-servers"),
    pytest.param("GET", "/api/admin/backup/mode", id="backup-mode"),
]


@pytest.mark.parametrize("method,path", OPEN_ENDPOINTS_AUTHED)
class TestOpenEndpointsBypassGate:
    def test_does_not_return_wizard_incomplete(
        self, fresh_admin_pre_finalize, method, path
    ):
        client = fresh_admin_pre_finalize
        r = client.get(path) if method == "GET" else client.post(path, json_data={})
        # Whatever the response is, it must not be the wizard gate refusing
        # the request \u2014 the wizard itself drives these endpoints.
        if r.status_code == 403:
            body = r.json() if r.headers.get("content-type", "").startswith("application/json") else {}
            assert body.get("error_code") != "wizard_incomplete", (
                f"Wizard-required endpoint {method} {path} was gated: {body}"
            )


# ── E2E: finalize semantics ─────────────────────────────────────────


class TestFinalize:
    def test_requires_authentication(self, fresh_client):
        fresh_client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        # No login yet \u2014 access_token is None
        r = fresh_client.post("/api/setup/finalize")
        assert r.status_code == 401, f"unauthenticated finalize must 401, got {r.status_code}"

    def test_requires_admin_role(self, fresh_admin_pre_finalize):
        admin = fresh_admin_pre_finalize
        # Finalize so we can create a regular user (admin/users is open, but
        # the user we create logging in below would otherwise hit the gate
        # on subsequent calls).
        admin.setup_finalize()
        # Create a non-admin user
        r = admin.post(
            "/api/admin/users",
            json_data={
                "username": "regularuser",
                "password": USER_PASSWORD,
                "role": "user",
            },
        )
        r.raise_for_status()
        # Log in as the non-admin
        non_admin = APIClient(admin.base_url)
        login_res = non_admin.post(
            "/api/auth/login",
            json_data={"username": "regularuser", "password": USER_PASSWORD},
        )
        login_res.raise_for_status()
        non_admin.access_token = login_res.json()["access_token"]
        # Finalize is idempotent (already true) but the role check runs first
        # and must reject a non-admin caller.
        r = non_admin.post("/api/setup/finalize")
        assert r.status_code == 403, (
            f"non-admin finalize must 403, got {r.status_code}: {r.text[:200]}"
        )

    def test_is_idempotent(self, fresh_admin_pre_finalize):
        admin = fresh_admin_pre_finalize
        first = admin.setup_finalize()
        second = admin.setup_finalize()
        assert first["wizard_completed"] is True
        assert second["wizard_completed"] is True


# ── E2E: regression for the original bug ────────────────────────────


class TestBrowserCrashRegression:
    """The bug we're fixing: user gets through account creation, browser
    crashes, server is now stuck thinking setup is done and only a server
    reset can recover it. With ``wizard_completed`` we can recover by
    sending the user back to /welcome (the wizard) instead."""

    def test_can_resume_wizard_after_simulated_crash(self, fresh_client):
        # Simulate: admin created, wizard interrupted before finalize.
        fresh_client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
        status = fresh_client.setup_status()
        assert status["setup_complete"] is True
        assert status["wizard_completed"] is False

        # The frontend uses this exact pair of flags to decide whether to
        # forward the user to /welcome. Both guard rails must work:
        #  * ``setup_complete`` true \u2014 don't try to create another admin.
        #  * ``wizard_completed`` false \u2014 send them to /welcome anyway.
        # New client (simulating fresh browser) can still log in as admin
        # and finish the wizard.
        recovery_client = APIClient(fresh_client.base_url)
        login_res = recovery_client.post(
            "/api/auth/login",
            json_data={"username": ADMIN_USERNAME, "password": ADMIN_PASSWORD},
        )
        login_res.raise_for_status()
        recovery_client.access_token = login_res.json()["access_token"]
        recovery_client.refresh_token = login_res.json().get("refresh_token")

        # User-data endpoint still gated.
        r = recovery_client.get("/api/photos")
        assert r.status_code == 403
        assert r.json()["error_code"] == "wizard_incomplete"

        # Finishing the wizard unsticks everything.
        recovery_client.setup_finalize()
        assert recovery_client.setup_status()["wizard_completed"] is True
        r = recovery_client.get("/api/photos")
        assert r.status_code != 403 or r.json().get("error_code") != "wizard_incomplete"
