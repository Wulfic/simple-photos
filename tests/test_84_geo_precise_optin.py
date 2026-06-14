"""
Test 84: Precise (street-level) geocoding opt-in contract.

Precise reverse geocoding is the ONLY geo feature that sends a user's
coordinates to a third party (OpenStreetMap/Photon).  Its privacy contract is
therefore security-critical and verified here end-to-end:

  1. `precise_enabled` is exposed in geo settings and defaults to OFF.
  2. The per-user opt-in can be turned on and back off via the settings API.
  3. Toggling other geo settings does not implicitly flip precise.
  4. Multi-user isolation: one user's opt-in never affects another user.

The actual provider HTTP call / response parsing / rate-limiting is covered by
Rust unit tests (geo::precise) — these would require a live external service,
so they are intentionally not exercised here.
"""

from helpers import APIClient


class TestGeoPreciseOptIn:
    def test_precise_defaults_off(self, user_client: APIClient):
        status = user_client.geo_settings()
        assert "precise_enabled" in status, "geo settings must expose precise_enabled"
        assert status["precise_enabled"] is False, "precise must be OFF by default"

    def test_precise_toggle_on_and_off(self, user_client: APIClient):
        # Opt in.
        r = user_client.geo_update_settings(precise_enabled=True)
        r.raise_for_status()
        assert user_client.geo_settings()["precise_enabled"] is True

        # Opt back out.
        r = user_client.geo_update_settings(precise_enabled=False)
        r.raise_for_status()
        assert user_client.geo_settings()["precise_enabled"] is False

    def test_enabling_geo_does_not_enable_precise(self, user_client: APIClient):
        # Ensure precise is off, then flip the (separate) geo master toggle.
        user_client.geo_update_settings(precise_enabled=False).raise_for_status()
        user_client.geo_update_settings(enabled=True).raise_for_status()
        status = user_client.geo_settings()
        assert status["enabled"] is True
        assert status["precise_enabled"] is False, \
            "enabling geolocation must not implicitly enable precise geocoding"

    def test_precise_opt_in_is_per_user(
        self, user_client: APIClient, second_user_client: APIClient
    ):
        # User A opts in; user B must stay opted out.
        user_client.geo_update_settings(precise_enabled=True).raise_for_status()
        assert user_client.geo_settings()["precise_enabled"] is True
        assert second_user_client.geo_settings()["precise_enabled"] is False, \
            "precise opt-in must not leak across users"
