"""
Test 71: HTTP → HTTPS redirect listener (E2E + DDT).

When `[tls] enabled = true` AND `redirect_http = true` (the default), the
server spawns a small auxiliary listener on `http_redirect_port` that
returns a `301 Moved Permanently` to the equivalent `https://…` URL,
preserving path and query string.

This file boots a real release-build server with a self-signed
certificate, then drives plain-HTTP requests at the redirect port
across a parametrised set of paths/queries and asserts:

  • status code == 301
  • Location header scheme == "https"
  • Location preserves path and query exactly
  • the redirect target's host matches the request Host header

A separate test confirms that with `redirect_http = false` no plain-HTTP
listener is bound — the port stays free for some other process to use.
"""

import datetime
import ipaddress
import os
import shutil
import signal
import socket
import subprocess
import tempfile
import time

import pytest
import requests
import urllib3

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

from conftest import REPO_ROOT, SERVER_DIR, SERVER_BINARY, _find_free_port

urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)


# ── Self-signed cert helper (duplicated from test_22 to keep this file
#    self-contained — both files exercise distinct concerns).
def _generate_self_signed(cert_dir: str) -> tuple[str, str]:
    os.makedirs(cert_dir, exist_ok=True)
    cert_path = os.path.join(cert_dir, "cert.pem")
    key_path = os.path.join(cert_dir, "key.pem")
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = issuer = x509.Name(
        [x509.NameAttribute(NameOID.COMMON_NAME, "localhost")]
    )
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=1))
        .not_valid_after(datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=30))
        .add_extension(
            x509.SubjectAlternativeName([
                x509.DNSName("localhost"),
                x509.IPAddress(ipaddress.ip_address("127.0.0.1")),
            ]),
            critical=False,
        )
        .sign(key, hashes.SHA256())
    )
    with open(cert_path, "wb") as f:
        f.write(cert.public_bytes(serialization.Encoding.PEM))
    with open(key_path, "wb") as f:
        f.write(key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.TraditionalOpenSSL,
            serialization.NoEncryption(),
        ))
    return cert_path, key_path


def _write_redirect_config(
    path: str,
    https_port: int,
    redirect_port: int,
    db_path: str,
    storage_root: str,
    cert_path: str,
    key_path: str,
    *,
    redirect_http: bool,
) -> None:
    config = f"""
[server]
host = "127.0.0.1"
port = {https_port}
base_url = "https://127.0.0.1:{https_port}"
trust_proxy = true
discovery_port = 0

[database]
path = "{db_path}"
max_connections = 4

[storage]
root = "{storage_root}"
default_quota_bytes = 0
max_blob_size_bytes = 104857600

[auth]
jwt_secret = "e2e_test_jwt_secret_must_be_at_least_32_characters_long_for_security"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 4

[web]
static_root = ""

[backup]

[tls]
enabled = true
cert_path = "{cert_path}"
key_path = "{key_path}"
redirect_http = {"true" if redirect_http else "false"}
http_redirect_port = {redirect_port}

[scan]
auto_scan_interval_secs = 0
"""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:  # codeql[py/clear-text-storage-sensitive-data] -- test-only config
        f.write(config)


def _wait_for_https(base_url: str, timeout: float = 30.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = requests.get(f"{base_url}/health", timeout=2, verify=False)
            if r.status_code == 200:
                return
        except requests.RequestException:
            pass
        time.sleep(0.4)
    raise TimeoutError(f"HTTPS server at {base_url} not ready within {timeout}s")


def _port_open(port: int) -> bool:
    """Return True if something is listening on `port` on 127.0.0.1."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(0.5)
    try:
        s.connect(("127.0.0.1", port))
        return True
    except (ConnectionRefusedError, socket.timeout, OSError):
        return False
    finally:
        s.close()


@pytest.fixture(scope="module")
def redirect_server_tmpdir():
    d = tempfile.mkdtemp(prefix="e2e_le_redirect_")
    yield d
    shutil.rmtree(d, ignore_errors=True)


@pytest.fixture(scope="module")
def redirect_server(redirect_server_tmpdir):
    """Boot a TLS-enabled server with the HTTP→HTTPS redirect listener on."""
    # Build server binary if stale.
    if not os.path.exists(SERVER_BINARY) or (time.time() - os.path.getmtime(SERVER_BINARY)) > 600:
        result = subprocess.run(
            ["cargo", "build", "--release"],
            cwd=SERVER_DIR, capture_output=True, text=True, timeout=600,
        )
        if result.returncode != 0:
            print("STDOUT:", result.stdout[-2000:])
            print("STDERR:", result.stderr[-2000:])
            pytest.fail("server build failed")

    cert_path, key_path = _generate_self_signed(os.path.join(redirect_server_tmpdir, "tls"))
    https_port = _find_free_port()
    redirect_port = _find_free_port()
    db_path = os.path.join(redirect_server_tmpdir, "db", "simple-photos.db")
    storage = os.path.join(redirect_server_tmpdir, "storage")
    config_path = os.path.join(redirect_server_tmpdir, "config.toml")
    log_path = os.path.join(redirect_server_tmpdir, "server.log")
    os.makedirs(os.path.dirname(db_path), exist_ok=True)
    os.makedirs(storage, exist_ok=True)

    _write_redirect_config(
        config_path, https_port, redirect_port, db_path, storage,
        cert_path, key_path, redirect_http=True,
    )

    log_file = open(log_path, "w")
    proc = subprocess.Popen(
        [SERVER_BINARY],
        env={**os.environ, "SIMPLE_PHOTOS_CONFIG": config_path, "RUST_LOG": "info"},
        stdout=log_file, stderr=subprocess.STDOUT, cwd=redirect_server_tmpdir,
    )
    try:
        _wait_for_https(f"https://127.0.0.1:{https_port}", timeout=30)
        # Give the redirect listener a moment to bind.
        for _ in range(20):
            if _port_open(redirect_port):
                break
            time.sleep(0.25)
    except Exception:
        proc.send_signal(signal.SIGTERM)
        proc.wait(timeout=5)
        log_file.close()
        with open(log_path) as f:
            print("\n=== redirect_server boot log ===")
            print(f.read()[-5000:])
            print("=== end log ===\n")
        raise

    yield {"https_port": https_port, "redirect_port": redirect_port}

    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    log_file.close()


# ── DDT cases ────────────────────────────────────────────────────────────────
REDIRECT_PATH_CASES = [
    pytest.param("/", id="root_path"),
    pytest.param("/health", id="health_endpoint"),
    pytest.param("/api/photos", id="api_photos"),
    pytest.param("/?foo=bar", id="root_with_query"),
    pytest.param("/api/photos?limit=10&offset=20", id="api_with_multi_query"),
    pytest.param("/some/deep/nested/path", id="deep_path"),
    pytest.param("/path%20with%20spaces", id="encoded_path"),
]


@pytest.mark.parametrize("request_path", REDIRECT_PATH_CASES)
def test_http_request_redirects_to_https(redirect_server, request_path: str):
    """Plain-HTTP request to the redirect listener must produce a 301 to
    the equivalent https:// URL preserving path and query."""
    redirect_port = redirect_server["redirect_port"]
    https_port = redirect_server["https_port"]

    res = requests.get(
        f"http://127.0.0.1:{redirect_port}{request_path}",
        allow_redirects=False,
        timeout=5,
    )
    # Either 301 (Moved Permanently) or 308 (Permanent Redirect) is
    # acceptable — axum's `Redirect::permanent` returns 308 which
    # additionally preserves the request method.
    assert res.status_code in (301, 308), (
        f"Expected permanent redirect, got {res.status_code}: {res.text[:200]}"
    )
    location = res.headers.get("Location", "")
    assert location.startswith("https://"), f"Location must use https: {location!r}"
    # Path + query must be preserved verbatim.
    assert location.endswith(request_path), (
        f"Redirect Location {location!r} must preserve {request_path!r}"
    )
    # Host portion should include the explicit https port (since it's not 443).
    assert f":{https_port}" in location or str(https_port) in location, (
        f"Location should reference https port {https_port}: {location!r}"
    )


def test_http_redirect_preserves_host_header(redirect_server):
    """The redirect must echo back the Host the client sent so virtual
    hosts behind the same listener route back correctly."""
    redirect_port = redirect_server["redirect_port"]
    res = requests.get(
        f"http://127.0.0.1:{redirect_port}/",
        headers={"Host": "photos.example.com"},
        allow_redirects=False,
        timeout=5,
    )
    assert res.status_code in (301, 308)
    assert "photos.example.com" in res.headers.get("Location", "")


# ── Negative case: redirect disabled ─────────────────────────────────────────

@pytest.fixture(scope="module")
def no_redirect_server():
    """Boot a TLS-enabled server with redirect_http=false."""
    tmpdir = tempfile.mkdtemp(prefix="e2e_le_noredirect_")
    try:
        cert_path, key_path = _generate_self_signed(os.path.join(tmpdir, "tls"))
        https_port = _find_free_port()
        redirect_port = _find_free_port()
        db_path = os.path.join(tmpdir, "db", "simple-photos.db")
        storage = os.path.join(tmpdir, "storage")
        config_path = os.path.join(tmpdir, "config.toml")
        log_path = os.path.join(tmpdir, "server.log")
        os.makedirs(os.path.dirname(db_path), exist_ok=True)
        os.makedirs(storage, exist_ok=True)

        _write_redirect_config(
            config_path, https_port, redirect_port, db_path, storage,
            cert_path, key_path, redirect_http=False,
        )

        log_file = open(log_path, "w")
        proc = subprocess.Popen(
            [SERVER_BINARY],
            env={**os.environ, "SIMPLE_PHOTOS_CONFIG": config_path, "RUST_LOG": "info"},
            stdout=log_file, stderr=subprocess.STDOUT, cwd=tmpdir,
        )
        try:
            _wait_for_https(f"https://127.0.0.1:{https_port}", timeout=30)
        except Exception:
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=5)
            log_file.close()
            with open(log_path) as f:
                print("\n=== no_redirect_server boot log ===")
                print(f.read()[-5000:])
                print("=== end log ===\n")
            raise
        yield {"https_port": https_port, "redirect_port": redirect_port}
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        log_file.close()
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_redirect_disabled_does_not_bind_http(no_redirect_server):
    """When redirect_http=false, the http_redirect_port must remain free."""
    # The HTTPS listener should be up.
    assert _port_open(no_redirect_server["https_port"])
    # The redirect listener should NOT have been bound.
    assert not _port_open(no_redirect_server["redirect_port"]), (
        "HTTP redirect listener bound despite redirect_http=false"
    )
