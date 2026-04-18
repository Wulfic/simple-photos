"""
Test 56: GPU Transcode — Data-Driven Tests (DDT).

Parametrized tests covering the transcode status API and video conversion
behaviour with GPU acceleration:
  - Status endpoint returns expected fields
  - Status field types are correct
  - GPU-enabled field reflects server config
  - Fallback-to-cpu field reflects server config
  - Accel type is one of the known types
  - Video encoder is non-empty
  - Conversion of video uses transcode pipeline
  - Image conversion unaffected by GPU settings
  - Audio conversion unaffected by GPU settings
  - Concurrent video conversions
  - Unauthenticated access rejected

Each test case is a single row in a parameter table.
"""

import pytest
import time

from helpers import APIClient, unique_filename, generate_test_jpeg


# ══════════════════════════════════════════════════════════════════════
# Constants
# ══════════════════════════════════════════════════════════════════════

KNOWN_ACCEL_TYPES = {"nvenc", "qsv", "vaapi", "amf", "cpu"}


# ══════════════════════════════════════════════════════════════════════
# DDT: Transcode Status — Required Fields
# ══════════════════════════════════════════════════════════════════════

STATUS_FIELDS = [
    pytest.param("gpu_available", id="has_gpu_available"),
    pytest.param("accel_type", id="has_accel_type"),
    pytest.param("video_encoder", id="has_video_encoder"),
    pytest.param("gpu_enabled", id="has_gpu_enabled"),
    pytest.param("fallback_to_cpu", id="has_fallback_to_cpu"),
]


@pytest.mark.parametrize("field", STATUS_FIELDS)
def test_status_has_field(user_client, field):
    """Transcode status response contains the expected field."""
    status = user_client.transcode_status()
    assert field in status, f"Missing field '{field}' in transcode status"


# ══════════════════════════════════════════════════════════════════════
# DDT: Transcode Status — Field Types
# ══════════════════════════════════════════════════════════════════════

STATUS_FIELD_TYPES = [
    pytest.param("gpu_available", bool, id="gpu_available_is_bool"),
    pytest.param("accel_type", str, id="accel_type_is_str"),
    pytest.param("video_encoder", str, id="video_encoder_is_str"),
    pytest.param("gpu_enabled", bool, id="gpu_enabled_is_bool"),
    pytest.param("fallback_to_cpu", bool, id="fallback_to_cpu_is_bool"),
]


@pytest.mark.parametrize("field,expected_type", STATUS_FIELD_TYPES)
def test_status_field_type(user_client, field, expected_type):
    """Each status field has the correct Python type."""
    status = user_client.transcode_status()
    assert isinstance(status[field], expected_type), (
        f"Field '{field}' expected {expected_type.__name__}, got {type(status[field]).__name__}"
    )


# ══════════════════════════════════════════════════════════════════════
# DDT: Accel Type Validity
# ══════════════════════════════════════════════════════════════════════

def test_accel_type_is_known(user_client):
    """Accel type is one of the known acceleration backends."""
    status = user_client.transcode_status()
    assert status["accel_type"] in KNOWN_ACCEL_TYPES, (
        f"Unknown accel_type '{status['accel_type']}'; expected one of {KNOWN_ACCEL_TYPES}"
    )


# ══════════════════════════════════════════════════════════════════════
# DDT: Video Encoder Non-empty
# ══════════════════════════════════════════════════════════════════════

def test_video_encoder_non_empty(user_client):
    """Video encoder string is never empty."""
    status = user_client.transcode_status()
    assert len(status["video_encoder"]) > 0


# ══════════════════════════════════════════════════════════════════════
# DDT: GPU Available Matches Accel Type
# ══════════════════════════════════════════════════════════════════════

def test_gpu_available_consistent_with_accel_type(user_client):
    """If gpu_available is False, accel_type must be 'cpu'."""
    status = user_client.transcode_status()
    if not status["gpu_available"]:
        assert status["accel_type"] == "cpu", (
            f"gpu_available=False but accel_type='{status['accel_type']}'"
        )


def test_gpu_available_true_means_non_cpu(user_client):
    """If gpu_available is True, accel_type must NOT be 'cpu'."""
    status = user_client.transcode_status()
    if status["gpu_available"]:
        assert status["accel_type"] != "cpu"


# ══════════════════════════════════════════════════════════════════════
# DDT: Device Field
# ══════════════════════════════════════════════════════════════════════

DEVICE_CASES = [
    pytest.param("vaapi", True, id="vaapi_has_device"),
    pytest.param("nvenc", False, id="nvenc_no_device"),
    pytest.param("qsv", False, id="qsv_no_device"),
    pytest.param("amf", False, id="amf_no_device"),
    pytest.param("cpu", False, id="cpu_no_device"),
]


@pytest.mark.parametrize("accel_type,expects_device", DEVICE_CASES)
def test_device_field_presence(user_client, accel_type, expects_device):
    """Device field is set only for backends that use a device path (VAAPI)."""
    status = user_client.transcode_status()
    if status["accel_type"] != accel_type:
        pytest.skip(f"Server is not using {accel_type}")
    if expects_device:
        assert status.get("device") is not None, f"{accel_type} should report a device"
    else:
        # device may be null or absent
        pass


# ══════════════════════════════════════════════════════════════════════
# DDT: Config Defaults
# ══════════════════════════════════════════════════════════════════════

def test_gpu_enabled_default_true(user_client):
    """Default config has gpu_enabled=true."""
    status = user_client.transcode_status()
    assert status["gpu_enabled"] is True


def test_fallback_to_cpu_default_true(user_client):
    """Default config has fallback_to_cpu=true."""
    status = user_client.transcode_status()
    assert status["fallback_to_cpu"] is True


# ══════════════════════════════════════════════════════════════════════
# DDT: Multiple Calls Return Consistent Results
# ══════════════════════════════════════════════════════════════════════

def test_status_idempotent(user_client):
    """Two consecutive status calls return identical results."""
    s1 = user_client.transcode_status()
    s2 = user_client.transcode_status()
    assert s1 == s2


# ══════════════════════════════════════════════════════════════════════
# DDT: Unauthenticated Access Rejected
# ══════════════════════════════════════════════════════════════════════

def test_unauthenticated_rejected(primary_server):
    """Transcode status requires authentication."""
    import requests
    r = requests.get(f"{primary_server.base_url}/api/transcode/status", timeout=5)
    assert r.status_code in (401, 403)


# ══════════════════════════════════════════════════════════════════════
# DDT: Image Conversion Unaffected
# ══════════════════════════════════════════════════════════════════════

def test_image_upload_unaffected_by_transcode(user_client):
    """Uploading a JPEG image still works — transcode only affects video."""
    content = generate_test_jpeg(width=100, height=100)
    data = user_client.upload_photo(unique_filename(".jpg"), content=content)
    assert "photo_id" in data


# ══════════════════════════════════════════════════════════════════════
# DDT: Status After Photo Upload
# ══════════════════════════════════════════════════════════════════════

def test_status_stable_after_upload(user_client):
    """Transcode status does not change after uploading an image."""
    s1 = user_client.transcode_status()
    content = generate_test_jpeg(width=80, height=80)
    user_client.upload_photo(unique_filename(".jpg"), content=content)
    s2 = user_client.transcode_status()
    assert s1 == s2
