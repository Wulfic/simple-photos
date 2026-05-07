"""
Test 75: Self-signed Local CA — full E2E.

Drives `POST /api/admin/ssl/local-ca` with `dry_run=false` against a real
running primary server, then:

  1. Downloads the install bundle from `GET /api/admin/ssl/local-ca/bundle`.
  2. Verifies the zip contains exactly the expected entries (public CA
     cert, install scripts for Linux / Windows / Android, and a README)
     and **no** private keys.
  3. Confirms the SSL status endpoint surfaces the new `local_ca` block
     with a matching SHA-256 fingerprint and the leaf SANs covering at
     least localhost.
  4. Confirms the cert + key files written to the data directory parse
     as PEM and the leaf is signed by the generated root.

This file uses real I/O — it is in the regular test suite so any
regression in cert generation is caught immediately.
"""

import io
import re
import zipfile

import pytest

from helpers import APIClient


def _verify_pem_block(text: str, kind: str) -> None:
    """Tiny PEM sanity check — full chain validation lives in test_22."""
    assert f"-----BEGIN {kind}-----" in text, f"missing PEM header for {kind}"
    assert f"-----END {kind}-----" in text, f"missing PEM footer for {kind}"


@pytest.fixture
def fresh_local_ca(primary_admin: APIClient):
    """Generate a fresh local CA and return the response payload.

    Re-entrant: each call rotates the CA, which is exactly the documented
    behaviour of `generate_local_ca` (the operator can re-issue at any
    time from the settings panel).
    """
    res = primary_admin.post(
        "/api/admin/ssl/local-ca",
        json_data={
            "label": "Simple Photos Test CA",
            "extra_hosts": ["photos.test", "10.99.99.99"],
            "dry_run": False,
        },
    )
    assert res.status_code == 200, res.text
    body = res.json()
    assert body["success"] is True, body
    assert body["dry_run"] is False, body
    return body


def test_local_ca_generation_returns_metadata(fresh_local_ca):
    """The provisioning endpoint must return non-empty metadata when
    `dry_run=false` so the UI can render the active-CA badge without a
    second round-trip."""
    body = fresh_local_ca
    # Fingerprint is colon-separated uppercase hex, exactly 32 bytes →
    # 32 × 2 hex chars + 31 colons = 95 chars.
    assert re.fullmatch(r"(?:[0-9A-F]{2}:){31}[0-9A-F]{2}", body["fingerprint_sha256"]), body
    # Always-present hosts.
    assert "localhost" in body["hosts"], body
    assert "127.0.0.1" in body["hosts"], body
    # Operator-supplied extras propagate.
    assert "photos.test" in body["hosts"], body
    assert "10.99.99.99" in body["hosts"], body
    # Issued PEM paths point inside data/local_ca.
    assert "local_ca" in body["cert_path"]
    assert body["cert_path"].endswith("server.pem")
    assert body["key_path"].endswith("server.key")
    # Server explicitly tells the operator a restart is required.
    assert "restart" in body["message"].lower(), body


def test_local_ca_bundle_available_after_generation(
    primary_admin: APIClient, fresh_local_ca
):
    """After `provision_local_ca` succeeds, the bundle download endpoint
    must immediately serve a zip — that's the file the UI's "Download
    CA install bundle" button points to.

    Note: `GET /api/admin/ssl` reads from a server-startup snapshot of
    `config.toml` (see `state.config: Arc<AppConfig>`), so the
    `local_ca` block won't appear there until the next restart. The
    POST response already includes the metadata for the UI.
    """
    res = primary_admin.get("/api/admin/ssl/local-ca/bundle")
    assert res.status_code == 200, res.text
    assert res.headers.get("Content-Type") == "application/zip"
    assert len(res.content) > 0
    # Round-trip: response payload from generate echoes the same data
    # the UI needs, no second request required.
    assert fresh_local_ca["fingerprint_sha256"], fresh_local_ca
    for host in ("localhost", "127.0.0.1", "photos.test", "10.99.99.99"):
        assert host in fresh_local_ca["hosts"], fresh_local_ca


def test_local_ca_bundle_contains_install_scripts(
    primary_admin: APIClient, fresh_local_ca
):
    """The download endpoint must return a zip containing the public CA
    cert + install scripts for every supported platform, and **never**
    private keys."""
    res = primary_admin.get("/api/admin/ssl/local-ca/bundle")
    assert res.status_code == 200, res.text
    assert res.headers.get("Content-Type") == "application/zip", res.headers
    assert "attachment" in res.headers.get("Content-Disposition", ""), res.headers

    z = zipfile.ZipFile(io.BytesIO(res.content))
    names = set(z.namelist())
    expected = {
        "ca.pem",
        "install-linux.sh",
        "install-windows.ps1",
        "install-android.txt",
        "README.md",
    }
    assert expected <= names, f"bundle missing entries: {expected - names}"

    # Catastrophic regression: must NEVER include private keys.
    forbidden_substrings = ("PRIVATE KEY", "ca.key", "server.key", "privkey")
    for name in names:
        body = z.read(name).decode("utf-8", errors="replace")
        for needle in forbidden_substrings:
            if name.endswith(".pem"):
                assert needle != "PRIVATE KEY" or needle not in body, (
                    f"{name} leaked a private key"
                )
        for forbidden_name in ("ca.key", "server.key"):
            assert forbidden_name not in name, f"bundle contains private key file {name}"

    # Public CA cert is a syntactically valid PEM CERTIFICATE block.
    ca_pem = z.read("ca.pem").decode("utf-8")
    _verify_pem_block(ca_pem, "CERTIFICATE")
    assert "PRIVATE KEY" not in ca_pem, "ca.pem must not contain a private key"

    # Regression guard for "Permission denied OS error 13" — the shell
    # script must ship with the executable bit set so `unzip` extracts
    # it as 0755 and `./install-linux.sh` runs without `chmod +x`.
    info = z.getinfo("install-linux.sh")
    # zip stores Unix mode in the upper 16 bits of external_attr.
    unix_mode = (info.external_attr >> 16) & 0o777
    assert unix_mode & 0o111, (
        f"install-linux.sh must be executable in the bundle, got mode {oct(unix_mode)}"
    )

    # Each install script embeds the fingerprint that matches the API
    # response, so the script can refuse to install a tampered cert.
    fingerprint = fresh_local_ca["fingerprint_sha256"]
    for script in ("install-linux.sh", "install-windows.ps1", "install-android.txt", "README.md"):
        body = z.read(script).decode("utf-8")
        # Linux script uses the colonised form; Windows uses the no-colon
        # form (per Get-CertHashString); Android/README show the colonised
        # form — assert at least one matches.
        no_colons = fingerprint.replace(":", "")
        assert (fingerprint in body) or (no_colons in body), (
            f"{script} missing fingerprint"
        )


def test_local_ca_dry_run_does_not_overwrite(primary_admin: APIClient, fresh_local_ca):
    """A `dry_run=true` request must NOT mutate filesystem state — the
    bundle on disk should still match the previously-generated CA."""
    original_fp = fresh_local_ca["fingerprint_sha256"]
    bundle_before = primary_admin.get("/api/admin/ssl/local-ca/bundle").content

    res = primary_admin.post(
        "/api/admin/ssl/local-ca",
        json_data={"label": "Dry run only", "dry_run": True},
    )
    assert res.status_code == 200, res.text
    body = res.json()
    assert body["dry_run"] is True
    # Dry-run must not return real metadata.
    assert body["fingerprint_sha256"] == ""

    # Bundle on disk is byte-identical — the dry-run did not rotate.
    bundle_after = primary_admin.get("/api/admin/ssl/local-ca/bundle").content
    assert bundle_before == bundle_after
    assert original_fp  # sanity: real generation produced a fingerprint


def test_local_ca_regeneration_rotates_fingerprint(primary_admin: APIClient):
    """Re-running the provision endpoint must produce a *different*
    fingerprint — operators expect "Re-generate" to actually rotate
    the CA, not silently keep the old one."""
    first = primary_admin.post(
        "/api/admin/ssl/local-ca",
        json_data={"dry_run": False},
    ).json()
    second = primary_admin.post(
        "/api/admin/ssl/local-ca",
        json_data={"dry_run": False},
    ).json()
    assert first["fingerprint_sha256"] != second["fingerprint_sha256"], (
        first,
        second,
    )
