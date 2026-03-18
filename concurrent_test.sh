#!/usr/bin/env bash
# ════════════════════════════════════════════════════════════════════════════════
# Concurrent 5-User Stress Test for Simple Photos Server
# ════════════════════════════════════════════════════════════════════════════════
# Validates dual-pool SQLite architecture and spawn_blocking under concurrent
# multi-user load. Creates 5 users, runs simultaneous operations, checks for
# errors, hangs, and response time regressions.
# ════════════════════════════════════════════════════════════════════════════════
set -eo pipefail

BASE="http://localhost:8080"
API="$BASE/api"
RESULTS_DIR="${TMPDIR:-/tmp}/concurrent_test_results"
MAX_TIME=30   # Per-request timeout in seconds
ROUNDS=5      # Number of rounds each user performs

# Colors
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

TOTAL=0; PASSED=0; FAILED=0; WARNINGS=0

pass()  { TOTAL=$((TOTAL+1)); PASSED=$((PASSED+1));     echo -e "  ${GREEN}✓${NC} $1"; }
fail()  { TOTAL=$((TOTAL+1)); FAILED=$((FAILED+1));     echo -e "  ${RED}✗${NC} $1"; }
warn()  { WARNINGS=$((WARNINGS+1));                       echo -e "  ${YELLOW}⚠${NC} $1"; }
hdr()   { echo -e "\n${BOLD}${CYAN}═══ $1 ═══${NC}"; }
log()   { echo -e "  ${CYAN}→${NC} $1"; }

jget() {
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    keys = '$1'.split('.')
    for k in keys:
        d = d[k] if isinstance(d, dict) else d[int(k)]
    print(d)
except:
    print('${2:-__MISSING__}')
"
}

http_status() {
  curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" "$@"
}

# ════════════════════════════════════════════════════════════════════════════════
# SETUP: Clean results directory
# ════════════════════════════════════════════════════════════════════════════════
rm -rf "$RESULTS_DIR"
mkdir -p "$RESULTS_DIR"

echo -e "${BOLD}╔════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  Concurrent 5-User Stress Test — Simple Photos        ║${NC}"
echo -e "${BOLD}╚════════════════════════════════════════════════════════╝${NC}"

# ════════════════════════════════════════════════════════════════════════════════
# PHASE 1: Server health check
# ════════════════════════════════════════════════════════════════════════════════
hdr "Phase 1: Server Health Check"

HEALTH=$(curl -s --max-time 10 "$BASE/health" 2>/dev/null || echo '{"error":"unreachable"}')
if echo "$HEALTH" | grep -q '"ok"'; then
  pass "Server is healthy"
else
  fail "Server is not responding — aborting"
  echo "$HEALTH"
  exit 1
fi

STATUS=$(curl -s --max-time 10 "$API/setup/status")
SETUP_DONE=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('setup_complete',False))" 2>/dev/null)

# ════════════════════════════════════════════════════════════════════════════════
# PHASE 2: Initialize server + create 5 users
# ════════════════════════════════════════════════════════════════════════════════
hdr "Phase 2: Create 5 Users"

USERS=("admin1" "alice" "bob" "carol" "dave")
PASSWORDS=("AdminPass1!" "AlicePass1!" "BobbyPass1!" "CarolPass1!" "DaveyPass1!")
declare -a TOKENS
declare -a USER_IDS

if [[ "$SETUP_DONE" == "False" || "$SETUP_DONE" == "false" ]]; then
  # Initialize admin user
  INIT=$(curl -s --max-time "$MAX_TIME" -X POST "$API/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[0]}\",\"password\":\"${PASSWORDS[0]}\"}")
  ADMIN_ID=$(echo "$INIT" | jget user_id "")
  if [[ -n "$ADMIN_ID" && "$ADMIN_ID" != "__MISSING__" ]]; then
    pass "Admin '${USERS[0]}' created (ID: $ADMIN_ID)"
    USER_IDS[0]="$ADMIN_ID"
  else
    fail "Admin creation failed: $INIT"
    exit 1
  fi
else
  log "Server already initialized — using existing admin"
  USERS[0]="testadmin"
  PASSWORDS[0]='TestPass123!'
fi

# Login admin to get token
LOGIN=$(curl -s --max-time "$MAX_TIME" -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"${USERS[0]}\",\"password\":\"${PASSWORDS[0]}\"}")
TOKENS[0]=$(echo "$LOGIN" | jget access_token "")
if [[ -z "${TOKENS[0]}" || "${TOKENS[0]}" == "__MISSING__" ]]; then
  fail "Admin login failed: $LOGIN"
  exit 1
fi
pass "Admin '${USERS[0]}' logged in"

# Create remaining 4 users (using admin endpoint)
for i in 1 2 3 4; do
  CREATE=$(curl -s --max-time "$MAX_TIME" -X POST "$API/admin/users" \
    -H "Authorization: Bearer ${TOKENS[0]}" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[$i]}\",\"password\":\"${PASSWORDS[$i]}\",\"role\":\"user\"}")
  URID=$(echo "$CREATE" | jget user_id "")
  if [[ -n "$URID" && "$URID" != "__MISSING__" ]]; then
    pass "User '${USERS[$i]}' created (ID: $URID)"
    USER_IDS[$i]="$URID"
  else
    # Might already exist if re-running
    warn "User '${USERS[$i]}' creation response: ${CREATE:0:120}"
  fi

  # Login each user
  L=$(curl -s --max-time "$MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[$i]}\",\"password\":\"${PASSWORDS[$i]}\"}")
  TOKENS[$i]=$(echo "$L" | jget access_token "")
  if [[ -n "${TOKENS[$i]}" && "${TOKENS[$i]}" != "__MISSING__" ]]; then
    pass "User '${USERS[$i]}' logged in"
  else
    fail "User '${USERS[$i]}' login failed"
  fi
done

# Trigger a photo scan so there's data to work with
hdr "Phase 2b: Trigger Photo Scan"
log "Scanning photos (may take a while for FFmpeg thumbnails)..."
SCAN=$(curl -s --max-time 600 -X POST "$API/admin/photos/scan" \
  -H "Authorization: Bearer ${TOKENS[0]}")
REGISTERED=$(echo "$SCAN" | jget registered 0)
log "Scan result: registered=$REGISTERED"

# Wait for conversions briefly (up to 60s)
for i in $(seq 1 20); do
  CS=$(curl -s --max-time 5 "$API/photos/conversion-status" \
    -H "Authorization: Bearer ${TOKENS[0]}" 2>/dev/null || echo "{}")
  P=$(echo "$CS" | jget pending_conversions 0)
  A=$(echo "$CS" | jget converting false)
  if [[ "$P" == "0" && "$A" == "false" ]]; then
    log "Conversions complete (waited ~$((i*3))s)"
    break
  fi
  sleep 3
done

# ════════════════════════════════════════════════════════════════════════════════
# PHASE 3: Concurrent operations — all 5 users at once
# ════════════════════════════════════════════════════════════════════════════════
hdr "Phase 3: Concurrent Multi-User Operations ($ROUNDS rounds)"

# Each user runs this workload function in the background.
# It writes results to a per-user file in RESULTS_DIR.
user_workload() {
  set +e  # Disable errexit — we track errors manually
  local user_idx=$1
  local username="${USERS[$user_idx]}"
  local token="${TOKENS[$user_idx]}"
  local auth="Authorization: Bearer $token"
  local result_file="$RESULTS_DIR/user_${user_idx}.log"
  local errors=0
  local requests=0
  local start_time=$(date +%s%N)

  echo "USER=$username" > "$result_file"

  for round in $(seq 1 "$ROUNDS"); do
    # ---- 1. List photos ----
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
      "$API/photos" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=list_photos status=$status" >> "$result_file"
      ((errors++))
    fi

    # ---- 2. List photos with pagination ----
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
      "$API/photos?limit=5&offset=0" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=list_photos_paginated status=$status" >> "$result_file"
      ((errors++))
    fi

    # ---- 3. Get first photo details (thumb, file, web) ----
    photos_json=$(curl -s --max-time "$MAX_TIME" "$API/photos" -H "$auth" 2>/dev/null)
    first_id=$(echo "$photos_json" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    photos = d.get('photos', [])
    print(photos[0]['id'] if photos else '')
except:
    print('')
" 2>/dev/null)

    if [[ -n "$first_id" ]]; then
      # Serve thumbnail
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        "$API/photos/$first_id/thumb" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" && "$status" != "404" ]]; then
        echo "FAIL round=$round op=thumb status=$status" >> "$result_file"
        ((errors++))
      fi

      # Serve file
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        "$API/photos/$first_id/file" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" && "$status" != "206" ]]; then
        echo "FAIL round=$round op=file status=$status" >> "$result_file"
        ((errors++))
      fi

      # Toggle favorite on then off
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        -X PUT "$API/photos/$first_id/favorite" \
        -H "$auth" -H 'Content-Type: application/json' \
        -d '{"is_favorite":true}')
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=fav_on status=$status" >> "$result_file"
        ((errors++))
      fi

      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        -X PUT "$API/photos/$first_id/favorite" \
        -H "$auth" -H 'Content-Type: application/json' \
        -d '{"is_favorite":false}')
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=fav_off status=$status" >> "$result_file"
        ((errors++))
      fi

      # List favorites
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        "$API/photos?favorites_only=true" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=list_favorites status=$status" >> "$result_file"
        ((errors++))
      fi
    fi

    # ---- 4. Tags ----
    TAGS_RESP=$(curl -s --max-time "$MAX_TIME" "$API/tags" -H "$auth" 2>/dev/null)
    ((requests++))

    if [[ -n "$first_id" ]]; then
      # Add tag
      tag_name="tag_${username}_r${round}"
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        -X POST "$API/photos/$first_id/tags" \
        -H "$auth" -H 'Content-Type: application/json' \
        -d "{\"tag\":\"$tag_name\"}")
      ((requests++))
      if [[ "$status" != "201" && "$status" != "200" && "$status" != "409" ]]; then
        echo "FAIL round=$round op=add_tag status=$status" >> "$result_file"
        ((errors++))
      fi

      # Get photo tags
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        "$API/photos/$first_id/tags" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=get_tags status=$status" >> "$result_file"
        ((errors++))
      fi

      # Search by tag
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        "$API/search?q=$tag_name" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=search status=$status" >> "$result_file"
        ((errors++))
      fi

      # Remove tag
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        -X DELETE "$API/photos/$first_id/tags" -H "$auth" \
        -H 'Content-Type: application/json' \
        -d "{\"tag\":\"$tag_name\"}")
      ((requests++))
      if [[ "$status" != "204" && "$status" != "200" && "$status" != "404" ]]; then
        echo "FAIL round=$round op=remove_tag status=$status" >> "$result_file"
        ((errors++))
      fi
    fi

    # ---- 5. Blob upload + list + download ----
    blob_file="${TMPDIR:-/tmp}/concurrent_blob_${user_idx}_${round}.bin"
    dd if=/dev/urandom of="$blob_file" bs=1024 count=2 status=none 2>/dev/null
    blob_hash=$(sha256sum "$blob_file" | cut -d' ' -f1)

    upload_resp=$(curl -s --max-time "$MAX_TIME" -X POST "$API/blobs" \
      -H "$auth" \
      -H "x-blob-type: photo" \
      -H "x-client-hash: $blob_hash" \
      -H "Content-Type: application/octet-stream" \
      --data-binary "@$blob_file")
    ((requests++))
    blob_id=$(echo "$upload_resp" | python3 -c "
import sys, json
try: print(json.load(sys.stdin).get('blob_id',''))
except: print('')
" 2>/dev/null)
    if [[ -z "$blob_id" ]]; then
      echo "FAIL round=$round op=blob_upload resp=${upload_resp:0:100}" >> "$result_file"
      ((errors++))
    fi
    rm -f "$blob_file"

    # List blobs
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
      "$API/blobs" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=list_blobs status=$status" >> "$result_file"
      ((errors++))
    fi

    # Download blob
    if [[ -n "$blob_id" ]]; then
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
        "$API/blobs/$blob_id" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=blob_download status=$status" >> "$result_file"
        ((errors++))
      fi
    fi

    # ---- 6. Conversion status (read-heavy) ----
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
      "$API/photos/conversion-status" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=conversion_status status=$status" >> "$result_file"
      ((errors++))
    fi

    # ---- 7. Health check ----
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$MAX_TIME" \
      "$BASE/health")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=health status=$status" >> "$result_file"
      ((errors++))
    fi

  done  # end rounds

  local end_time=$(date +%s%N)
  local elapsed_ms=$(( (end_time - start_time) / 1000000 ))

  echo "REQUESTS=$requests" >> "$result_file"
  echo "ERRORS=$errors" >> "$result_file"
  echo "ELAPSED_MS=$elapsed_ms" >> "$result_file"
}

# Launch all 5 users concurrently
log "Launching 5 concurrent users, each doing $ROUNDS rounds..."
START_ALL=$(date +%s%N)

declare -a PIDS
for i in 0 1 2 3 4; do
  user_workload "$i" &
  PIDS[$i]=$!
  log "  Started ${USERS[$i]} (PID ${PIDS[$i]})"
done

# Wait for all with a global timeout (5 minutes)
TIMEOUT_SEC=300
GLOBAL_DEADLINE=$(($(date +%s) + TIMEOUT_SEC))
ALL_OK=true

for i in 0 1 2 3 4; do
  remaining=$((GLOBAL_DEADLINE - $(date +%s)))
  if [[ $remaining -le 0 ]]; then
    fail "Global timeout ($TIMEOUT_SEC s) exceeded — killing remaining"
    for j in 0 1 2 3 4; do
      kill "${PIDS[$j]}" 2>/dev/null || true
    done
    ALL_OK=false
    break
  fi

  # Wait for this specific PID with timeout
  if wait "${PIDS[$i]}"; then
    : # succeeded
  else
    warn "User ${USERS[$i]} (PID ${PIDS[$i]}) exited with non-zero"
    ALL_OK=false
  fi
done

END_ALL=$(date +%s%N)
TOTAL_MS=$(( (END_ALL - START_ALL) / 1000000 ))

# ════════════════════════════════════════════════════════════════════════════════
# PHASE 4: Collect and summarize results
# ════════════════════════════════════════════════════════════════════════════════
hdr "Phase 4: Results Summary"

TOTAL_REQUESTS=0
TOTAL_ERRORS=0

for i in 0 1 2 3 4; do
  result_file="$RESULTS_DIR/user_${i}.log"
  if [[ -f "$result_file" ]]; then
    user=$(grep "^USER=" "$result_file" | cut -d= -f2)
    reqs=$(grep "^REQUESTS=" "$result_file" | cut -d= -f2)
    errs=$(grep "^ERRORS=" "$result_file" | cut -d= -f2)
    elapsed=$(grep "^ELAPSED_MS=" "$result_file" | cut -d= -f2)

    reqs=${reqs:-0}
    errs=${errs:-0}
    elapsed=${elapsed:-0}

    TOTAL_REQUESTS=$((TOTAL_REQUESTS + reqs))
    TOTAL_ERRORS=$((TOTAL_ERRORS + errs))

    elapsed_sec=$(python3 -c "print(f'{$elapsed/1000:.1f}')")
    rps=$(python3 -c "print(f'{$reqs/($elapsed/1000):.1f}' if $elapsed > 0 else 'N/A')")

    if [[ "$errs" == "0" ]]; then
      pass "User '$user': $reqs requests, 0 errors, ${elapsed_sec}s (${rps} req/s)"
    else
      fail "User '$user': $reqs requests, $errs ERRORS, ${elapsed_sec}s (${rps} req/s)"
      # Show the failure details
      grep "^FAIL" "$result_file" | head -10 | while read -r line; do
        echo -e "         ${RED}$line${NC}"
      done
    fi
  else
    fail "No results for user index $i"
    TOTAL_ERRORS=$((TOTAL_ERRORS + 1))
  fi
done

total_sec=$(python3 -c "print(f'{$TOTAL_MS/1000:.1f}')")
total_rps=$(python3 -c "print(f'{$TOTAL_REQUESTS/($TOTAL_MS/1000):.1f}' if $TOTAL_MS > 0 else 'N/A')")

echo ""
echo -e "${BOLD}─── Aggregate ───${NC}"
log "Total wall-clock time: ${total_sec}s"
log "Total requests across all users: $TOTAL_REQUESTS"
log "Total errors: $TOTAL_ERRORS"
log "Aggregate throughput: ${total_rps} req/s"

# ════════════════════════════════════════════════════════════════════════════════
# PHASE 5: Post-stress server health check
# ════════════════════════════════════════════════════════════════════════════════
hdr "Phase 5: Post-Stress Health Verification"

# Give the server a moment to settle
sleep 2

HEALTH2=$(curl -s --max-time 10 "$BASE/health" 2>/dev/null || echo '{"error":"unreachable"}')
if echo "$HEALTH2" | grep -q '"ok"'; then
  pass "Server still healthy after stress test"
else
  fail "Server is NOT responding after stress test!"
fi

# Verify each user can still authenticate
for i in 0 1 2 3 4; do
  L=$(curl -s --max-time "$MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[$i]}\",\"password\":\"${PASSWORDS[$i]}\"}")
  T=$(echo "$L" | python3 -c "import sys,json; print(json.load(sys.stdin).get('access_token',''))" 2>/dev/null)
  if [[ -n "$T" ]]; then
    pass "User '${USERS[$i]}' can still authenticate"
  else
    fail "User '${USERS[$i]}' cannot authenticate post-stress"
  fi
done

# Verify photos endpoint still works
PHOTOS_CHECK=$(curl -s --max-time 10 "$API/photos" \
  -H "Authorization: Bearer ${TOKENS[0]}")
if echo "$PHOTOS_CHECK" | grep -q "photos"; then
  pass "Photos endpoint still functional"
else
  fail "Photos endpoint broken after stress"
fi

# ════════════════════════════════════════════════════════════════════════════════
# FINAL VERDICT
# ════════════════════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}╔════════════════════════════════════════════════════════╗${NC}"
if [[ $TOTAL_ERRORS -eq 0 ]]; then
  echo -e "${BOLD}║  ${GREEN}✓ ALL CONCURRENT TESTS PASSED${NC}${BOLD}                          ║${NC}"
  echo -e "${BOLD}║  ${NC}$TOTAL_REQUESTS requests, 0 errors, ${total_sec}s${BOLD}              ║${NC}"
else
  echo -e "${BOLD}║  ${RED}✗ CONCURRENT TEST FAILURES: $TOTAL_ERRORS errors${NC}${BOLD}              ║${NC}"
  echo -e "${BOLD}║  ${NC}$TOTAL_REQUESTS requests, $TOTAL_ERRORS errors, ${total_sec}s${BOLD}           ║${NC}"
fi
echo -e "${BOLD}╚════════════════════════════════════════════════════════╝${NC}"

# Setup/teardown totals
echo -e "\nSetup/teardown assertions: $TOTAL total, $PASSED passed, $FAILED failed, $WARNINGS warnings"

exit $TOTAL_ERRORS
