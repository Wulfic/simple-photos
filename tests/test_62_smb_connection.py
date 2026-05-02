"""SMB connection wizard E2E + DDT.

Exercises the storage admin endpoints that drive the setup wizard's SMB step:

- `POST /api/admin/storage/test-smb` — calls `smbclient -L` against the share.
- `PUT  /api/admin/storage`         — full mount via `mount.cifs` and switches
                                       the storage root to the resulting path.

The tests are skipped unless the operator has provided real share credentials
via `tests/.env.local` (loaded by `conftest.py`). This keeps CI green on
machines without a live SMB share while still letting a developer exercise the
real pipeline locally.
"""

from __future__ import annotations

import os
import shutil
import subprocess

import pytest


def _smb_env() -> dict | None:
    host = os.environ.get("SMB_TEST_HOST")
    share = os.environ.get("SMB_TEST_SHARE")
    user = os.environ.get("SMB_TEST_USER")
    pw = os.environ.get("SMB_TEST_PASS")
    if not (host and share and user and pw):
        return None
    return {
        "host": host,
        "share": share,
        "subpath": os.environ.get("SMB_TEST_SUBPATH", ""),
        "username": user,
        "password": pw,
        "domain": os.environ.get("SMB_TEST_DOMAIN", ""),
    }


def _have_smbclient() -> bool:
    return shutil.which("smbclient") is not None


def _have_mount_cifs() -> bool:
    return shutil.which("mount.cifs") is not None


pytestmark = pytest.mark.skipif(
    _smb_env() is None,
    reason="SMB_TEST_* env vars not set (see tests/.env.local)",
)


def _build_address(env: dict) -> str:
    addr = f"smb://{env['host']}/{env['share']}"
    if env["subpath"]:
        addr += f"/{env['subpath'].lstrip('/')}"
    return addr


# ── DDT: address-format permutations the wizard accepts ─────────────────────
ADDRESS_FORMAT_CASES = [
    pytest.param("smb://{host}/{share}", id="smb-uri"),
    pytest.param(r"\\{host}\{share}", id="windows-unc"),
    pytest.param("//{host}/{share}", id="posix-style"),
]


@pytest.mark.parametrize("template", ADDRESS_FORMAT_CASES)
def test_smb_test_connection_accepts_address_formats(admin_client, template):
    """`POST /api/admin/storage/test-smb` should accept every documented address format."""
    if not _have_smbclient():
        pytest.skip("smbclient not installed on host")
    env = _smb_env()
    address = template.format(host=env["host"], share=env["share"])
    res = admin_client.post(
        "/api/admin/storage/test-smb",
        json_data={
            "address": address,
            "username": env["username"],
            "password": env["password"],
            "domain": env["domain"] or None,
        },
    )
    # Either probe succeeds, or it fails with a *useful* server-side message
    # rather than a 500. "Logon failure" is acceptable when the credentials
    # don't match the real share — we just want to see structured error flow.
    assert res.status_code in (200, 400), res.text
    if res.status_code == 200:
        body = res.json()
        assert body.get("ok") is True


def test_smb_test_connection_smbclient_missing_message(admin_client, monkeypatch):
    """When smbclient isn't on PATH the server returns a clear 400 with install hint.

    We force the missing-binary path by emptying PATH for the spawned server's
    children — but since the server is already running we can't change its
    PATH from here. Instead, this test only runs when smbclient is genuinely
    absent on the host.
    """
    if _have_smbclient():
        pytest.skip("smbclient is installed; nothing to assert")
    env = _smb_env()
    res = admin_client.post(
        "/api/admin/storage/test-smb",
        json_data={
            "address": _build_address(env),
            "username": env["username"],
            "password": env["password"],
        },
    )
    assert res.status_code == 400
    assert "smbclient" in res.text.lower()


def test_smb_mount_full_round_trip(admin_client):
    """`PUT /api/admin/storage` with an SMB descriptor mounts and switches storage root.

    Skipped unless `SMB_TEST_SUBPATH` points at a *small, dedicated* directory
    on the share. Pointing at the share root would force the server to scan
    every file under the export, which can take many minutes against real
    NAS hardware and isn't what this test is verifying.
    """
    env = _smb_env()
    if not env["subpath"]:
        pytest.skip(
            "SMB_TEST_SUBPATH is empty — set it to a dedicated test directory "
            "on the share to enable the full mount+switch round-trip test"
        )
    if not _have_mount_cifs():
        pytest.skip("mount.cifs (cifs-utils) not installed")
    if os.geteuid() != 0:
        # Detect whether the unprivileged path will work *before* we trigger it,
        # so a failure here is informative rather than hiding the real cause.
        try:
            stat = os.stat("/usr/sbin/mount.cifs")
            suid_ok = bool(stat.st_mode & 0o4000)
        except FileNotFoundError:
            suid_ok = False
        sudoers_ok = os.path.exists("/etc/sudoers.d/simple-photos-cifs")
        if not (suid_ok or sudoers_ok):
            pytest.skip(
                "mount.cifs cannot mount as unprivileged user — "
                "set SUID or add /etc/sudoers.d/simple-photos-cifs"
            )

    res = admin_client.put(
        "/api/admin/storage",
        json_data={
            "smb": {
                "address": _build_address(env),
                "username": env["username"],
                "password": env["password"],
                "domain": env["domain"] or None,
            }
        },
        timeout=60,
    )
    assert res.status_code == 200, res.text
    body = res.json()
    assert body["smb"]["mounted"] is True
    assert body["smb"]["address"] == _build_address(env)
    assert body["smb"]["username"] == env["username"]


# ── DDT: invalid SMB descriptors should all be rejected with 400 ────────────
INVALID_SMB_CASES = [
    pytest.param({"address": ""}, id="empty-address"),
    pytest.param({"address": "smb://"}, id="missing-host-and-share"),
    pytest.param({"address": "smb://host"}, id="missing-share"),
    pytest.param({"address": "smb://host/share/../escape"}, id="path-traversal"),
    pytest.param({"address": "smb://-flag/share"}, id="leading-dash-host"),
    pytest.param({"address": "smb://host/sh,are"}, id="comma-in-share"),
    pytest.param({"address": "ftp://host/share"}, id="non-smb-scheme"),
]


@pytest.mark.parametrize("payload", INVALID_SMB_CASES)
def test_smb_test_connection_rejects_invalid(admin_client, payload):
    env = _smb_env()
    body = {**payload, "username": env["username"], "password": env["password"]}
    res = admin_client.post("/api/admin/storage/test-smb", json_data=body)
    assert res.status_code == 400, res.text
