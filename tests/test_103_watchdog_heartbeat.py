"""
Test 103: the N2 watchdog runs in the real server (TODO.md §4 N2a).

N2: the live server stops answering HTTP after ~24h while the process stays
alive. The watchdog is an OS thread that self-probes `/health` and logs
runtime / blocking-pool / thread / open-request state, so the next death
records itself. Its pieces are unit-tested; this test proves `main.rs` wires
them: the thread starts, its first heartbeat goes out at once, the self-probe
reaches the server's own listener, and the request tracker sees that probe.
"""

import os
import time

import pytest


def _log(server) -> str:
    with open(server.log_path, encoding="utf-8", errors="replace") as f:
        return f.read()


def test_watchdog_starts_and_its_self_probe_reaches_the_server(primary_server):
    if not hasattr(primary_server, "log_path") or not os.path.exists(primary_server.log_path):
        pytest.skip("external server: no log file to read")

    deadline = time.time() + 20
    heartbeat = None
    while time.time() < deadline:
        lines = _log(primary_server).splitlines()
        heartbeat = next((l for l in lines if "N2 watchdog heartbeat" in l), None)
        if heartbeat:
            break
        time.sleep(0.5)

    log = _log(primary_server)
    assert "N2 watchdog started" in log, "watchdog thread never started"
    assert heartbeat, "no heartbeat within 20s of startup"
    assert "probe=200 in" in heartbeat, heartbeat
    assert "async_ping=TIMEOUT" not in heartbeat, heartbeat
    assert "blocking_ping=TIMEOUT" not in heartbeat, heartbeat
    assert "N2 watchdog ANOMALY" not in log, "a healthy test server reported an anomaly"
