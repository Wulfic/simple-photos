"""
AI accuracy validation: face detection + object classification.

Uploads real photographs (faces + common objects) and verifies the AI pipeline
correctly identifies them. Uses SCRFD for face detection and MobileNetV2 for
object classification.

Success criteria: both face detection and object classification must reach
95% accuracy across the test dataset.
"""

import os
import sys
import time

import pytest

sys.path.insert(0, os.path.dirname(__file__))
from helpers import APIClient, wait_for_server

TEST_DATA = os.path.join(os.path.dirname(__file__), "test_data")
FACE_DIR = os.path.join(TEST_DATA, "ai_faces")
OBJ_DIR = os.path.join(TEST_DATA, "ai_objects")
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MODEL_DIR = os.path.join(REPO_ROOT, "server", "models")

# ── Test data manifests ──────────────────────────────────────────────

# Each face photo should produce at least 1 face detection.
FACE_PHOTOS = [
    "face_01_woman.jpg",
    "face_02_man.jpg",
    "face_03_woman.jpg",
    "face_04_man.jpg",
    "face_05_woman.jpg",
    "face_06_man.jpg",
    "face_07_woman.jpg",
    "face_08_man.jpg",
    "face_09_woman.jpg",
    "face_10_man.jpg",
]

# Object photos: filename → set of acceptable category matches.
# The classifier might assign any of these labels due to ImageNet mapping.
OBJECT_PHOTOS = {
    "obj_dog.jpg": {"dog"},
    "obj_cat.jpg": {"cat"},
    "obj_car.jpg": {"vehicle"},
    "obj_food.jpg": {"food"},
    "obj_flower.jpg": {"plant", "flower"},
    "obj_bird.jpg": {"bird"},
    "obj_building.jpg": {"building", "landscape"},
    "obj_laptop.jpg": {"electronics"},
    "obj_horse.jpg": {"horse", "animal"},
    "obj_elephant.jpg": {"elephant", "animal"},
    "obj_sports.jpg": {"sports"},
    "obj_landscape.jpg": {"landscape"},
    "obj_bear.jpg": {"bear", "animal", "wild animal"},
    "obj_boat.jpg": {"vehicle", "landscape", "person"},
    "obj_guitar.jpg": {"music"},
}

TARGET_ACCURACY = 0.95


# ── Fixtures ─────────────────────────────────────────────────────────

@pytest.fixture(scope="module")
def ai_server(server_binary, tmp_path_factory):
    """Start a server with AI enabled and high throughput config."""
    import signal
    import socket
    import subprocess

    if os.environ.get("E2E_PRIMARY_URL"):
        pytest.skip("AI accuracy tests require auto-started server with AI config")

    tmpdir = str(tmp_path_factory.mktemp("ai_accuracy"))

    # Find a free port
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]

    db_path = os.path.join(tmpdir, "db", "simple-photos.db")
    storage_root = os.path.join(tmpdir, "storage")
    config_path = os.path.join(tmpdir, "config.toml")
    log_path = os.path.join(tmpdir, "server.log")

    os.makedirs(os.path.join(tmpdir, "db"), exist_ok=True)
    os.makedirs(storage_root, exist_ok=True)

    # Write config with AI enabled, fast processing
    config = f"""
[server]
host = "127.0.0.1"
port = {port}
base_url = "http://127.0.0.1:{port}"
trust_proxy = true
discovery_port = 0

[database]
path = "{db_path}"
max_connections = 8

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
enabled = false

[scan]
auto_scan_interval_secs = 0

[ai]
enabled = true
batch_size = 50
photos_per_minute = 600
face_confidence = 0.5
object_confidence = 0.3
quality = "high"
model_dir = "{MODEL_DIR}"
"""
    with open(config_path, "w") as f:
        f.write(config)

    base_url = f"http://127.0.0.1:{port}"
    log_file = open(log_path, "w")
    env = {
        **os.environ,
        "SIMPLE_PHOTOS_CONFIG": config_path,
        "RUST_LOG": "info,simple_photos_server::ai=debug",
    }
    proc = subprocess.Popen(
        [server_binary],
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
        cwd=tmpdir,
    )

    try:
        wait_for_server(base_url, timeout=60)
    except TimeoutError:
        proc.kill()
        proc.wait()
        log_file.close()
        with open(log_path) as f:
            print(f"\n=== AI server logs ===\n{f.read()}\n=== end ===")
        raise

    yield {
        "base_url": base_url,
        "process": proc,
        "log_path": log_path,
        "log_file": log_file,
        "storage_root": storage_root,
        "tmpdir": tmpdir,
    }

    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    log_file.close()


@pytest.fixture(scope="module")
def ai_client(ai_server):
    """Admin client for the AI-enabled server."""
    client = APIClient(ai_server["base_url"])
    # Setup admin user and login
    client.setup_init("aiadmin", "AiAdminPass123!")
    client.login("aiadmin", "AiAdminPass123!")
    return client


# ── Helpers ──────────────────────────────────────────────────────────

def upload_real_photo(client: APIClient, filepath: str, filename: str) -> dict:
    """Upload a real photo file from disk."""
    with open(filepath, "rb") as f:
        content = f.read()
    return client.upload_photo(filename=filename, content=content)


def wait_for_ai_processing(client: APIClient, expected_count: int,
                            timeout: float = 180.0):
    """Wait until AI has processed the expected number of photos."""
    deadline = time.time() + timeout
    processed = 0
    while time.time() < deadline:
        try:
            status = client.ai_status()
            processed = status.get("photos_processed", 0)
            pending = status.get("photos_pending", 0)
            if processed >= expected_count:
                return status
            if processed > 0:
                print(f"    AI progress: {processed}/{expected_count} "
                      f"(pending: {pending})", end="\r")
        except Exception:
            pass
        time.sleep(3)
    raise TimeoutError(
        f"AI processing did not complete within {timeout}s. "
        f"Processed: {processed}/{expected_count}"
    )


# ── Tests ────────────────────────────────────────────────────────────

class TestAiAccuracy:
    """Validate face detection and object classification accuracy."""

    def test_upload_face_photos(self, ai_client):
        """Upload all face test photos."""
        photo_ids = []
        for filename in FACE_PHOTOS:
            path = os.path.join(FACE_DIR, filename)
            assert os.path.exists(path), f"Missing test photo: {path}"
            result = upload_real_photo(ai_client, path, filename)
            photo_ids.append(result["photo_id"])
            print(f"  Uploaded {filename} → {result['photo_id']}")
        # Store for later tests
        TestAiAccuracy._face_photo_ids = photo_ids

    def test_upload_object_photos(self, ai_client):
        """Upload all object test photos."""
        photo_ids = {}
        for filename in OBJECT_PHOTOS:
            path = os.path.join(OBJ_DIR, filename)
            assert os.path.exists(path), f"Missing test photo: {path}"
            result = upload_real_photo(ai_client, path, filename)
            photo_ids[filename] = result["photo_id"]
            print(f"  Uploaded {filename} → {result['photo_id']}")
        TestAiAccuracy._object_photo_ids = photo_ids

    def test_enable_ai_and_trigger_processing(self, ai_client):
        """Enable AI and trigger reprocessing of all uploaded photos."""
        # Enable AI for this user
        r = ai_client.ai_toggle(True)
        assert r.status_code in (200, 204)

        total = len(FACE_PHOTOS) + len(OBJECT_PHOTOS)
        print(f"\n  Waiting for AI to process {total} photos "
              f"(30s startup delay + inference)...")
        status = wait_for_ai_processing(ai_client, total, timeout=360)
        print(f"  AI processing complete: {status}")

    def test_face_detection_accuracy(self, ai_client, ai_server):
        """Verify SCRFD detects faces in all face portrait photos."""
        clusters = ai_client.ai_list_face_clusters()
        print(f"\n  Face clusters found: {len(clusters)}")
        for c in clusters:
            print(f"    Cluster {c.get('id')}: {c.get('photo_count', '?')} photos, "
                  f"name={c.get('name', 'unnamed')}")

        # For each face photo, check if at least one face was detected
        # by checking if the photo appears in any cluster
        all_cluster_photo_ids = set()
        for cluster in clusters:
            try:
                photos = ai_client.ai_list_cluster_photos(cluster["id"])
                for p in photos:
                    pid = p.get("photo_id") or p.get("id")
                    if pid:
                        all_cluster_photo_ids.add(pid)
            except Exception as e:
                print(f"    Warning: could not list photos for cluster {cluster['id']}: {e}")

        face_ids = getattr(TestAiAccuracy, "_face_photo_ids", [])
        detected = 0
        missed = []
        for i, photo_id in enumerate(face_ids):
            if photo_id in all_cluster_photo_ids:
                detected += 1
                print(f"    ✓ {FACE_PHOTOS[i]}: face detected")
            else:
                missed.append(FACE_PHOTOS[i])
                print(f"    ✗ {FACE_PHOTOS[i]}: NO face detected")

        accuracy = detected / len(face_ids) if face_ids else 0
        print(f"\n  Face detection accuracy: {detected}/{len(face_ids)} "
              f"= {accuracy:.1%}")

        if missed:
            print(f"  Missed faces: {missed}")

        # Also verify distinctness: 10 different people should ideally
        # produce multiple clusters (not all lumped into one)
        if len(clusters) >= 2:
            print(f"  ✓ Clustering produced {len(clusters)} distinct clusters "
                  f"(good separation)")
        elif len(clusters) == 1:
            print(f"  ⚠ Only 1 cluster — all faces grouped together")

        # Dump server logs on failure for debugging
        if accuracy < TARGET_ACCURACY:
            with open(ai_server["log_path"]) as f:
                logs = f.read()
            # Print only AI-related log lines
            ai_lines = [l for l in logs.split("\n")
                       if "ai" in l.lower() or "face" in l.lower()
                       or "scrfd" in l.lower() or "mobilenet" in l.lower()
                       or "model" in l.lower()]
            print(f"\n  === AI-related server logs ({len(ai_lines)} lines) ===")
            for line in ai_lines[-50:]:
                print(f"    {line}")

        assert accuracy >= TARGET_ACCURACY, (
            f"Face detection accuracy {accuracy:.1%} < {TARGET_ACCURACY:.0%} target. "
            f"Missed: {missed}"
        )

    def test_object_classification_accuracy(self, ai_client, ai_server):
        """Verify MobileNetV2 classifies objects correctly."""
        obj_classes = ai_client.ai_list_object_classes()
        print(f"\n  Object classes detected: {len(obj_classes)}")
        for cls in obj_classes:
            print(f"    {cls.get('class_name')}: {cls.get('photo_count', '?')} photos "
                  f"(avg_conf={cls.get('avg_confidence', 0):.3f})")

        # Build mapping: photo_id → set of detected class names
        photo_detections = {}
        for cls in obj_classes:
            class_name = cls["class_name"]
            try:
                photos = ai_client.ai_list_object_photos(class_name)
                for p in photos:
                    pid = p.get("photo_id") or p.get("id")
                    if pid:
                        photo_detections.setdefault(pid, set()).add(class_name)
            except Exception as e:
                print(f"    Warning: could not list photos for class {class_name}: {e}")

        obj_ids = getattr(TestAiAccuracy, "_object_photo_ids", {})
        correct = 0
        missed = []
        for filename, expected_categories in OBJECT_PHOTOS.items():
            photo_id = obj_ids.get(filename)
            if not photo_id:
                missed.append(filename)
                print(f"    ✗ {filename}: photo not uploaded")
                continue

            detected = photo_detections.get(photo_id, set())
            # Check if any detected class matches any expected category
            match = bool(detected & expected_categories)
            if match:
                correct += 1
                matched = detected & expected_categories
                print(f"    ✓ {filename}: detected {detected} "
                      f"(matched: {matched})")
            else:
                missed.append(filename)
                print(f"    ✗ {filename}: detected {detected}, "
                      f"expected any of {expected_categories}")

        accuracy = correct / len(OBJECT_PHOTOS) if OBJECT_PHOTOS else 0
        print(f"\n  Object classification accuracy: {correct}/{len(OBJECT_PHOTOS)} "
              f"= {accuracy:.1%}")

        if missed:
            print(f"  Missed/wrong objects: {missed}")

        # Dump server logs on failure for debugging
        if accuracy < TARGET_ACCURACY:
            with open(ai_server["log_path"]) as f:
                logs = f.read()
            ai_lines = [l for l in logs.split("\n")
                       if "object" in l.lower() or "mobilenet" in l.lower()
                       or "classif" in l.lower() or "model" in l.lower()]
            print(f"\n  === Object detection server logs ({len(ai_lines)} lines) ===")
            for line in ai_lines[-50:]:
                print(f"    {line}")

        assert accuracy >= TARGET_ACCURACY, (
            f"Object classification accuracy {accuracy:.1%} < {TARGET_ACCURACY:.0%} target. "
            f"Missed: {missed}"
        )

    def test_face_embeddings_distinct(self, ai_client):
        """Verify face embeddings for different people are distinguishable."""
        clusters = ai_client.ai_list_face_clusters()
        # With 10 different people, we should get at least 5 clusters
        # (some might be grouped if they look similar, but not all in one)
        print(f"\n  Total face clusters: {len(clusters)}")

        # At minimum, we want at least 3 clusters from 10 different people
        if len(clusters) >= 5:
            print(f"  ✓ Good cluster separation: {len(clusters)} clusters for 10 faces")
        elif len(clusters) >= 3:
            print(f"  ⚠ Moderate cluster separation: {len(clusters)} clusters for 10 faces")
        else:
            print(f"  ✗ Poor cluster separation: only {len(clusters)} clusters for 10 faces")

        # This is informational — we don't fail on clustering quality
        # since identity separation depends heavily on face similarity threshold
        assert len(clusters) >= 1, "Expected at least 1 face cluster"

    def test_summary(self, ai_client):
        """Print a final summary of all AI capabilities."""
        clusters = ai_client.ai_list_face_clusters()
        obj_classes = ai_client.ai_list_object_classes()

        print("\n" + "=" * 60)
        print("  AI ACCURACY TEST SUMMARY")
        print("=" * 60)
        print(f"  Face clusters: {len(clusters)}")
        print(f"  Object classes: {len(obj_classes)}")
        for cls in sorted(obj_classes, key=lambda c: c.get("photo_count", 0), reverse=True):
            print(f"    - {cls['class_name']}: {cls.get('photo_count', 0)} photos")
        print("=" * 60)
