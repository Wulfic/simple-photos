"""E2E regression for todo P0-2 / P0-5: AI must not silently run heuristic
fallbacks when the ONNX models are missing.

The previous behaviour was that face detection and object detection silently
fell back to skin-tone / colour-histogram guesses that *looked* like real AI
output to API consumers (filled `tags`, populated `face_detections`).
Operators who never installed the ONNX models had no way to tell their
deployment was running in a degraded mode.

After the fix:

  * `GET /api/ai/status` reports the new fields:
      - `face_model_loaded`
      - `object_model_loaded`
      - `degraded_mode`  (true iff no models AND no allow_heuristic_fallback)
      - `allow_heuristic_fallback`
  * When degraded_mode is true the server logs an `error!` on startup,
    and the detection pipelines return empty results instead of synthesising
    fake detections.

This test runs against whatever the conftest spins up.  The conftest
default config does NOT pre-install models, so we expect:

  * The status fields are present and have boolean types.
  * `degraded_mode == true` ⇔ both model_loaded flags are false AND
    allow_heuristic_fallback is false.
  * `allow_heuristic_fallback` defaults to false.
"""

from __future__ import annotations

import pytest


REQUIRED_FIELDS = [
    pytest.param("face_model_loaded", id="face_model_loaded"),
    pytest.param("object_model_loaded", id="object_model_loaded"),
    pytest.param("degraded_mode", id="degraded_mode"),
    pytest.param("allow_heuristic_fallback", id="allow_heuristic_fallback"),
]


@pytest.mark.parametrize("field", REQUIRED_FIELDS)
def test_status_exposes_field(user_client, field: str):
    """Every new field must be present and boolean-typed."""
    status = user_client.ai_status()
    assert field in status, (
        f"GET /api/ai/status missing required field {field!r}.  "
        "Operators cannot tell a degraded deployment from a healthy one "
        "without this signal."
    )
    assert isinstance(status[field], bool), (
        f"Field {field!r} should be bool, got {type(status[field]).__name__}"
    )


def test_degraded_mode_consistent_with_model_flags(user_client):
    """`degraded_mode` is computed from `(face|object)_model_loaded` and
    `allow_heuristic_fallback`. Lock that contract in."""
    status = user_client.ai_status()
    expected = (
        not status["face_model_loaded"]
        and not status["object_model_loaded"]
        and not status["allow_heuristic_fallback"]
    )
    assert status["degraded_mode"] == expected, (
        f"degraded_mode {status['degraded_mode']!r} disagrees with derived "
        f"expectation {expected!r}; status={status}"
    )


def test_default_config_does_not_allow_heuristic_fallback(user_client):
    """Production default must NOT enable heuristic fallback.  This is the
    main behavioural switch fixing P0-2 / P0-5: shipping with this true
    re-introduces the silent-fake-AI bug."""
    status = user_client.ai_status()
    assert status["allow_heuristic_fallback"] is False, (
        "Default AiConfig.allow_heuristic_fallback must be false.  "
        "Setting it true makes the heuristic detectors run silently "
        "again, which is the bug P0-2 / P0-5 fixed."
    )
