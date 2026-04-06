"""
Pytest fixtures for Simple Photos E2E tests.

Manages server lifecycle, test users, and API clients.

Usage modes:
  1. Auto-start: tests start fresh server instances on ephemeral ports (default)
  2. External: set E2E_PRIMARY_URL / E2E_BACKUP_URL env vars to use running servers

Auto-start builds the server once and reuses the binary for all instances.
Each test session gets isolated temp directories for databases and storage.
"""

import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

import pytest

# Allow importing helpers from the tests directory
sys.path.insert(0, os.path.dirname(__file__))
from helpers import (
    APIClient,
    random_password,
    random_username,
    wait_for_server,
)

# ── Configuration ────────────────────────────────────────────────────

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER_DIR = os.path.join(REPO_ROOT, "server")
SERVER_BINARY = os.path.join(SERVER_DIR, "target", "release", "simple-photos-server")

# Test credentials
ADMIN_USERNAME = "e2eadmin"
ADMIN_PASSWORD = "E2eAdminPass123!"
USER_PASSWORD = "E2eUserPass456!"

# Encryption key for tests (64 hex chars = 32 bytes AES-256)
TEST_ENCRYPTION_KEY = "a" * 64

# Backup API key for server-to-server auth
TEST_BACKUP_API_KEY = "e2e-backup-test-key-" + "x" * 32


def _find_free_port() -> int:
    """Find an available TCP port."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _write_config(path: str, port: int, db_path: str, storage_root: str,
                  backup_api_key: str = "", discovery_port: int = 0) -> None:
    """Write a minimal test config.toml."""
    config = f"""
[server]
host = "127.0.0.1"
port = {port}
base_url = "http://127.0.0.1:{port}"
trust_proxy = true
discovery_port = {discovery_port}

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

[tls]
enabled = false

[scan]
auto_scan_interval_secs = 0
"""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(config)


class ServerInstance:
    """Manages a running server process."""

    def __init__(self, name: str, port: int, tmpdir: str, backup_api_key: str = ""):
        self.name = name
        self.port = port
        self.tmpdir = tmpdir
        self.base_url = f"http://127.0.0.1:{port}"
        self.backup_api_key = backup_api_key
        self.process = None

        self.db_path = os.path.join(tmpdir, "db", "simple-photos.db")
        self.storage_root = os.path.join(tmpdir, "storage")
        self.config_path = os.path.join(tmpdir, "config.toml")
        self.log_path = os.path.join(tmpdir, "server.log")

        os.makedirs(os.path.join(tmpdir, "db"), exist_ok=True)
        os.makedirs(self.storage_root, exist_ok=True)

        _write_config(
            self.config_path, port, self.db_path, self.storage_root,
            backup_api_key=backup_api_key, discovery_port=0,
        )

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
            wait_for_server(self.base_url, timeout=30)
        except TimeoutError:
            # Dump logs for debugging
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
                print(f.read()[-5000:])  # Last 5000 chars
                print("=== end logs ===\n")


# ── Session-scoped fixtures ──────────────────────────────────────────

@pytest.fixture(scope="session")
def server_binary():
    """Build the server in release mode (once per session)."""
    external = os.environ.get("E2E_PRIMARY_URL")
    if external:
        return None  # Using external servers, no build needed

    # Check if binary already exists and is recent (< 5 min old)
    if os.path.exists(SERVER_BINARY):
        age = time.time() - os.path.getmtime(SERVER_BINARY)
        if age < 300:
            return SERVER_BINARY

    print("\n[E2E] Building server (release mode)...")
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
def session_tmpdir():
    """Temp directory for the entire test session."""
    d = tempfile.mkdtemp(prefix="e2e_simple_photos_")
    yield d
    shutil.rmtree(d, ignore_errors=True)


@pytest.fixture(scope="session")
def primary_server(server_binary, session_tmpdir):
    """Start (or connect to) the primary server."""
    external = os.environ.get("E2E_PRIMARY_URL")
    if external:
        yield type("ExtServer", (), {
            "base_url": external,
            "backup_api_key": os.environ.get("E2E_BACKUP_API_KEY", ""),
            "dump_logs": lambda self: None,
        })()
        return

    port = _find_free_port()
    tmpdir = os.path.join(session_tmpdir, "primary")
    server = ServerInstance("primary", port, tmpdir)
    server.start(server_binary)
    yield server
    server.stop()


@pytest.fixture(scope="session")
def backup_server(server_binary, session_tmpdir):
    """Start (or connect to) a backup server."""
    external = os.environ.get("E2E_BACKUP_URL")
    if external:
        yield type("ExtServer", (), {
            "base_url": external,
            "backup_api_key": os.environ.get("E2E_BACKUP_API_KEY", TEST_BACKUP_API_KEY),
            "dump_logs": lambda self: None,
        })()
        return

    port = _find_free_port()
    tmpdir = os.path.join(session_tmpdir, "backup")
    server = ServerInstance("backup", port, tmpdir, backup_api_key=TEST_BACKUP_API_KEY)
    server.start(server_binary)
    yield server
    server.stop()


@pytest.fixture(scope="session")
def primary_admin(primary_server) -> APIClient:
    """Admin API client for the primary server (initialized once)."""
    client = APIClient(primary_server.base_url)

    # Check if server needs setup
    status = client.setup_status()
    if not status.get("setup_complete"):
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)

    client.login(ADMIN_USERNAME, ADMIN_PASSWORD)
    # Store encryption key for server-side operations
    try:
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
    except Exception:
        pass  # May already be stored
    return client


@pytest.fixture(scope="session")
def backup_admin(backup_server) -> APIClient:
    """Admin API client for the backup server."""
    client = APIClient(backup_server.base_url)
    status = client.setup_status()
    if not status.get("setup_complete"):
        client.setup_init("backupadmin", "BackupAdminPass123!")
    client.login("backupadmin", "BackupAdminPass123!")
    try:
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
    except Exception:
        pass
    return client


@pytest.fixture(scope="session")
def backup_configured(primary_admin, primary_server, backup_server) -> str:
    """Register the backup server on the primary and return its server_id."""
    # Check if already registered
    servers = primary_admin.admin_list_backup_servers()
    backup_addr = backup_server.base_url.replace("http://", "")
    for s in servers.get("servers", []):
        if s.get("address") == backup_addr:
            return s["id"]

    result = primary_admin.admin_add_backup_server(
        name="e2e-backup",
        address=backup_server.base_url.replace("http://", ""),
        api_key=backup_server.backup_api_key,
        sync_hours=999,  # Don't auto-sync; we trigger manually
    )
    return result["id"]


# ── Function-scoped fixtures ─────────────────────────────────────────

@pytest.fixture
def admin_client(primary_admin) -> APIClient:
    """Fresh admin client reference (function-scoped alias)."""
    return primary_admin


@pytest.fixture
def user_client(primary_server, primary_admin) -> APIClient:
    """Create a fresh non-admin user and return a logged-in client."""
    username = random_username()
    primary_admin.admin_create_user(username, USER_PASSWORD, role="user")

    client = APIClient(primary_server.base_url)
    client.login(username, USER_PASSWORD)
    client.username = username
    return client


@pytest.fixture
def second_user_client(primary_server, primary_admin) -> APIClient:
    """Create a second non-admin user for multi-user tests."""
    username = random_username("user2_")
    primary_admin.admin_create_user(username, USER_PASSWORD, role="user")

    client = APIClient(primary_server.base_url)
    client.login(username, USER_PASSWORD)
    client.username = username
    return client


@pytest.fixture
def backup_client(backup_server) -> APIClient:
    """API client for the backup server with the backup API key."""
    return APIClient(backup_server.base_url, api_key=backup_server.backup_api_key)
