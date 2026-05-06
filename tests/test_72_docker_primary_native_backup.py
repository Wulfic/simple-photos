"""
Test 72: Mixed-deployment backup pipeline — Docker primary + native backup.

Validates the deployment shape that operators actually run in production
but that no existing test exercises: the primary photo server runs inside
a Docker container (built from `server/Dockerfile`), while the backup
server runs as a native binary on the host.  The primary registers the
native backup over the loopback network and a sync transfers a mixed
dataset across the container/host boundary.

Skipped automatically when:
  * `docker` CLI is not on PATH;
  * the docker daemon is unreachable;
  * `E2E_SKIP_DOCKER=1` is set; or
  * an external server URL is configured via `E2E_PRIMARY_URL`.

Per repo testing rules this file is the next sequential number (71 → 72).
A small DDT table covers multiple MIME types so the cross-boundary path
is exercised against jpeg / png / blob payloads in a single test
function with one parametrised row per format.
"""

from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import time
import uuid
from pathlib import Path

import pytest

from helpers import (
    APIClient,
    generate_random_bytes,
    generate_test_jpeg,
    generate_test_png,
    trigger_and_wait,
    wait_for_server,
)
from conftest import (
    ADMIN_PASSWORD,
    ADMIN_USERNAME,
    REPO_ROOT,
    SERVER_DIR,
    ServerInstance,
    TEST_BACKUP_API_KEY,
    TEST_ENCRYPTION_KEY,
    USER_PASSWORD,
)


# ── Skip-gating helpers ──────────────────────────────────────────────

DOCKER_IMAGE_TAG = "simple-photos-server:e2e"
SKIP_REASON_NO_DOCKER = "docker CLI / daemon unavailable on this host"


def _docker_available() -> tuple[bool, str]:
    if os.environ.get("E2E_SKIP_DOCKER") == "1":
        return False, "E2E_SKIP_DOCKER=1"
    if os.environ.get("E2E_PRIMARY_URL"):
        return False, "external E2E_PRIMARY_URL configured"
    if shutil.which("docker") is None:
        return False, SKIP_REASON_NO_DOCKER
    try:
        r = subprocess.run(
            ["docker", "info", "--format", "{{.ServerVersion}}"],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (subprocess.SubprocessError, OSError) as exc:
        return False, f"docker info failed: {exc}"
    if r.returncode != 0:
        return False, f"docker daemon not reachable (rc={r.returncode})"
    return True, ""


_DOCKER_OK, _DOCKER_SKIP_MSG = _docker_available()
pytestmark = pytest.mark.skipif(not _DOCKER_OK, reason=_DOCKER_SKIP_MSG or SKIP_REASON_NO_DOCKER)


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _write_primary_container_config(
    cfg_path: Path,
    host_port: int,
) -> None:
    """Write the config.toml that gets bind-mounted into the container.

    Paths inside the container are fixed (`/data/storage`, `/data/db`)
    to match `server/Dockerfile`. The host port is reused as the
    container port via `--network host` so the native backup can reach
    the primary on `127.0.0.1` without DNS gymnastics.
    """
    cfg_path.parent.mkdir(parents=True, exist_ok=True)
    cfg_path.write_text(
        f"""
[server]
host = "0.0.0.0"
port = {host_port}
base_url = "http://127.0.0.1:{host_port}"
trust_proxy = true
discovery_port = 0

[database]
path = "/data/db/simple-photos.db"
max_connections = 4

[storage]
root = "/data/storage"
default_quota_bytes = 0
max_blob_size_bytes = 104857600

[auth]
jwt_secret = "e2e_docker_test_jwt_secret_must_be_at_least_32_characters_long"
access_token_ttl_secs = 3600
refresh_token_ttl_days = 30
allow_registration = true
bcrypt_cost = 4

[web]
static_root = ""

[backup]
# api_key only needed when this node is acting as a backup target.

[tls]
enabled = false

[scan]
auto_scan_interval_secs = 0

[geo]
enabled = false
poll_interval_secs = 2

[ai]
enabled = false
"""
    )


# ── Fixtures ────────────────────────────────────────────────────────

@pytest.fixture(scope="module")
def docker_image() -> str:
    """Build the server image once per session, reuse the tag.

    The Dockerfile honours a `CARGO_FEATURES` build-arg (added in the
    GPU-detection work); for E2E we deliberately leave it empty to
    keep the image portable on CI runners without an NVIDIA GPU.
    """
    print(f"\n[E2E] Building docker image {DOCKER_IMAGE_TAG} (this may take several minutes)...")
    r = subprocess.run(
        [
            "docker",
            "build",
            "--tag",
            DOCKER_IMAGE_TAG,
            "--build-arg",
            "CARGO_FEATURES=",
            SERVER_DIR,
        ],
        capture_output=True,
        text=True,
        timeout=1800,  # First build can be slow; subsequent layers are cached.
    )
    if r.returncode != 0:
        pytest.skip(
            f"docker build failed (rc={r.returncode}); skipping mixed-deployment "
            f"test. Last 1KB of stderr:\n{r.stderr[-1024:]}"
        )
    return DOCKER_IMAGE_TAG


@pytest.fixture(scope="module")
def docker_primary(docker_image, session_tmpdir):
    """Run the primary photo server inside a Docker container.

    Uses `--network host` so the native backup server (also bound to
    127.0.0.1) is reachable from the registration call without
    having to figure out container IPs or `host.docker.internal`
    on every CI distro.
    """
    instance_dir = Path(session_tmpdir) / "docker_primary"
    cfg_path = instance_dir / "config.toml"
    storage_dir = instance_dir / "storage"
    db_dir = instance_dir / "db"
    storage_dir.mkdir(parents=True, exist_ok=True)
    db_dir.mkdir(parents=True, exist_ok=True)

    port = _free_port()
    _write_primary_container_config(cfg_path, port)

    container_name = f"sp-e2e-primary-{uuid.uuid4().hex[:8]}"
    log_path = instance_dir / "container.log"

    run_cmd = [
        "docker",
        "run",
        "--rm",
        "--detach",
        "--name",
        container_name,
        "--network",
        "host",
        "--user",
        f"{os.getuid()}:{os.getgid()}",
        "--env",
        "RUST_LOG=info",
        "--env",
        f"SIMPLE_PHOTOS_CONFIG=/app/config.toml",
        "--volume",
        f"{cfg_path}:/app/config.toml:ro",
        "--volume",
        f"{storage_dir}:/data/storage",
        "--volume",
        f"{db_dir}:/data/db",
        docker_image,
    ]

    proc = subprocess.run(run_cmd, capture_output=True, text=True, timeout=60)
    if proc.returncode != 0:
        pytest.skip(
            f"docker run failed (rc={proc.returncode}): {proc.stderr.strip()}"
        )
    container_id = proc.stdout.strip()

    base_url = f"http://127.0.0.1:{port}"
    try:
        wait_for_server(base_url, timeout=60)
    except TimeoutError:
        # Capture container logs for the failure report and tear it down.
        try:
            logs = subprocess.run(
                ["docker", "logs", container_id],
                capture_output=True,
                text=True,
                timeout=10,
            )
            log_path.write_text(
                (logs.stdout or "") + "\n--- stderr ---\n" + (logs.stderr or "")
            )
            print(f"\n=== docker primary logs ({log_path}) ===")
            print(log_path.read_text()[-4000:])
            print("=== end logs ===")
        finally:
            subprocess.run(["docker", "rm", "-f", container_id],
                           capture_output=True, timeout=30)
        raise

    yield type("DockerPrimary", (), {
        "base_url": base_url,
        "container_id": container_id,
        "container_name": container_name,
        "log_path": log_path,
        "port": port,
    })()

    # Teardown — capture logs first for post-mortem if KEEP_E2E_TMPDIR is set.
    try:
        logs = subprocess.run(
            ["docker", "logs", container_id],
            capture_output=True, text=True, timeout=15,
        )
        log_path.write_text(
            (logs.stdout or "") + "\n--- stderr ---\n" + (logs.stderr or "")
        )
    except (subprocess.SubprocessError, OSError):
        pass
    subprocess.run(["docker", "rm", "-f", container_id],
                   capture_output=True, timeout=30)


@pytest.fixture(scope="module")
def docker_primary_admin(docker_primary) -> APIClient:
    """Bootstrap the dockerised primary the same way conftest does for native."""
    client = APIClient(docker_primary.base_url)
    if not client.setup_status().get("setup_complete"):
        client.setup_init(ADMIN_USERNAME, ADMIN_PASSWORD)
    client.login(ADMIN_USERNAME, ADMIN_PASSWORD)
    if not client.setup_status().get("wizard_completed"):
        client.setup_finalize()
    try:
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
    except Exception:
        pass
    return client


@pytest.fixture(scope="module")
def docker_primary_user(docker_primary, docker_primary_admin) -> APIClient:
    username = f"e2e_user_{uuid.uuid4().hex[:8]}"
    docker_primary_admin.admin_create_user(username, USER_PASSWORD, role="user")
    client = APIClient(docker_primary.base_url)
    client.login(username, USER_PASSWORD)
    client.username = username
    return client


@pytest.fixture(scope="module")
def docker_test_backup(server_binary, session_tmpdir):
    """Dedicated native backup server for the docker E2E test.

    Avoids using the session-scoped `backup_server` fixture so this
    test cannot pollute downstream tests' backup state (and vice
    versa). The binary is the same release artifact that conftest
    builds, just pointed at a fresh tmpdir / db / storage tree.
    """
    port = _free_port()
    tmpdir = os.path.join(session_tmpdir, "docker_test_backup")
    server = ServerInstance(
        "docker_test_backup", port, tmpdir, backup_api_key=TEST_BACKUP_API_KEY,
    )
    server.start(server_binary)
    yield server
    server.stop()


@pytest.fixture(scope="module")
def docker_test_backup_admin(docker_test_backup) -> APIClient:
    client = APIClient(docker_test_backup.base_url)
    if not client.setup_status().get("setup_complete"):
        client.setup_init("dockerbackupadmin", "DockerBackupAdmin123!")
    client.login("dockerbackupadmin", "DockerBackupAdmin123!")
    if not client.setup_status().get("wizard_completed"):
        client.setup_finalize()
    try:
        client.admin_store_encryption_key(TEST_ENCRYPTION_KEY)
    except Exception:
        pass
    return client


@pytest.fixture(scope="module")
def docker_test_backup_client(docker_test_backup) -> APIClient:
    return APIClient(
        docker_test_backup.base_url, api_key=docker_test_backup.backup_api_key,
    )


@pytest.fixture(scope="module")
def docker_backup_configured(
    docker_primary_admin, docker_test_backup, docker_test_backup_admin,
) -> str:
    """Register the dedicated native backup with the dockerised primary."""
    backup_addr = docker_test_backup.base_url.replace("http://", "")
    existing = docker_primary_admin.admin_list_backup_servers().get("servers", [])
    for s in existing:
        if s.get("address") == backup_addr:
            return s["id"]
    result = docker_primary_admin.admin_add_backup_server(
        name="e2e-native-backup",
        address=backup_addr,
        api_key=docker_test_backup.backup_api_key,
        sync_hours=999,
    )
    return result["id"]


# ── DDT: cross-boundary payload table ────────────────────────────────

PAYLOAD_CASES = [
    pytest.param(
        "jpeg_photo",
        lambda: generate_test_jpeg(width=8, height=8),
        "image/jpeg",
        ".jpg",
        id="jpeg_photo",
    ),
    pytest.param(
        "png_photo",
        lambda: generate_test_png(),
        "image/png",
        ".png",
        id="png_photo",
    ),
    pytest.param(
        "binary_blob",
        lambda: generate_random_bytes(2048),
        "application/octet-stream",
        ".bin",
        id="binary_blob",
    ),
]


# ── Tests ────────────────────────────────────────────────────────────

class TestDockerPrimaryNativeBackup:
    """Primary in Docker, backup native — full upload → sync round-trip."""

    def test_primary_container_is_healthy(self, docker_primary):
        """Container responds on its host-published port."""
        import requests
        r = requests.get(f"{docker_primary.base_url}/health", timeout=5)
        assert r.status_code == 200, f"health check failed: {r.status_code} {r.text}"

    def test_backup_registration_succeeds(
        self, docker_primary_admin, docker_backup_configured,
    ):
        """The native backup is registered with the dockerised primary."""
        servers = docker_primary_admin.admin_list_backup_servers().get("servers", [])
        ids = {s["id"] for s in servers}
        assert docker_backup_configured in ids

    @pytest.mark.parametrize("label,factory,mime,ext", PAYLOAD_CASES)
    def test_cross_boundary_sync_per_payload(
        self,
        label,
        factory,
        mime,
        ext,
        docker_primary_admin,
        docker_primary_user,
        docker_backup_configured,
        docker_test_backup_client,
    ):
        """Upload a payload to the dockerised primary and confirm it
        appears on the native backup after a triggered sync."""
        backup_client = docker_test_backup_client
        # Snapshot the backup before so we only count *our* additions.
        before_photos = {p["id"] for p in backup_client.backup_list()}
        before_blobs = {b["id"] for b in backup_client.backup_list_blobs()}

        content = factory()
        filename = f"docker_{label}_{uuid.uuid4().hex[:6]}{ext}"

        if mime.startswith("image/"):
            upload = docker_primary_user.upload_photo(
                filename=filename, content=content, mime_type=mime,
            )
            uploaded_id = upload["photo_id"]
            target_set = "photos"
        else:
            upload = docker_primary_user.upload_blob("photo", content)
            uploaded_id = upload["blob_id"]
            target_set = "blobs"

        # Trigger sync and wait for completion (sync runs over loopback
        # from inside the container to the native backup on the host).
        result = trigger_and_wait(
            docker_primary_admin, docker_backup_configured, timeout=180,
        )
        assert result.get("status") != "error", (
            f"Sync failed for {label}: {result}"
        )

        if target_set == "photos":
            after = {p["id"] for p in backup_client.backup_list()}
            new = after - before_photos
            assert uploaded_id in new, (
                f"{label}: photo {uploaded_id} not on backup after sync. "
                f"new={new}"
            )
        else:
            after = {b["id"] for b in backup_client.backup_list_blobs()}
            new = after - before_blobs
            assert uploaded_id in new, (
                f"{label}: blob {uploaded_id} not on backup after sync. "
                f"new={new}"
            )
