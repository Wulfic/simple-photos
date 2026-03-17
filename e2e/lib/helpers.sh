#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Shared Helper Functions for Simple Photos E2E Tests
# ══════════════════════════════════════════════════════════════════════════════
# Unified helper library extracted from e2e_test.sh, e2e_backup_test.sh,
# and concurrent_test.sh. Automatically sources config.sh and timing.sh.
#
# Usage (from any module):
#   SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#   source "$SCRIPT_DIR/../lib/helpers.sh"
#
# Or from the repo root:
#   source e2e/lib/helpers.sh
# ══════════════════════════════════════════════════════════════════════════════

# Guard against double-sourcing
[[ -n "${_HELPERS_LOADED:-}" ]] && return 0
_HELPERS_LOADED=1

# Resolve the lib directory regardless of who sources us
_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source dependencies
source "$_LIB_DIR/config.sh"
source "$_LIB_DIR/timing.sh"

# ── Color Codes ──────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# ── Logging & Output ────────────────────────────────────────────────────────

log() {
  echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"
}

pass() {
  echo -e "${GREEN}  [PASS]${NC} $*"
  PASSES=$((PASSES + 1))
  TOTAL=$((TOTAL + 1))
}

fail() {
  echo -e "${RED}  [FAIL]${NC} $*"
  FAILURES=$((FAILURES + 1))
  TOTAL=$((TOTAL + 1))
}

warn() {
  echo -e "${YELLOW}  [WARN]${NC} $*"
  WARNINGS=$((WARNINGS + 1))
}

hdr() {
  # Start a timer for this section (strip spaces for timer key)
  local timer_key
  timer_key=$(echo "$*" | sed 's/[^a-zA-Z0-9_]/_/g')
  # Stop any previous section timer
  if [[ -n "$_CURRENT_SECTION_KEY" ]]; then
    timer_stop "$_CURRENT_SECTION_KEY" > /dev/null 2>&1
  fi
  _CURRENT_SECTION_KEY="$timer_key"

  echo ""
  echo -e "${BOLD}════════════════════════════════════════════════════════════════${NC}"
  echo -e "${BOLD}  $*${NC}"
  echo -e "${BOLD}════════════════════════════════════════════════════════════════${NC}"

  timer_start "$timer_key"
}

subhdr() {
  echo -e "\n${BOLD}  ── $* ──${NC}"
}

# Track the current section for auto-timing
_CURRENT_SECTION_KEY=""

# ── JSON Helpers ─────────────────────────────────────────────────────────────

# Unified JSON field extractor.
#
# Supports:
#   - Top-level keys:     jget "field" "default"
#   - Dot-path access:    jget "a.b.c" "default"
#   - Array indexing:     jget "items.0.id" "default"
#   - Bool normalization: Python True/False → bash true/false
#   - List/dict → JSON string
#
# Input: reads JSON from stdin
# Args:  $1 = key/path, $2 = default value (optional, defaults to __MISSING__)
jget() {
  local path="$1"
  local default="${2:-__MISSING__}"
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    keys = '$path'.split('.')
    for k in keys:
        if isinstance(d, list):
            d = d[int(k)]
        elif isinstance(d, dict):
            d = d.get(k, '$default')
        else:
            d = '$default'
            break
    if isinstance(d, bool):
        print('true' if d else 'false')
    elif isinstance(d, (list, dict)):
        print(json.dumps(d))
    else:
        print(d)
except:
    print('$default')
"
}

# Extract a field from the first element of a JSON array.
#   echo '[{"id":1},{"id":2}]' | jget_first "id" "default"
jget_first() {
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if isinstance(d, list) and len(d) > 0:
        v = d[0].get('$1', '$2')
        if isinstance(v, bool):
            print('true' if v else 'false')
        elif isinstance(v, (list, dict)):
            print(json.dumps(v))
        else:
            print(v)
    else:
        print('$2')
except:
    print('$2')
"
}

# Count items in a JSON array, or items under a dict key.
#   echo '[1,2,3]' | jcount          → 3
#   echo '{"items":[1,2]}' | jcount "items"  → 2
jcount() {
  local key="${1:-}"
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if isinstance(d, list):
        print(len(d))
    elif isinstance(d, dict) and '$key':
        val = d.get('$key', [])
        print(len(val) if isinstance(val, (list, dict)) else 0)
    elif isinstance(d, dict):
        print(len(d))
    else:
        print(0)
except:
    print(0)
"
}

# ── HTTP Helpers ─────────────────────────────────────────────────────────────

# Return only the HTTP status code from a curl request.
#   status=$(http_status -X GET "$url" -H "Auth: ...")
http_status() {
  curl -s -o /dev/null -w "%{http_code}" --max-time "$CURL_MAX_TIME" "$@"
}

# ── Assertion Helpers ────────────────────────────────────────────────────────

# Assert that a response string contains an expected substring (case-insensitive).
#   assert_contains "description" "$response" "expected_substring"
assert_contains() {
  local desc="$1" response="$2" expected="$3"
  if echo "$response" | grep -qi "$expected"; then
    pass "$desc"
  else
    fail "$desc (expected '$expected' in response)"
    if $VERBOSE; then log "  Response: ${response:0:500}"; fi
  fi
}

# Assert that a JSON field in the response equals an expected value.
#   assert_json "description" "$json_response" "field" "expected_value"
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

# Assert that an HTTP request returns an expected status code.
#   assert_status "description" "200" -X GET "$url" -H "Auth: ..."
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

# ── Server Setup Helpers ─────────────────────────────────────────────────────

# Ensure a server is initialized. If not, initialize with given credentials.
#   ensure_server_initialized "$API_URL" "username" "password"
# Returns: 0 if already initialized or newly initialized, 1 on failure.
ensure_server_initialized() {
  local api="$1" username="$2" password="$3"
  local status
  status=$(curl -s --max-time "$CURL_MAX_TIME" "$api/setup/status")
  local setup_complete
  setup_complete=$(echo "$status" | jget setup_complete "false")

  if [[ "$setup_complete" == "true" ]]; then
    log "Server at $api already initialized."
    return 0
  fi

  log "Initializing server at $api..."
  local init
  init=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$api/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$username\",\"password\":\"$password\"}")
  local user_id
  user_id=$(echo "$init" | jget user_id "")
  if [[ -n "$user_id" && "$user_id" != "__MISSING__" ]]; then
    pass "Server initialized at $api (user_id: $user_id)"
    return 0
  else
    fail "Failed to initialize server at $api: $init"
    return 1
  fi
}

# Login to a server and echo the access token. Exits on failure if $4 is "fatal".
#   token=$(login_and_get_token "$API_URL" "username" "password" ["fatal"])
login_and_get_token() {
  local api="$1" username="$2" password="$3" fatal="${4:-}"
  local login_resp
  login_resp=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$api/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$username\",\"password\":\"$password\"}")
  local token
  token=$(echo "$login_resp" | jget access_token "")
  if [[ -n "$token" && "$token" != "__MISSING__" ]]; then
    echo "$token"
    return 0
  else
    if [[ "$fatal" == "fatal" ]]; then
      fail "Login failed at $api: $login_resp"
      echo "FATAL: Cannot continue without auth token." >&2
      exit 1
    fi
    echo ""
    return 1
  fi
}

# Try multiple credential combinations to log in (useful for backup instances).
#   token=$(login_multi_cred "$API_URL" "user1:pass1" "user2:pass2" ...)
login_multi_cred() {
  local api="$1"
  shift
  local token=""
  for cred in "$@"; do
    local cred_user="${cred%%:*}"
    local cred_pass="${cred#*:}"
    token=$(login_and_get_token "$api" "$cred_user" "$cred_pass")
    if [[ -n "$token" ]]; then
      echo "$token"
      return 0
    fi
  done
  echo ""
  return 1
}

# ── Sync Helpers ─────────────────────────────────────────────────────────────

# Poll sync logs until a sync completes or times out.
#   status=$(wait_for_sync "$API_URL" "$AUTH_HEADER" "$server_id" "$sync_id" [max_wait_sec])
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

# ── Conversion Wait Helper ──────────────────────────────────────────────────

# Wait for photo conversions to complete (up to specified seconds).
#   wait_for_conversions "$API_URL" "$AUTH_HEADER" [max_wait_sec]
wait_for_conversions() {
  local api="$1" auth="$2" max_wait="${3:-180}"
  local interval=3
  local iterations=$(( max_wait / interval ))

  for i in $(seq 1 "$iterations"); do
    local cs
    cs=$(curl -s --max-time 5 "$api/photos/conversion-status" -H "$auth" 2>/dev/null || echo "{}")
    local pending converting thumbs
    pending=$(echo "$cs" | jget pending_conversions 0)
    converting=$(echo "$cs" | jget converting false)
    thumbs=$(echo "$cs" | jget missing_thumbnails 0)

    if [[ "$i" -eq 1 ]] || (( i % 10 == 0 )); then
      log "  [$i] pending=$pending converting=$converting thumbs=$thumbs"
    fi

    if [[ "$pending" == "0" && "$converting" == "false" && "$thumbs" == "0" ]]; then
      pass "Conversions complete (waited ~$((i * interval))s)"
      return 0
    fi
    sleep "$interval"
  done
  warn "Conversions still running after ${max_wait}s — continuing"
  return 1
}

# ── Summary / Reporting ─────────────────────────────────────────────────────

# Print the final test summary block. Call at the end of every module.
#   print_summary "Module Name"
print_summary() {
  local module_name="${1:-E2E Test}"

  # Close the last section timer
  if [[ -n "$_CURRENT_SECTION_KEY" ]]; then
    timer_stop "$_CURRENT_SECTION_KEY" > /dev/null 2>&1
    _CURRENT_SECTION_KEY=""
  fi

  hdr "$module_name Results Summary"
  echo ""
  echo -e "  Tests run:     ${BOLD}$TOTAL${NC}"
  echo -e "  Passed:        ${GREEN}${BOLD}$PASSES${NC}"
  echo -e "  Failed:        ${RED}${BOLD}$FAILURES${NC}"
  echo -e "  Warnings:      ${YELLOW}${BOLD}$WARNINGS${NC}"
  echo ""

  if [[ "$FAILURES" -eq 0 ]]; then
    echo -e "${GREEN}${BOLD}  ✓ ALL $PASSES TESTS PASSED${NC}"
  else
    echo -e "${RED}${BOLD}  ✗ $FAILURES TEST(S) FAILED${NC}"
  fi
  echo ""

  # Print timing breakdown
  print_timing_report

  # Output machine-parseable summary line for the master runner
  echo "E2E_RESULT:total=$TOTAL,passes=$PASSES,failures=$FAILURES,warnings=$WARNINGS"
}

# ── Logging to File ──────────────────────────────────────────────────────────

# Set up per-module log file. The log is overwritten each run.
# When running under the master runner (run_all.sh), this is a no-op because
# the master runner handles its own unified log.
#
# Call from each module after sourcing helpers:
#   setup_module_log "core"
#
# The log file is written to e2e/logs/<module_name>.log with ANSI codes
# stripped for clean review.
setup_module_log() {
  local module_name="$1"

  # Skip if running under the master runner — it captures everything already
  [[ "${_E2E_MASTER_RUNNER:-}" == "1" ]] && return 0

  local log_file="$E2E_LOG_DIR/${module_name}.log"
  MODULE_LOG_FILE="$log_file"

  # Redirect all output through tee so it goes to both terminal and log file.
  # We use a FIFO + background process to strip ANSI codes for the file copy.
  local fifo="$E2E_TMP_DIR/.log_fifo_$$"
  rm -f "$fifo"
  mkfifo "$fifo"

  # Background: read from fifo, strip ANSI, write to log file (overwrite)
  sed 's/\x1b\[[0-9;]*m//g' < "$fifo" > "$log_file" &
  _LOG_STRIP_PID=$!

  # Tee stdout+stderr to both terminal (with colors) and fifo (for stripping)
  exec > >(tee "$fifo") 2>&1

  # Cleanup on exit: close the fifo and wait for sed to finish
  trap '_cleanup_module_log' EXIT
}

_cleanup_module_log() {
  # Give tee/sed a moment to flush
  sleep 0.2
  if [[ -n "${_LOG_STRIP_PID:-}" ]]; then
    wait "$_LOG_STRIP_PID" 2>/dev/null
  fi
  if [[ -n "${MODULE_LOG_FILE:-}" ]]; then
    # Print log location to stderr (bypasses redirect) so user sees it
    echo "" >&2
    echo "Log saved to: $MODULE_LOG_FILE" >&2
  fi
}

# ── CLI Argument Parsing ─────────────────────────────────────────────────────

# Parse common CLI arguments. Call from each module's top level.
#   parse_common_args "$@"
parse_common_args() {
  for arg in "$@"; do
    case $arg in
      --skip-reset)    SKIP_RESET=true ;;
      --skip-recovery) SKIP_RECOVERY=true ;;
      --verbose)       VERBOSE=true ;;
      --help)
        echo "Usage: $0 [--skip-reset] [--skip-recovery] [--verbose]"
        exit 0
        ;;
      *)
        # Ignore unknown args (modules may have their own)
        ;;
    esac
  done
}
