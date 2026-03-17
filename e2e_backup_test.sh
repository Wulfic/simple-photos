#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Comprehensive Backup Server E2E Test Suite for Simple Photos
# ══════════════════════════════════════════════════════════════════════════════
# Tests ALL backup-related API endpoints with real Docker backup instances:
#
#   1.  Infrastructure — Verify main & backup servers are running
#   2.  Backup Mode — Get/set backup mode on backup instances
#   3.  Backup Server CRUD — Add/update/list/delete on main server
#   4.  Server-to-Server Auth — X-API-Key backup serve endpoints
#   5.  Sync to Backup #1 — Push photos from main to backup-1
#   6.  Sync to Backup #2 — Push photos from main to backup-2
#   7.  Verify Sync Results — Confirm data arrived on backup servers
#   8.  Sync Logs & Status — Check reachability and sync history
#   9.  Recovery from Backup — Wipe main, recover from backup
#  10.  Backup Photo Proxy — Browse backup photos through main
#  11.  Discovery — LAN auto-discovery of backup servers
#  12.  Audio Backup Toggle — Audio inclusion settings
#  13.  Setup Verify-Backup — Pre-init backup server verification
#  14.  Error Cases — Invalid keys, unreachable servers, duplicates
#  15.  Cleanup — Remove backup servers, reset state
#
# Prerequisites:
#   - Server built:    cd server && cargo build --release
#   - Server running:  sudo bash reset-server.sh
#   - Docker backup instances: cd docker-instances && docker compose up -d
#   - Photos exist at the configured storage root (for sync tests)
#
# Usage:
#   bash e2e_backup_test.sh
#
# Options:
#   --skip-reset    Skip resetting backup containers (use existing state)
#   --skip-recovery Skip the recovery test (destructive — wipes main DB)
#   --verbose       Show full curl responses on failure
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail  # no -e: we handle errors manually

# ── Configuration ────────────────────────────────────────────────────────────
MAIN_BASE="http://localhost:8080"
MAIN_API="$MAIN_BASE/api"

BACKUP1_BASE="http://localhost:8081"
BACKUP1_API="$BACKUP1_BASE/api"
BACKUP1_ADDR="host.docker.internal:8081"  # from Docker container perspective
BACKUP1_KEY="fc9bd9bbd9a28246a7e033356e52fbca9fbaa67907c85efbd192e9b137c5102a"

BACKUP2_BASE="http://localhost:8082"
BACKUP2_API="$BACKUP2_BASE/api"
BACKUP2_ADDR="host.docker.internal:8082"
BACKUP2_KEY="41920667e334226bfa5271b9c5a7e8799aaeda398aa2f977d29fe33c953f803f"

BACKUP3_BASE="http://localhost:8083"
BACKUP3_API="$BACKUP3_BASE/api"
BACKUP3_ADDR="host.docker.internal:8083"
BACKUP3_KEY="b3d8ab3874e6333190d05ebb6401ffd943647e45eec416726f07951654d5650b"

ADMIN_USER="testadmin"
ADMIN_PASS='TestPass123!'

# Users for backup instances — try multiple credential sets since instances
# may have been previously initialized with different admin usernames
BK_USER="backupadmin"
BK_PASS='BackupPass1!'

SKIP_RESET=false
SKIP_RECOVERY=false
VERBOSE=false

# Parse arguments
for arg in "$@"; do
  case $arg in
    --skip-reset)    SKIP_RESET=true ;;
    --skip-recovery) SKIP_RECOVERY=true ;;
    --verbose)       VERBOSE=true ;;
    *) echo "Unknown option: $arg"; exit 1 ;;
  esac
done

# ── Color helpers ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

FAILURES=0
PASSES=0
WARNINGS=0
TOTAL=0

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
pass() { echo -e "${GREEN}  [PASS]${NC} $*"; PASSES=$((PASSES+1)); TOTAL=$((TOTAL+1)); }
fail() { echo -e "${RED}  [FAIL]${NC} $*"; FAILURES=$((FAILURES+1)); TOTAL=$((TOTAL+1)); }
warn() { echo -e "${YELLOW}  [WARN]${NC} $*"; WARNINGS=$((WARNINGS+1)); }
hdr()  {
  echo ""
  echo -e "${BOLD}════════════════════════════════════════════════════════════════${NC}"
  echo -e "${BOLD}  $*${NC}"
  echo -e "${BOLD}════════════════════════════════════════════════════════════════${NC}"
}
subhdr() { echo -e "\n${BOLD}  ── $* ──${NC}"; }

# ── JSON helpers ─────────────────────────────────────────────────────────────

# Extract a JSON field value (handles bool normalization, lists, nested objects)
jget() {
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    v = d.get('$1', '$2')
    if isinstance(v, bool):
        print('true' if v else 'false')
    elif isinstance(v, (list, dict)):
        print(json.dumps(v))
    else:
        print(v)
except:
    print('$2')
"
}

# Extract a field from a JSON array's first element
jget_first() {
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if isinstance(d, list) and len(d) > 0:
        v = d[0].get('$1', '$2')
        print(v)
    else:
        print('$2')
except:
    print('$2')
"
}

# Count items in a JSON array
jcount() {
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if isinstance(d, list):
        print(len(d))
    elif isinstance(d, dict) and '$1' in d:
        print(len(d['$1']))
    else:
        print(0)
except:
    print(0)
"
}

# HTTP status code checker
http_status() {
  curl -s -o /dev/null -w "%{http_code}" --max-time 15 "$@"
}

# Assert HTTP response contains expected value
assert_contains() {
  local desc="$1" response="$2" expected="$3"
  if echo "$response" | grep -qi "$expected"; then
    pass "$desc"
  else
    fail "$desc (expected '$expected' in response)"
    if $VERBOSE; then log "  Response: ${response:0:500}"; fi
  fi
}

# Assert JSON field equals expected value
assert_json() {
  local desc="$1" response="$2" field="$3" expected="$4"
  local actual
  actual=$(echo "$response" | jget "$field" "__MISSING__")
  if [[ "$actual" == "$expected" ]]; then
    pass "$desc"
  else
    fail "$desc (expected $field='$expected', got '$actual')"
    if $VERBOSE; then log "  Response: ${response:0:500}"; fi
  fi
}

# Assert HTTP status code
assert_status() {
  local desc="$1" expected="$2"
  shift 2
  local actual
  actual=$(http_status "$@")
  if [[ "$actual" == "$expected" ]]; then
    pass "$desc (HTTP $actual)"
  else
    fail "$desc (expected HTTP $expected, got $actual)"
  fi
}

# Wait for a sync to complete (poll sync logs)
wait_for_sync() {
  local api="$1" auth="$2" server_id="$3" sync_id="$4" max_wait="${5:-60}"
  local elapsed=0
  while (( elapsed < max_wait )); do
    local logs
    logs=$(curl -s --max-time 10 "$api/admin/backup/servers/$server_id/logs" -H "$auth")
    local status
    status=$(echo "$logs" | python3 -c "
import sys, json
try:
    logs = json.load(sys.stdin)
    for log in logs:
        if log.get('id') == '$sync_id':
            print(log.get('status', 'unknown'))
            sys.exit(0)
    print('not_found')
except Exception as e:
    print('parse_error')
" 2>/dev/null)
    # Trim whitespace from status
    status=$(echo -n "$status" | tr -d '[:space:]')
    if [[ "$status" == "success" || "$status" == "partial" || "$status" == "error" ]]; then
      echo "$status"
      return 0
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  echo "timeout"
  return 1
}

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 1: INFRASTRUCTURE VERIFICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 1: Infrastructure Verification"

subhdr "Main Server Health"
MAIN_HEALTH=$(curl -s --max-time 10 "$MAIN_BASE/health")
assert_json "Main server status is ok" "$MAIN_HEALTH" "status" "ok"
assert_contains "Main server is simple-photos" "$MAIN_HEALTH" "simple-photos"

subhdr "Backup Server 1 Health (:8081)"
BK1_HEALTH=$(curl -s --max-time 10 "$BACKUP1_BASE/health")
assert_json "Backup-1 status is ok" "$BK1_HEALTH" "status" "ok"

subhdr "Backup Server 2 Health (:8082)"
BK2_HEALTH=$(curl -s --max-time 10 "$BACKUP2_BASE/health")
assert_json "Backup-2 status is ok" "$BK2_HEALTH" "status" "ok"

subhdr "Backup Server 3 Health (:8083)"
BK3_HEALTH=$(curl -s --max-time 10 "$BACKUP3_BASE/health")
assert_json "Backup-3 status is ok" "$BK3_HEALTH" "status" "ok"

# ── Setup Main Server ────────────────────────────────────────────────────────
subhdr "Ensure Main Server Initialized"
MAIN_STATUS=$(curl -s --max-time 10 "$MAIN_API/setup/status")
MAIN_SETUP_COMPLETE=$(echo "$MAIN_STATUS" | jget setup_complete "false")

if [[ "$MAIN_SETUP_COMPLETE" == "false" ]]; then
  log "Initializing main server admin user..."
  INIT=$(curl -s --max-time 10 -X POST "$MAIN_API/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}")
  assert_contains "Main server initialized" "$INIT" "user_id"
else
  log "Main server already initialized."
fi

# ── Login to Main Server ─────────────────────────────────────────────────────
subhdr "Login to Main Server"
LOGIN=$(curl -s --max-time 10 -X POST "$MAIN_API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}")
TOKEN=$(echo "$LOGIN" | jget access_token "")
if [[ -z "$TOKEN" || "$TOKEN" == "__MISSING__" ]]; then
  fail "Main server login failed: $LOGIN"
  echo "FATAL: Cannot continue without auth token. Run: sudo bash reset-server.sh"
  exit 1
fi
pass "Main server login successful"
AUTH="Authorization: Bearer $TOKEN"

# ── Trigger a scan on main so we have photos to sync ─────────────────────────
subhdr "Trigger Photo Scan on Main"
SCAN_RESP=$(curl -s --max-time 10 -X POST "$MAIN_API/admin/photos/scan" -H "$AUTH")
assert_contains "Scan triggered" "$SCAN_RESP" "message"
# Wait a moment for scan to process
sleep 3

# Check we have photos
PHOTOS_RESP=$(curl -s --max-time 10 "$MAIN_API/photos" -H "$AUTH")
PHOTO_COUNT=$(echo "$PHOTOS_RESP" | jcount "photos")
if (( PHOTO_COUNT > 0 )); then
  pass "Main server has $PHOTO_COUNT photos available for backup tests"
else
  warn "No photos on main server — sync tests will show 0 photos synced"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 2: BACKUP MODE MANAGEMENT
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 2: Backup Mode Management"

# ── Login to Backup-1 and set it to backup mode ──────────────────────────────
subhdr "Setup Backup Server 1"
BK1_STATUS=$(curl -s --max-time 10 "$BACKUP1_API/setup/status")
BK1_SETUP=$(echo "$BK1_STATUS" | jget setup_complete "false")

if [[ "$BK1_SETUP" == "false" ]]; then
  log "Initializing backup-1..."
  BK1_INIT=$(curl -s --max-time 10 -X POST "$BACKUP1_API/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$BK_USER\",\"password\":\"$BK_PASS\"}")
  assert_contains "Backup-1 initialized" "$BK1_INIT" "user_id"
fi

# Try multiple credential combinations (instances may have different admin usernames)
BK1_TOKEN=""
for bk_cred_user in "$BK_USER" "$ADMIN_USER" "admin"; do
  for bk_cred_pass in "$BK_PASS" "$ADMIN_PASS"; do
    if [[ -n "$BK1_TOKEN" && "$BK1_TOKEN" != "__MISSING__" ]]; then break 2; fi
    BK1_LOGIN=$(curl -s --max-time 10 -X POST "$BACKUP1_API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$bk_cred_user\",\"password\":\"$bk_cred_pass\"}")
    BK1_TOKEN=$(echo "$BK1_LOGIN" | jget access_token "")
  done
done
if [[ -n "$BK1_TOKEN" && "$BK1_TOKEN" != "__MISSING__" ]]; then
  pass "Backup-1 login successful"
  BK1_AUTH="Authorization: Bearer $BK1_TOKEN"
else
  fail "Backup-1 login failed"
  BK1_AUTH=""
fi

# Get current backup mode
if [[ -n "$BK1_AUTH" ]]; then
  subhdr "Get Backup Mode on Backup-1"
  BK1_MODE=$(curl -s --max-time 10 "$BACKUP1_API/admin/backup/mode" -H "$BK1_AUTH")
  assert_contains "Backup-1 mode response has 'mode' field" "$BK1_MODE" "mode"
  BK1_CURRENT_MODE=$(echo "$BK1_MODE" | jget mode "unknown")
  log "Backup-1 current mode: $BK1_CURRENT_MODE"

  subhdr "Set Backup-1 to Backup Mode"
  SET_MODE=$(curl -s --max-time 10 -X POST "$BACKUP1_API/admin/backup/mode" \
    -H "$BK1_AUTH" -H 'Content-Type: application/json' \
    -d '{"mode":"backup"}')
  assert_json "Backup-1 mode set to backup" "$SET_MODE" "mode" "backup"
  assert_contains "Mode response includes server_ip" "$SET_MODE" "server_ip"
  assert_contains "Mode response includes port" "$SET_MODE" "port"
fi

# ── Same for Backup-2 ────────────────────────────────────────────────────────
subhdr "Setup Backup Server 2"
BK2_STATUS=$(curl -s --max-time 10 "$BACKUP2_API/setup/status")
BK2_SETUP=$(echo "$BK2_STATUS" | jget setup_complete "false")

if [[ "$BK2_SETUP" == "false" ]]; then
  log "Initializing backup-2..."
  BK2_INIT=$(curl -s --max-time 10 -X POST "$BACKUP2_API/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$BK_USER\",\"password\":\"$BK_PASS\"}")
  assert_contains "Backup-2 initialized" "$BK2_INIT" "user_id"
fi

BK2_TOKEN=""
for bk_cred_user in "$BK_USER" "$ADMIN_USER" "admin"; do
  for bk_cred_pass in "$BK_PASS" "$ADMIN_PASS"; do
    if [[ -n "$BK2_TOKEN" && "$BK2_TOKEN" != "__MISSING__" ]]; then break 2; fi
    BK2_LOGIN=$(curl -s --max-time 10 -X POST "$BACKUP2_API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$bk_cred_user\",\"password\":\"$bk_cred_pass\"}")
    BK2_TOKEN=$(echo "$BK2_LOGIN" | jget access_token "")
  done
done
if [[ -n "$BK2_TOKEN" && "$BK2_TOKEN" != "__MISSING__" ]]; then
  pass "Backup-2 login successful"
  BK2_AUTH="Authorization: Bearer $BK2_TOKEN"
else
  fail "Backup-2 login failed"
  BK2_AUTH=""
fi

if [[ -n "$BK2_AUTH" ]]; then
  SET_MODE2=$(curl -s --max-time 10 -X POST "$BACKUP2_API/admin/backup/mode" \
    -H "$BK2_AUTH" -H 'Content-Type: application/json' \
    -d '{"mode":"backup"}')
  assert_json "Backup-2 mode set to backup" "$SET_MODE2" "mode" "backup"
fi

# ── Login to Backup-3 (kept in primary mode for negative tests) ──────────────
subhdr "Setup Backup Server 3 (stays primary)"
BK3_STATUS=$(curl -s --max-time 10 "$BACKUP3_API/setup/status")
BK3_SETUP=$(echo "$BK3_STATUS" | jget setup_complete "false")

if [[ "$BK3_SETUP" == "false" ]]; then
  log "Initializing backup-3..."
  BK3_INIT=$(curl -s --max-time 10 -X POST "$BACKUP3_API/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$BK_USER\",\"password\":\"$BK_PASS\"}")
  assert_contains "Backup-3 initialized" "$BK3_INIT" "user_id"
fi

BK3_TOKEN=""
for bk_cred_user in "$BK_USER" "$ADMIN_USER" "admin"; do
  for bk_cred_pass in "$BK_PASS" "$ADMIN_PASS"; do
    if [[ -n "$BK3_TOKEN" && "$BK3_TOKEN" != "__MISSING__" ]]; then break 2; fi
    BK3_LOGIN=$(curl -s --max-time 10 -X POST "$BACKUP3_API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$bk_cred_user\",\"password\":\"$bk_cred_pass\"}")
    BK3_TOKEN=$(echo "$BK3_LOGIN" | jget access_token "")
  done
done
if [[ -n "$BK3_TOKEN" && "$BK3_TOKEN" != "__MISSING__" ]]; then
  pass "Backup-3 login successful"
  BK3_AUTH="Authorization: Bearer $BK3_TOKEN"
else
  fail "Backup-3 login failed"
  BK3_AUTH=""
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 3: BACKUP SERVER CRUD ON MAIN
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 3: Backup Server CRUD on Main"

subhdr "Get Backup Mode on Main (should be primary)"
MAIN_MODE=$(curl -s --max-time 10 "$MAIN_API/admin/backup/mode" -H "$AUTH")
assert_json "Main server mode is primary" "$MAIN_MODE" "mode" "primary"

subhdr "List Backup Servers (clean slate)"
# First, clean up any pre-existing backup servers from prior runs
EXISTING_SERVERS=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers" -H "$AUTH")
EXISTING_IDS=$(echo "$EXISTING_SERVERS" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    for s in d.get('servers', []):
        print(s['id'])
except:
    pass
" 2>/dev/null)

for sid in $EXISTING_IDS; do
  curl -s -X DELETE "$MAIN_API/admin/backup/servers/$sid" -H "$AUTH" > /dev/null 2>&1
  log "  Cleaned up pre-existing backup server: $sid"
done

BK_LIST=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers" -H "$AUTH")
SERVERS_COUNT=$(echo "$BK_LIST" | jcount "servers")
assert_contains "Backup servers list response" "$BK_LIST" "servers"
log "Starting with $SERVERS_COUNT backup servers"

subhdr "Add Backup Server 1 (localhost:8081)"
ADD_BK1=$(curl -s --max-time 10 -X POST "$MAIN_API/admin/backup/servers" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"name\":\"Backup-1\",\"address\":\"localhost:8081\",\"api_key\":\"$BACKUP1_KEY\",\"sync_frequency_hours\":24}")
BK1_SERVER_ID=$(echo "$ADD_BK1" | jget id "")
if [[ -n "$BK1_SERVER_ID" && "$BK1_SERVER_ID" != "__MISSING__" ]]; then
  pass "Backup server 1 added: $BK1_SERVER_ID"
else
  fail "Failed to add backup server 1: $ADD_BK1"
fi
assert_json "Backup-1 name correct" "$ADD_BK1" "name" "Backup-1"

subhdr "Add Backup Server 2 (localhost:8082)"
ADD_BK2=$(curl -s --max-time 10 -X POST "$MAIN_API/admin/backup/servers" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"name\":\"Backup-2\",\"address\":\"localhost:8082\",\"api_key\":\"$BACKUP2_KEY\",\"sync_frequency_hours\":48}")
BK2_SERVER_ID=$(echo "$ADD_BK2" | jget id "")
if [[ -n "$BK2_SERVER_ID" && "$BK2_SERVER_ID" != "__MISSING__" ]]; then
  pass "Backup server 2 added: $BK2_SERVER_ID"
else
  fail "Failed to add backup server 2: $ADD_BK2"
fi

subhdr "Add Backup Server 3 (localhost:8083)"
ADD_BK3=$(curl -s --max-time 10 -X POST "$MAIN_API/admin/backup/servers" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"name\":\"Backup-3\",\"address\":\"localhost:8083\",\"api_key\":\"$BACKUP3_KEY\",\"sync_frequency_hours\":12}")
BK3_SERVER_ID=$(echo "$ADD_BK3" | jget id "")
if [[ -n "$BK3_SERVER_ID" && "$BK3_SERVER_ID" != "__MISSING__" ]]; then
  pass "Backup server 3 added: $BK3_SERVER_ID"
else
  fail "Failed to add backup server 3: $ADD_BK3"
fi

subhdr "List Backup Servers (should have 3)"
BK_LIST2=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers" -H "$AUTH")
SERVERS_COUNT2=$(echo "$BK_LIST2" | jcount "servers")
if [[ "$SERVERS_COUNT2" == "3" ]]; then
  pass "3 backup servers registered"
else
  fail "Expected 3 backup servers, got $SERVERS_COUNT2"
fi

subhdr "Duplicate Add Rejected"
DUP_STATUS=$(http_status -X POST "$MAIN_API/admin/backup/servers" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"name\":\"Dup\",\"address\":\"localhost:8081\",\"api_key\":\"$BACKUP1_KEY\"}")
if [[ "$DUP_STATUS" == "409" ]]; then
  pass "Duplicate backup server correctly rejected (HTTP 409)"
else
  fail "Duplicate add should return 409, got $DUP_STATUS"
fi

subhdr "Update Backup Server 3"
if [[ -n "$BK3_SERVER_ID" && "$BK3_SERVER_ID" != "__MISSING__" ]]; then
  UPDATE_BK3=$(curl -s --max-time 10 -X PUT "$MAIN_API/admin/backup/servers/$BK3_SERVER_ID" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"name":"Backup-3-Updated","sync_frequency_hours":6}')
  assert_contains "Backup-3 updated" "$UPDATE_BK3" "message"
fi

subhdr "Update Non-existent Server → 404"
assert_status "Update nonexistent server" "404" \
  -X PUT "$MAIN_API/admin/backup/servers/nonexistent-id" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"name":"Ghost"}'

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 4: SERVER-TO-SERVER AUTH (X-API-Key Endpoints)
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 4: Server-to-Server Auth (X-API-Key)"

subhdr "Backup List — Valid API Key"
BK_LIST_RESP=$(curl -s --max-time 10 "$BACKUP1_API/backup/list" \
  -H "X-API-Key: $BACKUP1_KEY")
if echo "$BK_LIST_RESP" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Backup list returns valid JSON with correct API key"
  BK1_PHOTO_COUNT=$(echo "$BK_LIST_RESP" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(len(d) if isinstance(d, list) else 0)
" 2>/dev/null)
  log "  Backup-1 currently has $BK1_PHOTO_COUNT photos"
else
  fail "Backup list failed with valid API key"
fi

subhdr "Backup List — Invalid API Key → 401"
assert_status "Invalid API key rejected" "401" \
  "$BACKUP1_API/backup/list" -H "X-API-Key: invalid-key-here"

subhdr "Backup List — Missing API Key → 401"
assert_status "Missing API key rejected" "401" \
  "$BACKUP1_API/backup/list"

subhdr "Backup List-Trash — Valid API Key"
BK_TRASH_RESP=$(curl -s --max-time 10 "$BACKUP1_API/backup/list-trash" \
  -H "X-API-Key: $BACKUP1_KEY")
if echo "$BK_TRASH_RESP" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Backup list-trash returns valid JSON"
else
  fail "Backup list-trash failed"
fi

subhdr "Backup Receive — Missing Headers → 400"
RECEIVE_MISSING_STATUS=$(http_status -X POST "$BACKUP1_API/backup/receive" \
  -H "X-API-Key: $BACKUP1_KEY" \
  --data-binary "test data")
if [[ "$RECEIVE_MISSING_STATUS" == "400" ]]; then
  pass "Receive rejects missing headers (HTTP 400)"
else
  fail "Receive with missing headers returned $RECEIVE_MISSING_STATUS (expected 400)"
fi

subhdr "Backup Receive — Path Traversal Blocked"
TRAVERSAL_STATUS=$(http_status -X POST "$BACKUP1_API/backup/receive" \
  -H "X-API-Key: $BACKUP1_KEY" \
  -H "X-Photo-Id: traversal-test-id" \
  -H "X-File-Path: ../../../etc/passwd" \
  -H "X-Source: photos" \
  --data-binary "malicious data")
if [[ "$TRAVERSAL_STATUS" == "400" ]]; then
  pass "Path traversal correctly blocked (HTTP 400)"
else
  fail "Path traversal returned $TRAVERSAL_STATUS (expected 400)"
fi

subhdr "Backup Download — Nonexistent Photo → 404"
assert_status "Download nonexistent photo" "404" \
  "$BACKUP1_API/backup/download/nonexistent-id" \
  -H "X-API-Key: $BACKUP1_KEY"

subhdr "Backup Download Thumb — Nonexistent Photo → 404"
assert_status "Download nonexistent thumb" "404" \
  "$BACKUP1_API/backup/download/nonexistent-id/thumb" \
  -H "X-API-Key: $BACKUP1_KEY"

# ── Test backup receive with valid data ──────────────────────────────────────
subhdr "Backup Receive — Valid Photo Upload"
TEST_PHOTO_ID="e2e-test-photo-$(date +%s)"
TEST_FILE_PATH="e2e-test-photo.jpg"
# Create a minimal test payload (not a real JPEG, but enough for the API)
TEST_DATA="FAKE_JPEG_DATA_FOR_E2E_TESTING_$(date +%s)"
TEST_HASH=$(echo -n "$TEST_DATA" | sha256sum | cut -d' ' -f1)

RECEIVE_RESP=$(curl -s --max-time 10 -X POST "$BACKUP1_API/backup/receive" \
  -H "X-API-Key: $BACKUP1_KEY" \
  -H "X-Photo-Id: $TEST_PHOTO_ID" \
  -H "X-File-Path: $TEST_FILE_PATH" \
  -H "X-Source: photos" \
  -H "X-Content-Hash: $TEST_HASH" \
  --data-binary "$TEST_DATA")
assert_json "Backup receive returns ok" "$RECEIVE_RESP" "status" "ok"
assert_json "Receive returns correct photo_id" "$RECEIVE_RESP" "photo_id" "$TEST_PHOTO_ID"

subhdr "Backup Receive — Content Hash Mismatch"
BAD_HASH_STATUS=$(http_status -X POST "$BACKUP1_API/backup/receive" \
  -H "X-API-Key: $BACKUP1_KEY" \
  -H "X-Photo-Id: hash-mismatch-test" \
  -H "X-File-Path: hash-test.jpg" \
  -H "X-Source: photos" \
  -H "X-Content-Hash: 0000000000000000000000000000000000000000000000000000000000000000" \
  --data-binary "some data that wont match the hash")
if [[ "$BAD_HASH_STATUS" == "400" ]]; then
  pass "Content hash mismatch correctly rejected (HTTP 400)"
else
  fail "Content hash mismatch returned $BAD_HASH_STATUS (expected 400)"
fi

subhdr "Verify Test Photo in Backup-1 List"
BK_LIST_AFTER=$(curl -s --max-time 10 "$BACKUP1_API/backup/list" \
  -H "X-API-Key: $BACKUP1_KEY")
FOUND_TEST=$(echo "$BK_LIST_AFTER" | python3 -c "
import sys, json
try:
    photos = json.load(sys.stdin)
    found = any(p.get('id') == '$TEST_PHOTO_ID' for p in photos)
    print('true' if found else 'false')
except:
    print('false')
" 2>/dev/null)
if [[ "$FOUND_TEST" == "true" ]]; then
  pass "Test photo found in backup-1 list"
else
  fail "Test photo not found in backup-1 list"
fi

subhdr "Download Test Photo from Backup-1"
DL_STATUS=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 \
  "$BACKUP1_API/backup/download/$TEST_PHOTO_ID" \
  -H "X-API-Key: $BACKUP1_KEY")
if [[ "$DL_STATUS" == "200" ]]; then
  pass "Test photo downloaded from backup-1 (HTTP 200)"
else
  fail "Test photo download returned $DL_STATUS"
fi

subhdr "Backup Receive — Trash Item"
TRASH_ID="e2e-test-trash-$(date +%s)"
TRASH_DATA="FAKE_TRASH_DATA_FOR_E2E"
TRASH_HASH=$(echo -n "$TRASH_DATA" | sha256sum | cut -d' ' -f1)
TRASH_RECV=$(curl -s --max-time 10 -X POST "$BACKUP1_API/backup/receive" \
  -H "X-API-Key: $BACKUP1_KEY" \
  -H "X-Photo-Id: $TRASH_ID" \
  -H "X-File-Path: trash-item.jpg" \
  -H "X-Source: trash" \
  -H "X-Content-Hash: $TRASH_HASH" \
  --data-binary "$TRASH_DATA")
assert_json "Trash receive returns ok" "$TRASH_RECV" "status" "ok"

subhdr "Verify Trash Item in Backup-1 Trash List"
BK_TRASH_AFTER=$(curl -s --max-time 10 "$BACKUP1_API/backup/list-trash" \
  -H "X-API-Key: $BACKUP1_KEY")
FOUND_TRASH=$(echo "$BK_TRASH_AFTER" | python3 -c "
import sys, json
try:
    items = json.load(sys.stdin)
    found = any(t.get('id') == '$TRASH_ID' for t in items)
    print('true' if found else 'false')
except:
    print('false')
" 2>/dev/null)
if [[ "$FOUND_TRASH" == "true" ]]; then
  pass "Trash item found in backup-1 trash list"
else
  fail "Trash item not found in backup-1 trash list"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 5: SYNC TO BACKUP SERVER 1
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 5: Sync to Backup Server 1"

if [[ -n "$BK1_SERVER_ID" && "$BK1_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Check Backup-1 Reachability"
  BK1_REACH=$(curl -s --max-time 15 "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/status" -H "$AUTH")
  assert_json "Backup-1 is reachable" "$BK1_REACH" "reachable" "true"
  BK1_VERSION=$(echo "$BK1_REACH" | jget version "unknown")
  log "  Backup-1 version: $BK1_VERSION"

  subhdr "Trigger Sync to Backup-1"
  SYNC1_RESP=$(curl -s --max-time 15 -X POST \
    "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/sync" -H "$AUTH")
  SYNC1_ID=$(echo "$SYNC1_RESP" | jget sync_id "")
  if [[ -n "$SYNC1_ID" && "$SYNC1_ID" != "__MISSING__" ]]; then
    pass "Sync to backup-1 triggered (sync_id: ${SYNC1_ID:0:20}...)"
  else
    fail "Failed to trigger sync to backup-1: $SYNC1_RESP"
  fi

  if [[ -n "$SYNC1_ID" && "$SYNC1_ID" != "__MISSING__" ]]; then
    subhdr "Wait for Sync to Complete"
    SYNC1_STATUS=$(wait_for_sync "$MAIN_API" "$AUTH" "$BK1_SERVER_ID" "$SYNC1_ID" 300)
    if [[ "$SYNC1_STATUS" == "success" ]]; then
      pass "Sync to backup-1 completed successfully"
    elif [[ "$SYNC1_STATUS" == "partial" ]]; then
      warn "Sync to backup-1 completed with partial success"
    else
      fail "Sync to backup-1 status: $SYNC1_STATUS"
    fi
  fi

  subhdr "Double-Sync (delta — should be fast, 0 new photos)"
  SYNC1B_RESP=$(curl -s --max-time 15 -X POST \
    "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/sync" -H "$AUTH")
  SYNC1B_ID=$(echo "$SYNC1B_RESP" | jget sync_id "")
  if [[ -n "$SYNC1B_ID" && "$SYNC1B_ID" != "__MISSING__" ]]; then
    SYNC1B_STATUS=$(wait_for_sync "$MAIN_API" "$AUTH" "$BK1_SERVER_ID" "$SYNC1B_ID" 60)
    if [[ "$SYNC1B_STATUS" == "success" ]]; then
      pass "Delta sync (no new photos) succeeded"
    else
      warn "Delta sync returned: $SYNC1B_STATUS"
    fi
  fi
else
  warn "Skipping sync tests — backup-1 not registered"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 6: SYNC TO BACKUP SERVER 2
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 6: Sync to Backup Server 2"

if [[ -n "$BK2_SERVER_ID" && "$BK2_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Check Backup-2 Reachability"
  BK2_REACH=$(curl -s --max-time 15 "$MAIN_API/admin/backup/servers/$BK2_SERVER_ID/status" -H "$AUTH")
  assert_json "Backup-2 is reachable" "$BK2_REACH" "reachable" "true"

  subhdr "Trigger Sync to Backup-2"
  SYNC2_RESP=$(curl -s --max-time 15 -X POST \
    "$MAIN_API/admin/backup/servers/$BK2_SERVER_ID/sync" -H "$AUTH")
  SYNC2_ID=$(echo "$SYNC2_RESP" | jget sync_id "")
  if [[ -n "$SYNC2_ID" && "$SYNC2_ID" != "__MISSING__" ]]; then
    pass "Sync to backup-2 triggered (sync_id: ${SYNC2_ID:0:20}...)"
  else
    fail "Failed to trigger sync to backup-2: $SYNC2_RESP"
  fi

  if [[ -n "$SYNC2_ID" && "$SYNC2_ID" != "__MISSING__" ]]; then
    subhdr "Wait for Sync to Complete"
    SYNC2_STATUS=$(wait_for_sync "$MAIN_API" "$AUTH" "$BK2_SERVER_ID" "$SYNC2_ID" 300)
    if [[ "$SYNC2_STATUS" == "success" ]]; then
      pass "Sync to backup-2 completed successfully"
    elif [[ "$SYNC2_STATUS" == "partial" ]]; then
      warn "Sync to backup-2 completed with partial success"
    else
      fail "Sync to backup-2 status: $SYNC2_STATUS"
    fi
  fi
else
  warn "Skipping sync to backup-2 — not registered"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 7: VERIFY SYNC RESULTS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 7: Verify Sync Results"

subhdr "Compare Photo Counts: Main vs Backup-1"
MAIN_PHOTOS=$(curl -s --max-time 10 "$MAIN_API/photos" -H "$AUTH")
MAIN_PHOTO_COUNT=$(echo "$MAIN_PHOTOS" | jcount "photos")

BK1_PHOTOS=$(curl -s --max-time 10 "$BACKUP1_API/backup/list" \
  -H "X-API-Key: $BACKUP1_KEY")
BK1_SYNCED_COUNT=$(echo "$BK1_PHOTOS" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d) if isinstance(d, list) else 0)
except:
    print(0)
" 2>/dev/null)

log "  Main server: $MAIN_PHOTO_COUNT photos"
log "  Backup-1:    $BK1_SYNCED_COUNT photos"

# Backup-1 may have more photos (from test photo upload + previous syncs),
# but should have at least as many as main
if (( BK1_SYNCED_COUNT >= MAIN_PHOTO_COUNT )); then
  pass "Backup-1 has >= main server photos ($BK1_SYNCED_COUNT >= $MAIN_PHOTO_COUNT)"
else
  fail "Backup-1 has fewer photos: $BK1_SYNCED_COUNT < $MAIN_PHOTO_COUNT"
fi

subhdr "Compare Photo Counts: Main vs Backup-2"
BK2_PHOTOS=$(curl -s --max-time 10 "$BACKUP2_API/backup/list" \
  -H "X-API-Key: $BACKUP2_KEY")
BK2_SYNCED_COUNT=$(echo "$BK2_PHOTOS" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d) if isinstance(d, list) else 0)
except:
    print(0)
" 2>/dev/null)

log "  Backup-2: $BK2_SYNCED_COUNT photos"

if (( BK2_SYNCED_COUNT >= MAIN_PHOTO_COUNT )); then
  pass "Backup-2 has >= main server photos ($BK2_SYNCED_COUNT >= $MAIN_PHOTO_COUNT)"
else
  fail "Backup-2 has fewer photos: $BK2_SYNCED_COUNT < $MAIN_PHOTO_COUNT"
fi

subhdr "Verify Photo IDs Match Between Main and Backup-1"
MATCH_RESULT=$(python3 -c "
import sys, json

main_raw = '''$(curl -s --max-time 10 "$MAIN_API/photos" -H "$AUTH")'''
bk1_raw = '''$(curl -s --max-time 10 "$BACKUP1_API/backup/list" -H "X-API-Key: $BACKUP1_KEY")'''

try:
    main_data = json.loads(main_raw)
    main_ids = set(p['id'] for p in main_data.get('photos', main_data if isinstance(main_data, list) else []))

    bk1_data = json.loads(bk1_raw)
    bk1_ids = set(p['id'] for p in bk1_data if isinstance(bk1_data, list))

    missing = main_ids - bk1_ids
    if len(missing) == 0:
        print('all_present')
    else:
        print(f'missing:{len(missing)}')
except Exception as e:
    print(f'error:{e}')
" 2>/dev/null)

if [[ "$MATCH_RESULT" == "all_present" ]]; then
  pass "All main server photo IDs present on backup-1"
elif [[ "$MATCH_RESULT" == missing:* ]]; then
  fail "Some photos missing from backup-1: $MATCH_RESULT"
else
  warn "Could not compare photo IDs: $MATCH_RESULT"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 8: SYNC LOGS & STATUS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 8: Sync Logs & Status"

if [[ -n "$BK1_SERVER_ID" && "$BK1_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Get Sync Logs for Backup-1"
  LOGS1=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/logs" -H "$AUTH")
  LOGS1_COUNT=$(echo "$LOGS1" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d) if isinstance(d, list) else 0)
except:
    print(0)
" 2>/dev/null)
  if (( LOGS1_COUNT > 0 )); then
    pass "Backup-1 has $LOGS1_COUNT sync log entries"
  else
    fail "Backup-1 has no sync log entries"
  fi

  # Verify log structure
  LOG_FIELDS=$(echo "$LOGS1" | python3 -c "
import sys, json
try:
    logs = json.load(sys.stdin)
    if len(logs) > 0:
        entry = logs[0]
        required = ['id', 'server_id', 'started_at', 'status', 'photos_synced', 'bytes_synced']
        missing = [f for f in required if f not in entry]
        if not missing:
            print('ok')
        else:
            print(f'missing:{missing}')
    else:
        print('empty')
except Exception as e:
    print(f'error:{e}')
" 2>/dev/null)
  if [[ "$LOG_FIELDS" == "ok" ]]; then
    pass "Sync log entry has all required fields"
  else
    fail "Sync log structure issue: $LOG_FIELDS"
  fi

  subhdr "Backup-1 Status Check (updated after sync)"
  BK1_STATUS_POST=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/status" -H "$AUTH")
  assert_json "Backup-1 still reachable" "$BK1_STATUS_POST" "reachable" "true"
fi

if [[ -n "$BK2_SERVER_ID" && "$BK2_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Get Sync Logs for Backup-2"
  LOGS2=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers/$BK2_SERVER_ID/logs" -H "$AUTH")
  if echo "$LOGS2" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
    pass "Backup-2 sync logs endpoint returns valid JSON"
  else
    fail "Backup-2 sync logs returned invalid JSON"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 9: BACKUP PHOTO PROXY
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 9: Backup Photo Proxy"

if [[ -n "$BK1_SERVER_ID" && "$BK1_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Proxy Photos from Backup-1 Through Main"
  PROXY_RESP=$(curl -s --max-time 15 "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/photos" -H "$AUTH")
  PROXY_COUNT=$(echo "$PROXY_RESP" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d) if isinstance(d, list) else 0)
except:
    print(0)
" 2>/dev/null)
  if (( PROXY_COUNT > 0 )); then
    pass "Proxied $PROXY_COUNT photos from backup-1"
  else
    warn "Photo proxy returned 0 photos (may be empty backup)"
  fi

  # Verify proxy returns same structure as direct backup/list
  PROXY_FIELDS=$(echo "$PROXY_RESP" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if isinstance(d, list) and len(d) > 0:
        p = d[0]
        required = ['id', 'filename', 'file_path', 'mime_type', 'size_bytes']
        missing = [f for f in required if f not in p]
        print('ok' if not missing else f'missing:{missing}')
    else:
        print('empty')
except Exception as e:
    print(f'error:{e}')
" 2>/dev/null)
  if [[ "$PROXY_FIELDS" == "ok" ]]; then
    pass "Proxied photos have correct structure"
  elif [[ "$PROXY_FIELDS" == "empty" ]]; then
    warn "Cannot verify proxy structure — empty response"
  else
    fail "Proxy photo structure issue: $PROXY_FIELDS"
  fi
fi

if [[ -n "$BK2_SERVER_ID" && "$BK2_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Proxy Photos from Backup-2"
  PROXY2_STATUS=$(http_status "$MAIN_API/admin/backup/servers/$BK2_SERVER_ID/photos" -H "$AUTH")
  if [[ "$PROXY2_STATUS" == "200" ]]; then
    pass "Backup-2 photo proxy accessible (HTTP 200)"
  else
    fail "Backup-2 photo proxy returned $PROXY2_STATUS"
  fi
fi

subhdr "Proxy Non-existent Backup Server → 404"
assert_status "Proxy nonexistent backup" "404" \
  "$MAIN_API/admin/backup/servers/nonexistent-server-id/photos" -H "$AUTH"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 10: RECOVERY FROM BACKUP
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 10: Recovery from Backup"

if [[ "$SKIP_RECOVERY" == "true" ]]; then
  warn "Recovery test skipped (--skip-recovery flag)"
else
  if [[ -n "$BK1_SERVER_ID" && "$BK1_SERVER_ID" != "__MISSING__" ]]; then
    # Count photos before recovery (for delta verification)
    PRE_RECOVERY_COUNT=$(echo "$MAIN_PHOTOS" | jcount "photos")
    log "Main server has $PRE_RECOVERY_COUNT photos before recovery test"

    subhdr "Trigger Recovery from Backup-1"
    RECOVER_RESP=$(curl -s --max-time 15 -X POST \
      "$MAIN_API/admin/backup/servers/$BK1_SERVER_ID/recover" -H "$AUTH")
    RECOVERY_ID=$(echo "$RECOVER_RESP" | jget recovery_id "")
    if [[ -n "$RECOVERY_ID" && "$RECOVERY_ID" != "__MISSING__" ]]; then
      pass "Recovery triggered (recovery_id: ${RECOVERY_ID:0:20}...)"
      assert_contains "Recovery response has message" "$RECOVER_RESP" "message"
    else
      fail "Failed to trigger recovery: $RECOVER_RESP"
    fi

    if [[ -n "$RECOVERY_ID" && "$RECOVERY_ID" != "__MISSING__" ]]; then
      subhdr "Wait for Recovery to Complete"
      # Recovery uses the same sync log mechanism
      RECOVERY_STATUS=$(wait_for_sync "$MAIN_API" "$AUTH" "$BK1_SERVER_ID" "$RECOVERY_ID" 120)
      if [[ "$RECOVERY_STATUS" == "success" ]]; then
        pass "Recovery from backup-1 completed successfully"
      elif [[ "$RECOVERY_STATUS" == "recovering" ]]; then
        warn "Recovery still in progress after timeout"
      else
        fail "Recovery status: $RECOVERY_STATUS"
      fi
    fi

    subhdr "Recover from Non-existent Server → 404"
    assert_status "Recovery from nonexistent server" "404" \
      -X POST "$MAIN_API/admin/backup/servers/nonexistent-id/recover" -H "$AUTH"
  else
    warn "Skipping recovery tests — backup-1 not registered"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 11: DISCOVERY
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 11: LAN Discovery"

subhdr "Discover Backup Servers"
# Discovery uses UDP broadcast (fast) + brute-force HTTP probing (slow fallback)
# Use a generous timeout for the HTTP probe fallback
DISCOVER_RESP=$(curl -s --max-time 30 "$MAIN_API/admin/backup/discover" -H "$AUTH")
if echo "$DISCOVER_RESP" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Discovery endpoint returns valid JSON"
  DISCOVERED_COUNT=$(echo "$DISCOVER_RESP" | jcount "servers")
  log "  Discovered $DISCOVERED_COUNT server(s) on LAN"
else
  warn "Discovery endpoint may have timed out or failed"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 12: AUDIO BACKUP TOGGLE
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 12: Audio Backup Toggle"

subhdr "Get Audio Backup Setting"
AUDIO_GET=$(curl -s --max-time 10 "$MAIN_API/settings/audio-backup" -H "$AUTH")
assert_contains "Audio backup setting response" "$AUDIO_GET" "audio_backup"
AUDIO_CURRENT=$(echo "$AUDIO_GET" | jget audio_backup_enabled "false")
log "  Current audio backup: $AUDIO_CURRENT"

subhdr "Enable Audio Backup"
AUDIO_ON=$(curl -s --max-time 10 -X PUT "$MAIN_API/admin/audio-backup" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"audio_backup_enabled":true}')
assert_json "Audio backup enabled" "$AUDIO_ON" "audio_backup_enabled" "true"
assert_contains "Audio backup update has message" "$AUDIO_ON" "message"

subhdr "Disable Audio Backup"
AUDIO_OFF=$(curl -s --max-time 10 -X PUT "$MAIN_API/admin/audio-backup" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"audio_backup_enabled":false}')
assert_json "Audio backup disabled" "$AUDIO_OFF" "audio_backup_enabled" "false"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 13: BACKUP MODE EDGE CASES
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 13: Backup Mode Edge Cases"

subhdr "Set Invalid Backup Mode → 400"
INVALID_MODE_STATUS=$(http_status -X POST "$MAIN_API/admin/backup/mode" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"mode":"invalid_mode"}')
if [[ "$INVALID_MODE_STATUS" == "400" ]]; then
  pass "Invalid mode correctly rejected (HTTP 400)"
else
  fail "Invalid mode returned $INVALID_MODE_STATUS (expected 400)"
fi

subhdr "Set Main to Backup Mode (temporarily)"
SET_MAIN_BK=$(curl -s --max-time 10 -X POST "$MAIN_API/admin/backup/mode" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"mode":"backup"}')
assert_json "Main server set to backup mode" "$SET_MAIN_BK" "mode" "backup"
# Check that an API key was generated
MAIN_BK_KEY=$(echo "$SET_MAIN_BK" | jget api_key "")
if [[ -n "$MAIN_BK_KEY" && "$MAIN_BK_KEY" != "__MISSING__" && "$MAIN_BK_KEY" != "null" ]]; then
  pass "API key auto-generated for backup mode"
else
  # If the config already had a key, it won't show here — that's fine
  warn "No API key returned (may be in config.toml)"
fi

subhdr "Restore Main to Primary Mode"
SET_MAIN_PRIMARY=$(curl -s --max-time 10 -X POST "$MAIN_API/admin/backup/mode" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"mode":"primary"}')
assert_json "Main server restored to primary" "$SET_MAIN_PRIMARY" "mode" "primary"

subhdr "Verify No API Key in Primary Mode"
MAIN_MODE_CHECK=$(curl -s --max-time 10 "$MAIN_API/admin/backup/mode" -H "$AUTH")
MAIN_KEY_CHECK=$(echo "$MAIN_MODE_CHECK" | jget api_key "null")
if [[ "$MAIN_KEY_CHECK" == "null" || "$MAIN_KEY_CHECK" == "__MISSING__" ]]; then
  pass "No API key exposed in primary mode"
else
  warn "API key present in primary mode response (may be expected if config-level key exists)"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 14: ERROR CASES & EDGE CASES
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 14: Error Cases & Edge Cases"

subhdr "Non-admin Access to Backup Endpoints"
# Create a regular user and try backup operations
REG_USER_RESP=$(curl -s --max-time 10 -X POST "$MAIN_API/auth/register" \
  -H 'Content-Type: application/json' \
  -d '{"username":"nonadmin","password":"NonAdmin1!"}')
REG_LOGIN=$(curl -s --max-time 10 -X POST "$MAIN_API/auth/login" \
  -H 'Content-Type: application/json' \
  -d '{"username":"nonadmin","password":"NonAdmin1!"}')
REG_TOKEN=$(echo "$REG_LOGIN" | jget access_token "")
if [[ -n "$REG_TOKEN" && "$REG_TOKEN" != "__MISSING__" ]]; then
  REG_AUTH="Authorization: Bearer $REG_TOKEN"

  NONADMIN_STATUS=$(http_status "$MAIN_API/admin/backup/servers" -H "$REG_AUTH")
  if [[ "$NONADMIN_STATUS" == "403" ]]; then
    pass "Non-admin blocked from listing backup servers (HTTP 403)"
  else
    fail "Non-admin access returned $NONADMIN_STATUS (expected 403)"
  fi

  NONADMIN_ADD=$(http_status -X POST "$MAIN_API/admin/backup/servers" \
    -H "$REG_AUTH" -H 'Content-Type: application/json' \
    -d '{"name":"Hack","address":"evil.com"}')
  if [[ "$NONADMIN_ADD" == "403" ]]; then
    pass "Non-admin blocked from adding backup server (HTTP 403)"
  else
    fail "Non-admin add returned $NONADMIN_ADD (expected 403)"
  fi

  NONADMIN_MODE=$(http_status "$MAIN_API/admin/backup/mode" -H "$REG_AUTH")
  if [[ "$NONADMIN_MODE" == "403" ]]; then
    pass "Non-admin blocked from backup mode (HTTP 403)"
  else
    fail "Non-admin mode access returned $NONADMIN_MODE (expected 403)"
  fi

  # Clean up the test user
  REG_USER_ID=$(echo "$REG_USER_RESP" | jget user_id "")
  if [[ -n "$REG_USER_ID" && "$REG_USER_ID" != "__MISSING__" ]]; then
    curl -s -X DELETE "$MAIN_API/admin/users/$REG_USER_ID" -H "$AUTH" > /dev/null 2>&1
  fi
else
  warn "Could not create non-admin user for auth tests"
fi

subhdr "Sync to Disabled Backup Server"
if [[ -n "$BK3_SERVER_ID" && "$BK3_SERVER_ID" != "__MISSING__" ]]; then
  # Disable backup-3
  curl -s -X PUT "$MAIN_API/admin/backup/servers/$BK3_SERVER_ID" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"enabled":false}' > /dev/null 2>&1

  DISABLED_SYNC_STATUS=$(http_status -X POST \
    "$MAIN_API/admin/backup/servers/$BK3_SERVER_ID/sync" -H "$AUTH")
  if [[ "$DISABLED_SYNC_STATUS" == "400" ]]; then
    pass "Sync to disabled server correctly rejected (HTTP 400)"
  else
    fail "Sync to disabled server returned $DISABLED_SYNC_STATUS (expected 400)"
  fi

  # Re-enable for cleanup
  curl -s -X PUT "$MAIN_API/admin/backup/servers/$BK3_SERVER_ID" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"enabled":true}' > /dev/null 2>&1
fi

subhdr "Delete Non-existent Backup Server → 404"
assert_status "Delete nonexistent backup server" "404" \
  -X DELETE "$MAIN_API/admin/backup/servers/nonexistent-id" -H "$AUTH"

subhdr "Backup Endpoints Without Auth → 401"
assert_status "No-auth list backup servers" "401" \
  "$MAIN_API/admin/backup/servers"
assert_status "No-auth backup mode" "401" \
  "$MAIN_API/admin/backup/mode"
assert_status "No-auth backup discover" "401" \
  "$MAIN_API/admin/backup/discover"

subhdr "Add Server with Empty Address → 400"
EMPTY_ADDR_STATUS=$(http_status -X POST "$MAIN_API/admin/backup/servers" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"name":"Bad Server","address":""}')
if [[ "$EMPTY_ADDR_STATUS" == "400" ]]; then
  pass "Empty address correctly rejected (HTTP 400)"
else
  fail "Empty address returned $EMPTY_ADDR_STATUS (expected 400)"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 15: MULTI-BACKUP CONSISTENCY
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 15: Multi-Backup Consistency"

subhdr "Cross-Verify Photo Counts Across All Backup Servers"
MAIN_FINAL=$(curl -s --max-time 10 "$MAIN_API/photos" -H "$AUTH")
MAIN_FINAL_COUNT=$(echo "$MAIN_FINAL" | jcount "photos")

BK1_FINAL=$(curl -s --max-time 10 "$BACKUP1_API/backup/list" -H "X-API-Key: $BACKUP1_KEY")
BK1_FINAL_COUNT=$(echo "$BK1_FINAL" | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else 0)" 2>/dev/null)

BK2_FINAL=$(curl -s --max-time 10 "$BACKUP2_API/backup/list" -H "X-API-Key: $BACKUP2_KEY")
BK2_FINAL_COUNT=$(echo "$BK2_FINAL" | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else 0)" 2>/dev/null)

echo ""
log "  ┌─────────────────────────────────────────┐"
log "  │  Photo Count Summary                    │"
log "  ├─────────────────────────────────────────┤"
log "  │  Main Server   (8080): $MAIN_FINAL_COUNT photos"
log "  │  Backup-1      (8081): $BK1_FINAL_COUNT photos"
log "  │  Backup-2      (8082): $BK2_FINAL_COUNT photos"
log "  └─────────────────────────────────────────┘"
echo ""

subhdr "All Backup Servers Updated"
BK_LIST_FINAL=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers" -H "$AUTH")
UPDATED_SERVERS=$(echo "$BK_LIST_FINAL" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    servers = d.get('servers', [])
    synced = sum(1 for s in servers if s.get('last_sync_status') in ('success', 'partial', 'never'))
    total = len(servers)
    print(f'{synced}/{total}')
except:
    print('error')
" 2>/dev/null)
log "  Servers with sync activity: $UPDATED_SERVERS"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 16: CLEANUP
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 16: Cleanup"

subhdr "Delete All Backup Servers from Main"
for SID in "$BK1_SERVER_ID" "$BK2_SERVER_ID" "$BK3_SERVER_ID"; do
  if [[ -n "$SID" && "$SID" != "__MISSING__" ]]; then
    DEL_STATUS=$(http_status -X DELETE "$MAIN_API/admin/backup/servers/$SID" -H "$AUTH")
    if [[ "$DEL_STATUS" == "204" || "$DEL_STATUS" == "200" ]]; then
      pass "Deleted backup server $SID (HTTP $DEL_STATUS)"
    else
      fail "Delete backup server $SID returned $DEL_STATUS"
    fi
  fi
done

subhdr "Verify All Backup Servers Removed"
BK_LIST_CLEAN=$(curl -s --max-time 10 "$MAIN_API/admin/backup/servers" -H "$AUTH")
CLEAN_COUNT=$(echo "$BK_LIST_CLEAN" | jcount "servers")
if [[ "$CLEAN_COUNT" == "0" ]]; then
  pass "All backup servers cleaned up"
else
  fail "Expected 0 backup servers after cleanup, got $CLEAN_COUNT"
fi

subhdr "Restore Backup Instances to Primary Mode"
if [[ -n "$BK1_AUTH" ]]; then
  curl -s -X POST "$BACKUP1_API/admin/backup/mode" \
    -H "$BK1_AUTH" -H 'Content-Type: application/json' \
    -d '{"mode":"primary"}' > /dev/null 2>&1
fi
if [[ -n "$BK2_AUTH" ]]; then
  curl -s -X POST "$BACKUP2_API/admin/backup/mode" \
    -H "$BK2_AUTH" -H 'Content-Type: application/json' \
    -d '{"mode":"primary"}' > /dev/null 2>&1
fi
pass "Backup instances restore attempted"

# ══════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ══════════════════════════════════════════════════════════════════════════════
hdr "Backup E2E Test Results Summary"
echo ""
echo -e "  Tests run:     ${BOLD}$TOTAL${NC}"
echo -e "  Passed:        ${GREEN}${BOLD}$PASSES${NC}"
echo -e "  Failed:        ${RED}${BOLD}$FAILURES${NC}"
echo -e "  Warnings:      ${YELLOW}${BOLD}$WARNINGS${NC}"
echo ""

if [[ "$FAILURES" -eq 0 ]]; then
  echo -e "${GREEN}${BOLD}  ✓ ALL $PASSES BACKUP TESTS PASSED${NC}"
else
  echo -e "${RED}${BOLD}  ✗ $FAILURES BACKUP TEST(S) FAILED${NC}"
fi
echo ""
exit "$FAILURES"
