#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Shared Configuration for Simple Photos E2E Tests
# ══════════════════════════════════════════════════════════════════════════════
# Sourced by all test modules. Do NOT execute directly.
#
# Override any value via environment variables before sourcing:
#   MAIN_PORT=9090 source e2e/lib/config.sh
# ══════════════════════════════════════════════════════════════════════════════

# ── Main Server ──────────────────────────────────────────────────────────────
MAIN_PORT="${MAIN_PORT:-8080}"
MAIN_BASE="${MAIN_BASE:-http://localhost:$MAIN_PORT}"
MAIN_API="$MAIN_BASE/api"

# Convenience aliases used by core/concurrent modules
BASE="$MAIN_BASE"
API="$MAIN_API"

# ── Admin Credentials ────────────────────────────────────────────────────────
ADMIN_USER="${ADMIN_USER:-testadmin}"
ADMIN_PASS="${ADMIN_PASS:-TestPass123!}"

# Convenience aliases
USER="$ADMIN_USER"
PASS="$ADMIN_PASS"

# Second user for core tests
USER2="${USER2:-testuser2}"
PASS2="${PASS2:-SecondUser1!}"

# ── Backup Server Configuration ─────────────────────────────────────────────
BACKUP1_PORT="${BACKUP1_PORT:-8081}"
BACKUP1_BASE="${BACKUP1_BASE:-http://localhost:$BACKUP1_PORT}"
BACKUP1_API="$BACKUP1_BASE/api"
BACKUP1_ADDR="${BACKUP1_ADDR:-host.docker.internal:$BACKUP1_PORT}"
BACKUP1_KEY="${BACKUP1_KEY:-fc9bd9bbd9a28246a7e033356e52fbca9fbaa67907c85efbd192e9b137c5102a}"

BACKUP2_PORT="${BACKUP2_PORT:-8082}"
BACKUP2_BASE="${BACKUP2_BASE:-http://localhost:$BACKUP2_PORT}"
BACKUP2_API="$BACKUP2_BASE/api"
BACKUP2_ADDR="${BACKUP2_ADDR:-host.docker.internal:$BACKUP2_PORT}"
BACKUP2_KEY="${BACKUP2_KEY:-41920667e334226bfa5271b9c5a7e8799aaeda398aa2f977d29fe33c953f803f}"

BACKUP3_PORT="${BACKUP3_PORT:-8083}"
BACKUP3_BASE="${BACKUP3_BASE:-http://localhost:$BACKUP3_PORT}"
BACKUP3_API="$BACKUP3_BASE/api"
BACKUP3_ADDR="${BACKUP3_ADDR:-host.docker.internal:$BACKUP3_PORT}"
BACKUP3_KEY="${BACKUP3_KEY:-b3d8ab3874e6333190d05ebb6401ffd943647e45eec416726f07951654d5650b}"

# Backup instance credentials
BK_USER="${BK_USER:-backupadmin}"
BK_PASS="${BK_PASS:-BackupPass1!}"

# ── curl Defaults ────────────────────────────────────────────────────────────
CURL_MAX_TIME="${CURL_MAX_TIME:-15}"
CURL_LONG_MAX_TIME="${CURL_LONG_MAX_TIME:-600}"

# ── Test Counters (shared state) ─────────────────────────────────────────────
FAILURES=0
PASSES=0
WARNINGS=0
TOTAL=0

# ── Test Artifacts ───────────────────────────────────────────────────────────
E2E_TMP_DIR="${E2E_TMP_DIR:-/tmp/simple-photos-e2e}"
mkdir -p "$E2E_TMP_DIR"

# ── Log Directory (overwritten each run for fresh logs) ──────────────────────
# Resolve relative to the e2e/ directory regardless of cwd
_CONFIG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E2E_LOG_DIR="${E2E_LOG_DIR:-$(cd "$_CONFIG_DIR/.." && pwd)/logs}"
mkdir -p "$E2E_LOG_DIR"

# ── Flags (can be set before sourcing or via CLI args) ───────────────────────
VERBOSE="${VERBOSE:-false}"
SKIP_RESET="${SKIP_RESET:-false}"
SKIP_RECOVERY="${SKIP_RECOVERY:-false}"
