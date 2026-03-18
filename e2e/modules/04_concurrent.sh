#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Module 04: Concurrent Multi-User Stress Tests for Simple Photos
# ══════════════════════════════════════════════════════════════════════════════
# Validates dual-pool SQLite architecture and spawn_blocking under concurrent
# multi-user load. Creates 5 users, runs simultaneous operations, checks for
# errors, hangs, and response time regressions.
#
#   Phase 1 — Server health check
#   Phase 2 — Create 5 users + trigger photo scan
#   Phase 3 — Concurrent workloads (background processes)
#   Phase 4 — Collect and summarize per-user results
#   Phase 5 — Post-stress verification
#
# Prerequisites:
#   - Server running: sudo bash reset-server.sh
#   - Photos in storage root for meaningful workloads
#
# Usage:
#   bash e2e/modules/04_concurrent.sh [--verbose] [--rounds=N]
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/helpers.sh"
parse_common_args "$@"
setup_module_log "concurrent"

module_timer_start "Concurrent Stress Tests"

# ── Concurrent-specific config ───────────────────────────────────────────────
ROUNDS="${ROUNDS:-5}"
CONCURRENT_TIMEOUT="${CONCURRENT_TIMEOUT:-300}"
CONCURRENT_MAX_TIME="${CONCURRENT_MAX_TIME:-30}"

# Parse module-specific args
for arg in "$@"; do
  case $arg in
    --rounds=*) ROUNDS="${arg#--rounds=}" ;;
  esac
done

# Results directory for per-user logs
CONCURRENT_RESULTS="$E2E_TMP_DIR/concurrent_results"
rm -rf "$CONCURRENT_RESULTS"
mkdir -p "$CONCURRENT_RESULTS"

echo -e "${BOLD}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  Concurrent 5-User Stress Test — Simple Photos                 ║${NC}"
echo -e "${BOLD}╚════════════════════════════════════════════════════════════════╝${NC}"
log "Rounds per user: $ROUNDS"

# ══════════════════════════════════════════════════════════════════════════════
# PHASE 1: SERVER HEALTH CHECK
# ══════════════════════════════════════════════════════════════════════════════
hdr "Phase 1: Server Health Check"

HEALTH=$(curl -s --max-time 10 "$MAIN_BASE/health" 2>/dev/null || echo '{"error":"unreachable"}')
if echo "$HEALTH" | grep -q '"ok"'; then
  pass "Server is healthy"
else
  fail "Server is not responding — aborting"
  log "Response: $HEALTH"
  module_timer_stop > /dev/null
  print_summary "Concurrent E2E"
  exit 1
fi

STATUS=$(curl -s --max-time 10 "$MAIN_API/setup/status")
SETUP_DONE=$(echo "$STATUS" | jget setup_complete "false")

# ══════════════════════════════════════════════════════════════════════════════
# PHASE 2: CREATE 5 USERS + PHOTO SCAN
# ══════════════════════════════════════════════════════════════════════════════
hdr "Phase 2: Create 5 Users"

USERS=("admin1" "alice" "bob" "carol" "dave")
PASSWORDS=("AdminPass1!" "AlicePass1!" "BobbyPass1!" "CarolPass1!" "DaveyPass1!")
declare -a TOKENS
declare -a USER_IDS

if [[ "$SETUP_DONE" == "false" || "$SETUP_DONE" == "False" ]]; then
  INIT=$(curl -s --max-time "$CONCURRENT_MAX_TIME" -X POST "$MAIN_API/setup/init" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[0]}\",\"password\":\"${PASSWORDS[0]}\"}")
  ADMIN_ID=$(echo "$INIT" | jget user_id "")
  if [[ -n "$ADMIN_ID" && "$ADMIN_ID" != "__MISSING__" ]]; then
    pass "Admin '${USERS[0]}' created (ID: $ADMIN_ID)"
    USER_IDS[0]="$ADMIN_ID"
  else
    fail "Admin creation failed: $INIT"
    module_timer_stop > /dev/null
    print_summary "Concurrent E2E"
    exit 1
  fi
else
  log "Server already initialized — using existing admin"
  USERS[0]="$ADMIN_USER"
  PASSWORDS[0]="$ADMIN_PASS"
fi

# Login admin
LOGIN=$(curl -s --max-time "$CONCURRENT_MAX_TIME" -X POST "$MAIN_API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"${USERS[0]}\",\"password\":\"${PASSWORDS[0]}\"}")
TOKENS[0]=$(echo "$LOGIN" | jget access_token "")
if [[ -z "${TOKENS[0]}" || "${TOKENS[0]}" == "__MISSING__" ]]; then
  fail "Admin login failed: $LOGIN"
  module_timer_stop > /dev/null
  print_summary "Concurrent E2E"
  exit 1
fi
pass "Admin '${USERS[0]}' logged in"

# Create remaining 4 users
for i in 1 2 3 4; do
  CREATE=$(curl -s --max-time "$CONCURRENT_MAX_TIME" -X POST "$MAIN_API/admin/users" \
    -H "Authorization: Bearer ${TOKENS[0]}" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[$i]}\",\"password\":\"${PASSWORDS[$i]}\",\"role\":\"user\"}")
  URID=$(echo "$CREATE" | jget user_id "")
  if [[ -n "$URID" && "$URID" != "__MISSING__" ]]; then
    pass "User '${USERS[$i]}' created (ID: $URID)"
    USER_IDS[$i]="$URID"
  else
    warn "User '${USERS[$i]}' creation response: ${CREATE:0:120} (may already exist)"
  fi

  L=$(curl -s --max-time "$CONCURRENT_MAX_TIME" -X POST "$MAIN_API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"${USERS[$i]}\",\"password\":\"${PASSWORDS[$i]}\"}")
  TOKENS[$i]=$(echo "$L" | jget access_token "")
  if [[ -n "${TOKENS[$i]}" && "${TOKENS[$i]}" != "__MISSING__" ]]; then
    pass "User '${USERS[$i]}' logged in"
  else
    fail "User '${USERS[$i]}' login failed"
  fi
done

subhdr "Trigger Photo Scan"
# Check if photos already exist from a prior module — skip the full scan wait
EXISTING_PHOTOS=$(curl -s --max-time 10 "$MAIN_API/photos" \
  -H "Authorization: Bearer ${TOKENS[0]}" | jget photos "[]" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "0")

if [[ "$EXISTING_PHOTOS" -gt 0 ]]; then
  log "Server already has $EXISTING_PHOTOS photos — triggering quick scan"
  SCAN=$(curl -s --max-time 60 -X POST "$MAIN_API/admin/photos/scan" \
    -H "Authorization: Bearer ${TOKENS[0]}")
  REGISTERED=$(echo "$SCAN" | jget registered 0)
  log "Scan result: registered=$REGISTERED"
  # Only wait briefly since photos already exist and conversions likely done
  wait_for_conversions "$MAIN_API" "Authorization: Bearer ${TOKENS[0]}" 15
else
  log "Scanning photos..."
  SCAN=$(curl -s --max-time 600 -X POST "$MAIN_API/admin/photos/scan" \
    -H "Authorization: Bearer ${TOKENS[0]}")
  REGISTERED=$(echo "$SCAN" | jget registered 0)
  log "Scan result: registered=$REGISTERED"
  # Full wait for conversions (up to 60s)
  wait_for_conversions "$MAIN_API" "Authorization: Bearer ${TOKENS[0]}" 60
fi

# ══════════════════════════════════════════════════════════════════════════════
# PHASE 3: CONCURRENT OPERATIONS — ALL 5 USERS AT ONCE
# ══════════════════════════════════════════════════════════════════════════════
hdr "Phase 3: Concurrent Multi-User Operations ($ROUNDS rounds)"

# ── Workload function (runs in background per user) ──────────────────────────
# Each user performs a realistic mixed read/write workload and records results.
user_workload() {
  set +e  # Disable errexit — we track errors manually
  local user_idx=$1
  local username="${USERS[$user_idx]}"
  local token="${TOKENS[$user_idx]}"
  local auth="Authorization: Bearer $token"
  local result_file="$CONCURRENT_RESULTS/user_${user_idx}.log"
  local errors=0
  local requests=0
  local start_time
  start_time=$(date +%s%N)

  echo "USER=$username" > "$result_file"

  for round in $(seq 1 "$ROUNDS"); do
    # ── 1. List photos ──
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
      "$MAIN_API/photos" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=list_photos status=$status" >> "$result_file"
      ((errors++))
    fi

    # ── 2. Paginated photos ──
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
      "$MAIN_API/photos?limit=5&offset=0" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=list_photos_paginated status=$status" >> "$result_file"
      ((errors++))
    fi

    # ── 3. Photo details (thumb, file, favorite toggle) ──
    photos_json=$(curl -s --max-time "$CONCURRENT_MAX_TIME" "$MAIN_API/photos" -H "$auth" 2>/dev/null)
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
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        "$MAIN_API/photos/$first_id/thumb" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" && "$status" != "404" && "$status" != "202" ]]; then
        echo "FAIL round=$round op=thumb status=$status" >> "$result_file"
        ((errors++))
      fi

      # Serve file
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        "$MAIN_API/photos/$first_id/file" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" && "$status" != "206" ]]; then
        echo "FAIL round=$round op=file status=$status" >> "$result_file"
        ((errors++))
      fi

      # Toggle favorite on + off
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        -X PUT "$MAIN_API/photos/$first_id/favorite" \
        -H "$auth" -H 'Content-Type: application/json' \
        -d '{"is_favorite":true}')
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=fav_on status=$status" >> "$result_file"
        ((errors++))
      fi

      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        -X PUT "$MAIN_API/photos/$first_id/favorite" \
        -H "$auth" -H 'Content-Type: application/json' \
        -d '{"is_favorite":false}')
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=fav_off status=$status" >> "$result_file"
        ((errors++))
      fi

      # List favorites
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        "$MAIN_API/photos?favorites_only=true" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=list_favorites status=$status" >> "$result_file"
        ((errors++))
      fi
    fi

    # ── 4. Tags ──
    curl -s --max-time "$CONCURRENT_MAX_TIME" "$MAIN_API/tags" -H "$auth" > /dev/null 2>&1
    ((requests++))

    if [[ -n "$first_id" ]]; then
      tag_name="tag_${username}_r${round}"
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        -X POST "$MAIN_API/photos/$first_id/tags" \
        -H "$auth" -H 'Content-Type: application/json' \
        -d "{\"tag\":\"$tag_name\"}")
      ((requests++))
      if [[ "$status" != "201" && "$status" != "200" && "$status" != "409" ]]; then
        echo "FAIL round=$round op=add_tag status=$status" >> "$result_file"
        ((errors++))
      fi

      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        "$MAIN_API/photos/$first_id/tags" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=get_tags status=$status" >> "$result_file"
        ((errors++))
      fi

      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        "$MAIN_API/search?q=$tag_name" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=search status=$status" >> "$result_file"
        ((errors++))
      fi

      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        -X DELETE "$MAIN_API/photos/$first_id/tags" -H "$auth" \
        -H 'Content-Type: application/json' \
        -d "{\"tag\":\"$tag_name\"}")
      ((requests++))
      if [[ "$status" != "204" && "$status" != "200" && "$status" != "404" ]]; then
        echo "FAIL round=$round op=remove_tag status=$status" >> "$result_file"
        ((errors++))
      fi
    fi

    # ── 5. Blob upload + list + download ──
    blob_file="$CONCURRENT_RESULTS/blob_${user_idx}_${round}.bin"
    dd if=/dev/urandom of="$blob_file" bs=1024 count=2 status=none 2>/dev/null
    blob_hash=$(sha256sum "$blob_file" | cut -d' ' -f1)

    upload_resp=$(curl -s --max-time "$CONCURRENT_MAX_TIME" -X POST "$MAIN_API/blobs" \
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

    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
      "$MAIN_API/blobs" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=list_blobs status=$status" >> "$result_file"
      ((errors++))
    fi

    if [[ -n "$blob_id" ]]; then
      status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
        "$MAIN_API/blobs/$blob_id" -H "$auth")
      ((requests++))
      if [[ "$status" != "200" ]]; then
        echo "FAIL round=$round op=blob_download status=$status" >> "$result_file"
        ((errors++))
      fi
    fi

    # ── 6. Conversion status (read-heavy) ──
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
      "$MAIN_API/photos/conversion-status" -H "$auth")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=conversion_status status=$status" >> "$result_file"
      ((errors++))
    fi

    # ── 7. Health check ──
    status=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
      "$MAIN_BASE/health")
    ((requests++))
    if [[ "$status" != "200" ]]; then
      echo "FAIL round=$round op=health status=$status" >> "$result_file"
      ((errors++))
    fi
  done

  local end_time
  end_time=$(date +%s%N)
  local elapsed_ms=$(( (end_time - start_time) / 1000000 ))

  echo "REQUESTS=$requests" >> "$result_file"
  echo "ERRORS=$errors" >> "$result_file"
  echo "ELAPSED_MS=$elapsed_ms" >> "$result_file"
}

# ── Launch all 5 users concurrently ──────────────────────────────────────────
log "Launching 5 concurrent users, each doing $ROUNDS rounds..."
START_ALL=$(date +%s%N)

declare -a PIDS
for i in 0 1 2 3 4; do
  user_workload "$i" &
  PIDS[$i]=$!
  log "  Started ${USERS[$i]} (PID ${PIDS[$i]})"
done

# Wait for all with global timeout
GLOBAL_DEADLINE=$(($(date +%s) + CONCURRENT_TIMEOUT))
ALL_OK=true

for i in 0 1 2 3 4; do
  remaining=$((GLOBAL_DEADLINE - $(date +%s)))
  if [[ $remaining -le 0 ]]; then
    fail "Global timeout (${CONCURRENT_TIMEOUT}s) exceeded — killing remaining"
    for j in 0 1 2 3 4; do
      kill "${PIDS[$j]}" 2>/dev/null || true
    done
    ALL_OK=false
    break
  fi

  if wait "${PIDS[$i]}"; then
    : # succeeded
  else
    warn "User ${USERS[$i]} (PID ${PIDS[$i]}) exited with non-zero"
    ALL_OK=false
  fi
done

END_ALL=$(date +%s%N)
TOTAL_MS=$(( (END_ALL - START_ALL) / 1000000 ))

# ══════════════════════════════════════════════════════════════════════════════
# PHASE 4: COLLECT AND SUMMARIZE RESULTS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Phase 4: Results Summary"

TOTAL_REQUESTS=0
TOTAL_ERRORS=0

for i in 0 1 2 3 4; do
  result_file="$CONCURRENT_RESULTS/user_${i}.log"
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
log "  ┌─────────────────────────────────────────────┐"
log "  │  Concurrent Stress — Aggregate Results       │"
log "  ├─────────────────────────────────────────────┤"
log "  │  Wall-clock time:  ${total_sec}s"
log "  │  Total requests:   $TOTAL_REQUESTS"
log "  │  Total errors:     $TOTAL_ERRORS"
log "  │  Throughput:       ${total_rps} req/s"
log "  └─────────────────────────────────────────────┘"

# ══════════════════════════════════════════════════════════════════════════════
# PHASE 5: POST-STRESS VERIFICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Phase 5: Post-Stress Health Verification"

sleep 2  # Let the server settle

subhdr "Server Health After Stress"
HEALTH2=$(curl -s --max-time 10 "$MAIN_BASE/health" 2>/dev/null || echo '{"error":"unreachable"}')
if echo "$HEALTH2" | grep -q '"ok"'; then
  pass "Server still healthy after stress test"
else
  fail "Server is NOT responding after stress test!"
fi

subhdr "All Users Can Still Operate"
# Verify using existing tokens (avoids login rate-limiter: 10/60s per IP).
# The real concern is "can users still use the API post-stress", not
# "can the login endpoint handle more load".
for i in 0 1 2 3 4; do
  CHECK=$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CONCURRENT_MAX_TIME" \
    "$MAIN_API/photos" -H "Authorization: Bearer ${TOKENS[$i]}")
  if [[ "$CHECK" == "200" ]]; then
    pass "User '${USERS[$i]}' token still valid (photos endpoint: HTTP 200)"
  else
    # Token might have expired — try fresh login as fallback
    L=$(curl -s --max-time "$CONCURRENT_MAX_TIME" -X POST "$MAIN_API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"${USERS[$i]}\",\"password\":\"${PASSWORDS[$i]}\"}")
    T=$(echo "$L" | jget access_token "")
    if [[ -n "$T" && "$T" != "__MISSING__" ]]; then
      pass "User '${USERS[$i]}' re-authenticated (original token expired)"
    else
      fail "User '${USERS[$i]}' cannot operate post-stress (photos=$CHECK, login=${L:0:120})"
    fi
  fi
done

subhdr "Photos Endpoint Still Functional"
PHOTOS_CHECK=$(curl -s --max-time 10 "$MAIN_API/photos" \
  -H "Authorization: Bearer ${TOKENS[0]}")
if echo "$PHOTOS_CHECK" | grep -q "photos"; then
  pass "Photos endpoint still functional"
else
  fail "Photos endpoint broken after stress"
fi

subhdr "Response Time Sanity Check"
RESP_START=$(date +%s%N)
curl -s -o /dev/null --max-time 10 "$MAIN_API/photos" -H "Authorization: Bearer ${TOKENS[0]}"
RESP_END=$(date +%s%N)
RESP_MS=$(( (RESP_END - RESP_START) / 1000000 ))
if (( RESP_MS < 5000 )); then
  pass "Post-stress response time: ${RESP_MS}ms (< 5s threshold)"
else
  fail "Post-stress response time: ${RESP_MS}ms (>= 5s — regression?)"
fi

# ══════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ══════════════════════════════════════════════════════════════════════════════
module_timer_stop > /dev/null
print_summary "Concurrent E2E"

# Exit with assertion failure count for consistency with other modules.
# TOTAL_ERRORS is logged above for visibility but individual HTTP errors
# are already rolled up into per-user FAILURES via fail().
exit "$FAILURES"
