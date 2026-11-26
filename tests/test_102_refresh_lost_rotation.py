"""
Test 102: refresh-token rotation survives a lost response (TODO.md §4 N1a).

The bug (live repro 2026-11-26): the server rotated the phone's refresh token,
then went HTTP-dead before the response arrived. The phone kept the old token;
the rotation had revoked it and the hourly cleanup deleted it, so every later
refresh answered "invalid" and the app 401'd until the user logged in again.

Contract after N1a:
  - Replaying a rotated token whose successor was never presented is a lost
    response: the server reissues a pair and kills the unused successor.
  - Replaying a rotated token whose successor WAS presented is still theft:
    every token the user holds is revoked.
  - A successor killed by a recovery is a stolen token if it ever shows up:
    presenting it revokes everything (the attacker-replays-first case).
  - An unknown token is audited — it used to fail with no trace at all.
"""

import time

import pytest
from helpers import APIClient


def _refresh_raw(client: APIClient, token: str):
    """POST /api/auth/refresh with an explicit token, without touching client state."""
    return client.post("/api/auth/refresh", json_data={"refresh_token": token})


def _lose_one_rotation(client: APIClient) -> tuple[str, str]:
    """Rotate the client's token and drop the response, as a dead network would.

    Returns (presented, lost_successor). The client keeps `presented`.
    """
    presented = client.refresh_token
    r = _refresh_raw(client, presented)
    assert r.status_code == 200, f"setup rotation failed: {r.status_code} {r.text[:200]}"
    return presented, r.json()["refresh_token"]


def _wait_for_audit(admin: APIClient, event_type: str, ip: str, needle: str,
                    timeout: float = 15.0) -> dict:
    """Poll the audit log for an `event_type` row from `ip` whose details mention `needle`."""
    deadline = time.time() + timeout
    seen = []
    while time.time() < deadline:
        r = admin.get("/api/admin/audit-logs", params={"event_type": event_type, "limit": 200})
        assert r.status_code == 200, f"audit-logs {r.status_code}: {r.text[:200]}"
        seen = r.json()["logs"]
        for row in seen:
            if row.get("ip_address") == ip and needle in (row.get("details") or ""):
                return row
        time.sleep(0.5)
    pytest.fail(
        f"no '{event_type}' row from {ip} mentioning '{needle}' within {timeout}s; "
        f"saw {[(e.get('ip_address'), (e.get('details') or '')[:60]) for e in seen[:5]]}"
    )


class TestRefreshLostRotation:

    def test_replay_after_lost_response_recovers(self, user_client, admin_client):
        presented, _lost = _lose_one_rotation(user_client)

        r = _refresh_raw(user_client, presented)
        assert r.status_code == 200, (
            "a token whose rotation response was lost must still redeem — "
            f"got {r.status_code} {r.text[:200]}"
        )
        recovered = r.json()
        assert recovered["refresh_token"] not in (presented, _lost)

        # The recovered pair is a working session, and its chain rotates on.
        user_client.access_token = recovered["access_token"]
        user_client.refresh_token = recovered["refresh_token"]
        assert user_client.get("/api/photos").status_code == 200
        user_client.refresh()
        assert user_client.get("/api/photos").status_code == 200

        _wait_for_audit(admin_client, "token_refresh", user_client._fake_ip, "lost_rotation")

    def test_lost_response_can_be_recovered_repeatedly(self, user_client):
        """The phone may lose the recovery response too — the same token redeems again."""
        presented, _ = _lose_one_rotation(user_client)
        assert _refresh_raw(user_client, presented).status_code == 200  # response lost again
        r = _refresh_raw(user_client, presented)
        assert r.status_code == 200, f"second recovery: {r.status_code} {r.text[:200]}"
        user_client.refresh_token = r.json()["refresh_token"]
        user_client.refresh()

    def test_replay_after_successor_used_is_theft(self, user_client):
        """The successor reached its owner and was used ⇒ the old token is stolen."""
        stale = user_client.refresh_token
        user_client.refresh()  # successor delivered…
        user_client.refresh()  # …and presented
        live = user_client.refresh_token

        assert _refresh_raw(user_client, stale).status_code == 401
        # Theft revokes the whole session, including the legit live token.
        assert _refresh_raw(user_client, live).status_code == 401

    def test_killed_successor_presented_later_is_theft(self, user_client):
        """Attacker replays the old token first; the legit holder's next refresh trips theft."""
        stolen, legit_successor = _lose_one_rotation(user_client)

        attacker = _refresh_raw(user_client, stolen)
        assert attacker.status_code == 200  # indistinguishable from a lost response
        attacker_token = attacker.json()["refresh_token"]

        assert _refresh_raw(user_client, legit_successor).status_code == 401
        assert _refresh_raw(user_client, attacker_token).status_code == 401, (
            "presenting a successor a recovery killed must revoke the attacker's pair too"
        )

    def test_unknown_refresh_token_is_audited(self, primary_server, admin_client):
        client = APIClient(primary_server.base_url)
        r = _refresh_raw(client, "00000000-0000-4000-8000-000000000000")
        assert r.status_code == 401
        _wait_for_audit(admin_client, "login_failure", client._fake_ip, "refresh_not_found")
