"""
Test 22: TLS Connection Combos — verify that primary and backup servers
can communicate in all four TLS on/off combinations:

  1. Primary HTTP  ↔ Backup HTTP
  2. Primary HTTP  ↔ Backup HTTPS
  3. Primary HTTPS ↔ Backup HTTP
  4. Primary HTTPS ↔ Backup HTTPS

Each combo stands up a fresh pair of servers, initialises admin users,
registers the backup, uploads a photo on the primary, triggers a sync,
and confirms the photo arrives on the backup.

Self-signed TLS certificates are generated once per session using the
`cryptography` library.
"""

import datetime
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

from helpers import (
    APIClient,
    generate_test_jpeg,
    unique_filename,
    wait_for_sync,
)
from conftest import (
    ADMIN_USERNAME,
    ADMIN_PASSWORD,
    TEST_BACKUP_API_KEY,
    TEST_ENCRYPTION_KEY,
    REPO_ROOT,
    SERVER_DIR,
    SERVER_BINARY,
    _find_free_port,
)

# Suppress noisy InsecureRequestWarning from urllib3 for self-signed certs.
urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)


# ── Certificate helpers ──────────────────────────────────────────────────────

def _generate_self_signed_cert(cert_dir: str) -> tuple[str, str]:
    """Generate a self-signed TLS certificate for 127.0.0.1.

    Returns (cert_path, key_path).
    """
    os.makedirs(cert_dir, exist_ok=True)
    cert_path = os.path.join(cert_dir, "cert.pem")
    key_path = os.path.join(cert_dir, "key.pem")

    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)

    subject = issuer = x509.Name([
        x509.NameAttribute(NameOID.COMMON_NAME, "localhost"),
    ])

    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=1))
        .not_valid_after(datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=365))
        .add_extension(
            x509.SubjectAlternativeName([
                x509.DNSName("localhost"),
                x509.IPAddress(ipaddress_for("127.0.0.1")),
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


def ipaddress_for(addr: str):
    """Return an ipaddress object for the given string."""
    import ipaddress
    return ipaddress.ip_address(addr)


# ── Config / server helpers ──────────────────────────────────────────────────

def _write_tls_config(
    path: str,
    port: int,
    db_path: str,
    storage_root: str,
    *,
    tls_enabled: bool = False,
    cert_path: str = "",
    key_path: str = "",
    backup_api_key: str = "",
) -> None:
    """Write a test config.toml with optional TLS settings."""
    scheme = "https" if tls_enabled else "http"
    tls_section = f"""
[tls]
enabled = {"true" if tls_enabled else "false"}
{f'cert_path = "{cert_path}"' if tls_enabled else '# cert_path = ""'}
{f'key_path = "{key_path}"' if tls_enabled else '# key_path = ""'}
"""
    config = f"""
[server]
host = "127.0.0.1"
port = {port}
base_url = "{scheme}://127.0.0.1:{port}"
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
{f'api_key = "{backup_api_key}"' if backup_api_key else '# api_key = ""'}
{tls_section}
[scan]
auto_scan_interval_secs = 0
"""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(config)


class TLSServerInstance:
    """Manages a server process that may or may not use TLS."""

    def __init__(
        self,
        name: str,
        port: int,
        tmpdir: str,
        *,
        tls_enabled: bool = False,
        cert_path: str = "",
        key_path: str = "",
        backup_api_key: str = "",
    ):
        self.name = name
        self.port = port
        self.tmpdir = tmpdir
        self.tls_enabled = tls_enabled
        scheme = "https" if tls_enabled else "http"
        self.base_url = f"{scheme}://127.0.0.1:{port}"
        self.backup_api_key = backup_api_key
        self.process = None

        self.db_path = os.path.join(tmpdir, "db", "simple-photos.db")
        self.storage_root = os.path.join(tmpdir, "storage")
        self.config_path = os.path.join(tmpdir, "config.toml")
        self.log_path = os.path.join(tmpdir, "server.log")

        os.makedirs(os.path.join(tmpdir, "db"), exist_ok=True)
        os.makedirs(self.storage_root, exist_ok=True)

        _write_tls_config(
            self.config_path,
            port,
            self.db_path,
            self.storage_root,
            tls_enabled=tls_enabled,
            cert_path=cert_path,
            key_path=key_path,
            backup_api_key=backup_api_key,
        )

    @property
    def address_with_scheme(self) -> str:
        """Return host:port WITH scheme for backup registration."""
        scheme = "https" if self.tls_enabled else "http"
        return f"{scheme}://127.0.0.1:{self.port}"

    @property
    def address_bare(self) -> str:
        """Return host:port WITHOUT scheme (legacy format)."""
        return f"127.0.0.1:{self.port}"

    def start(self, binary: str):
        """Start the server process."""
        log_file = open(self.log_path, "w")
        env = {
            **os.environ,
            "SIMPLE_PHOTOS_CONFIG": self.config_path,
            "RUST_LOG": "info",
        }
        self.process = subprocess.Popen(
            [binary],
            env=env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
            cwd=self.tmpdir,
        )
        self._log_file = log_file
        try:
            _wait_for_server_tls(self.base_url, timeout=30)
        except TimeoutError:
            self.stop()
            with open(self.log_path) as f:
                print(f"\n=== {self.name} server logs ===")
                print(f.read())
                print("=== end logs ===\n")
            raise

    def stop(self):
        """Stop the server process."""
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if hasattr(self, "_log_file"):
            self._log_file.close()

    def dump_logs(self):
        """Print server logs (useful on test failure)."""
        if os.path.exists(self.log_path):
            with open(self.log_path) as f:
                print(f"\n=== {self.name} server logs ===")
                print(f.read()[-8000:])
                print("=== end logs ===\n")


def _wait_for_server_tls(base_url: str, timeout: float = 30.0, interval: float = 0.5):
    """Block until the server's /health endpoint responds 200.

    Accepts self-signed certificates for HTTPS servers.
    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = requests.get(f"{base_url}/health", timeout=2, verify=False)
            if r.status_code == 200:
                return
        except requests.ConnectionError:
            pass
        time.sleep(interval)
    raise TimeoutError(f"Server at {base_url} did not become ready within {timeout}s")


class TLSAPIClient(APIClient):
    """APIClient variant that accepts self-signed TLS certificates."""

    def __init__(self, base_url: str, api_key=None):
        super().__init__(base_url, api_key=api_key)
        self.session.verify = False


# ── Fixtures ─────────────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def tls_server_binary():
    """Build the server in release mode (reuse if recent)."""
    if os.path.exists(SERVER_BINARY):
        age = time.time() - os.path.getmtime(SERVER_BINARY)
        if age < 300:
            return SERVER_BINARY

    print("\n[TLS E2E] Building server (release mode)...")
    result = subprocess.run(
        ["cargo", "build", "--release"],
        cwd=SERVER_DIR,
        capture_output=True,
        text=True,
        timeout=600,
    )
    if result.returncode != 0:
        print("STDOUT:", result.stdout[-2000:])
        print("STDERR:", result.stderr[-2000:])
        pytest.fail(f"Server build failed with exit code {result.returncode}")
    return SERVER_BINARY


@pytest.fixture(scope="session")
def tls_tmpdir():
    """Temp directory for the entire TLS test session."""
    d = tempfile.mkdtemp(prefix="e2e_tls_combos_")
    yield d
    shutil.rmtree(d, ignore_errors=True)


@pytest.fixture(scope="session")
def tls_certs(tls_tmpdir) -> tuple[str, str]:
    """Generate self-signed TLS certs once per session.

    Returns (cert_path, key_path).
    """
    return _generate_self_signed_cert(os.path.join(tls_tmpdir, "certs"))


# ── The 4 combos ─────────────────────────────────────────────────────────────

TLS_COMBOS = [
    pytest.param(False, False, id="http-http"),
    pytest.param(False, True, id="http-https"),
    pytest.param(True, False, id="https-http"),
    pytest.param(True, True, id="https-https"),
]


class TestTLSCombos:
    """Run each TLS combo as a parametrized test."""

    @pytest.mark.parametrize("primary_tls,backup_tls", TLS_COMBOS)
    def test_sync_across_tls_combos(
        self,
        tls_server_binary,
        tls_tmpdir,
        tls_certs,
        primary_tls: bool,
        backup_tls: bool,
    ):
        cert_path, key_path = tls_certs
        combo_label = (
            f"{'https' if primary_tls else 'http'}_primary"
            f"_{'https' if backup_tls else 'http'}_backup"
        )
        combo_dir = os.path.join(tls_tmpdir, combo_label)
        os.makedirs(combo_dir, exist_ok=True)

        primary_port = _find_free_port()
        backup_port = _find_free_port()

        primary = TLSServerInstance(
            f"primary-{combo_label}",
            primary_port,
            os.path.join(combo_dir, "primary"),
            tls_enabled=primary_tls,
            cert_path=cert_path,
            key_path=key_path,
        )
        backup = TLSServerInstance(
            f"backup-{combo_label}",
            backup_port,
            os.path.join(combo_dir, "backup"),
            tls_enabled=backup_tls,
            cert_path=cert_path,
            key_path=key_path,
            backup_api_key=TEST_BACKUP_API_KEY,
        )

        try:
            # ── Start servers ────────────────────────────────────────
            primary.start(tls_server_binary)
            backup.start(tls_server_binary)

            # ── Initialize admin accounts ────────────────────────────
            pcli = TLSAPIClient(primary.base_url)
            bcli = TLSAPIClient(backup.base_url)

            pcli.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
            pcli.login(ADMIN_USERNAME, ADMIN_PASSWORD)

            bcli.setup_init("backupadmin", "BackupAdminPass123!")
            bcli.login("backupadmin", "BackupAdminPass123!")

            # Store encryption keys
            try:
                pcli.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
            except Exception:
                pass
            try:
                bcli.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
            except Exception:
                pass

            # ── Register backup on primary ───────────────────────────
            # Use the address WITH scheme so the server knows whether
            # the backup is reachable via HTTP or HTTPS.
            backup_address = backup.address_with_scheme
            if not backup_tls:
                # Plain HTTP backup — strip scheme (legacy format accepted)
                backup_address = backup.address_bare

            server_reg = pcli.admin_add_backup_server(
                name=f"tls-backup-{combo_label}",
                address=backup_address,
                api_key=TEST_BACKUP_API_KEY,
                sync_hours=999,
            )
            server_id = server_reg["id"]

            # ── Verify backup is reachable ───────────────────────────
            status = pcli.admin_backup_server_status(server_id)
            assert status["reachable"] is True, (
                f"[{combo_label}] Backup server not reachable. "
                f"primary_tls={primary_tls}, backup_tls={backup_tls}, "
                f"address={backup_address}, error={status.get('error')}"
            )

            # ── Upload a photo to primary ────────────────────────────
            content = generate_test_jpeg(width=3, height=3)
            fname = unique_filename()
            photo = pcli.upload_photo(fname, content=content)
            photo_id = photo["photo_id"]

            # ── Trigger sync ─────────────────────────────────────────
            pcli.admin_trigger_sync(server_id)
            sync_result = wait_for_sync(pcli, server_id, timeout=60)
            assert sync_result["status"] == "success", (
                f"[{combo_label}] Sync failed: {sync_result}"
            )

            # ── Verify photo arrived on backup ───────────────────────
            backup_data_cli = TLSAPIClient(backup.base_url, api_key=TEST_BACKUP_API_KEY)
            backup_photos = backup_data_cli.backup_list()
            backup_ids = [p["id"] for p in backup_photos]
            assert photo_id in backup_ids, (
                f"[{combo_label}] Photo {photo_id} not found on backup. "
                f"Backup has: {backup_ids}"
            )

            print(f"  ✓ {combo_label}: sync OK (photo {photo_id} on backup)")

        except Exception:
            # Dump logs on failure for debugging
            primary.dump_logs()
            backup.dump_logs()
            raise

        finally:
            backup.stop()
            primary.stop()

    @pytest.mark.parametrize("primary_tls,backup_tls", TLS_COMBOS)
    def test_health_check_across_tls_combos(
        self,
        tls_server_binary,
        tls_tmpdir,
        tls_certs,
        primary_tls: bool,
        backup_tls: bool,
    ):
        """Lighter test: just verify health-check connectivity for each combo."""
        cert_path, key_path = tls_certs
        combo_label = (
            f"hc_{'https' if primary_tls else 'http'}_primary"
            f"_{'https' if backup_tls else 'http'}_backup"
        )
        combo_dir = os.path.join(tls_tmpdir, combo_label)
        os.makedirs(combo_dir, exist_ok=True)

        primary_port = _find_free_port()
        backup_port = _find_free_port()

        primary = TLSServerInstance(
            f"primary-{combo_label}",
            primary_port,
            os.path.join(combo_dir, "primary"),
            tls_enabled=primary_tls,
            cert_path=cert_path,
            key_path=key_path,
        )
        backup = TLSServerInstance(
            f"backup-{combo_label}",
            backup_port,
            os.path.join(combo_dir, "backup"),
            tls_enabled=backup_tls,
            cert_path=cert_path,
            key_path=key_path,
            backup_api_key=TEST_BACKUP_API_KEY,
        )

        try:
            primary.start(tls_server_binary)
            backup.start(tls_server_binary)

            # Init admin on both
            pcli = TLSAPIClient(primary.base_url)
            bcli = TLSAPIClient(backup.base_url)

            pcli.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
            pcli.login(ADMIN_USERNAME, ADMIN_PASSWORD)

            bcli.setup_init("backupadmin", "BackupAdminPass123!")
            bcli.login("backupadmin", "BackupAdminPass123!")

            # Register backup
            backup_address = backup.address_with_scheme if backup_tls else backup.address_bare
            server_reg = pcli.admin_add_backup_server(
                name=f"tls-hc-{combo_label}",
                address=backup_address,
                api_key=TEST_BACKUP_API_KEY,
                sync_hours=999,
            )
            server_id = server_reg["id"]

            # Verify health check
            status = pcli.admin_backup_server_status(server_id)
            assert status["reachable"] is True, (
                f"[{combo_label}] Health check failed: {status}"
            )

            print(f"  ✓ {combo_label}: health check OK")

        except Exception:
            primary.dump_logs()
            backup.dump_logs()
            raise

        finally:
            backup.stop()
            primary.stop()
