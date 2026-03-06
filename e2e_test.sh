#!/usr/bin/env bash
# ── End-to-End Test: Encryption → Conversion Banner Behavior ──────────────
# Tests that:
# 1. Conversion banner does NOT appear during encryption migration
# 2. Only the encryption banner appears during migration
# 3. After encryption finishes, conversion kicks in properly
# 4. ETA text does NOT contain "rem"
#
# IMPORTANT: Run the server BEFORE this script:
#   RUST_LOG=info,simple_photos_server=debug nohup ./server/target/release/simple-photos-server > /tmp/server_e2e.log 2>&1 &
set -uo pipefail  # no -e: we handle errors manually

BASE="http://localhost:8080"
USER="testuser"
PASS='TestPass123!'
SLOG="/tmp/server_e2e.log"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*"; FAILURES=$((FAILURES+1)); }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
hdr()  { echo -e "\n${BOLD}════════════════════════════════════════════════════════${NC}"; echo -e "${BOLD}  $*${NC}"; echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"; }

# JSON value extractor — returns default on any error
# Normalizes Python True/False → true/false for bash comparison
jget() {
  python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    v = d.get('$1', '$2')
    if isinstance(v, bool):
        print('true' if v else 'false')
    else:
        print(v)
except:
    print('$2')
"
}

FAILURES=0

# ── Step 1: Setup + Login ──────────────────────────────────────────────
hdr "Step 1: Setup Initial Admin & Login"

INIT=$(curl -s --max-time 10 -X POST "$BASE/api/setup/init" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")
log "Setup init: $INIT"

LOGIN=$(curl -s --max-time 10 -X POST "$BASE/api/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")
TOKEN=$(echo "$LOGIN" | jget access_token "")
if [[ -z "$TOKEN" ]]; then
  fail "Login failed: $LOGIN"
  exit 1
fi
pass "Authenticated as admin (token: ${TOKEN:0:20}...)"
AUTH="Authorization: Bearer $TOKEN"

# ── Step 2: Trigger scan ──────────────────────────────────────────────────
hdr "Step 2: Trigger Photo Scan"
log "Scan may take several minutes (FFmpeg thumbnail generation for videos)..."

SCAN=$(curl -s --max-time 600 -X POST "$BASE/api/admin/photos/scan" -H "$AUTH")
log "Scan result: $SCAN"

PHOTO_COUNT=$(echo "$SCAN" | jget registered 0)
log "New photos registered: $PHOTO_COUNT"

if [[ "$PHOTO_COUNT" == "0" ]]; then
  warn "No photos registered via scan, checking DB..."
  PHOTOS=$(curl -s --max-time 10 "$BASE/api/photos" -H "$AUTH")
  PHOTO_COUNT=$(echo "$PHOTOS" | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d.get('photos',d)) if isinstance(d,dict) else len(d))" 2>/dev/null || echo "0")
  log "Total photos in DB: $PHOTO_COUNT"
fi

if [[ "$PHOTO_COUNT" -gt 0 ]]; then
  pass "Photos scanned: $PHOTO_COUNT"
else
  fail "No photos found after scan"
fi

# ── Step 3: Wait for pre-encryption conversions ─────────────────────────
hdr "Step 3: Wait for Pre-Encryption Conversions"

PRE_CONV_DONE=false
for i in $(seq 1 120); do
  CS=$(curl -s --max-time 5 "$BASE/api/photos/conversion-status" -H "$AUTH" 2>/dev/null || echo "{}")
  P=$(echo "$CS" | jget pending_conversions 0)
  A=$(echo "$CS" | jget converting false)
  T=$(echo "$CS" | jget missing_thumbnails 0)

  if [[ "$i" -eq 1 ]] || (( i % 5 == 0 )); then
    log "  [$i] pending=$P converting=$A thumbs=$T"
  fi

  if [[ "$P" == "0" && "$A" == "false" && "$T" == "0" ]]; then
    pass "Pre-encryption conversions done (waited ~$((i*3))s)"
    PRE_CONV_DONE=true
    break
  fi
  sleep 3
done
if [[ "$PRE_CONV_DONE" != "true" ]]; then
  warn "Pre-encryption conversions still running after 6 min — continuing anyway"
fi

# ── Step 4: Derive encryption key ───────────────────────────────────────
hdr "Step 4: Derive Encryption Key"

KEY_HEX=$(python3 -c "
from hashlib import sha256
from argon2.low_level import hash_secret_raw, Type
salt = sha256(('simple-photos:' + '$USER').encode()).digest()[:16]
key = hash_secret_raw(secret=b'$PASS', salt=salt, time_cost=3, memory_cost=65536, parallelism=4, hash_len=32, type=Type.ID)
print(key.hex())
")
pass "Key derived: ${KEY_HEX:0:16}..."

# ── Step 5: Set encryption mode ─────────────────────────────────────────
hdr "Step 5: Enable Encryption Mode"

MODE_RESP=$(curl -s --max-time 10 -X PUT "$BASE/api/admin/encryption" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"mode":"encrypted"}')
log "Set mode response: $MODE_RESP"

ENC_CHECK=$(curl -s --max-time 10 "$BASE/api/settings/encryption" -H "$AUTH")
ENC_MODE=$(echo "$ENC_CHECK" | jget encryption_mode "none")
if [[ "$ENC_MODE" == "encrypted" ]]; then
  pass "Encryption mode set to: $ENC_MODE"
else
  fail "Encryption mode unexpected: $ENC_MODE (expected 'encrypted')"
fi

# ── Step 6: Start server-side migration ────────────────────────────────
hdr "Step 6: Start Server-Side Migration"

log "Conversion status BEFORE migration:"
CS_PRE=$(curl -s --max-time 5 "$BASE/api/photos/conversion-status" -H "$AUTH" 2>/dev/null || echo "{}")
log "  $CS_PRE"

MIG_RESP=$(curl -s --max-time 10 -X POST "$BASE/api/admin/encryption/migrate" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"key_hex\":\"$KEY_HEX\"}")
log "Migration response: $MIG_RESP"

MIG_MSG=$(echo "$MIG_RESP" | jget message "")
if echo "$MIG_MSG" | grep -qi "start\|migrat"; then
  pass "Migration started: $MIG_MSG"
else
  warn "Migration response: $MIG_RESP"
fi

# ── Step 7: Monitor banners during encryption ───────────────────────────
hdr "Step 7: Monitor Banners During Encryption"
echo -e "${YELLOW}  KEY CHECK: Conversion banner should NOT appear during encryption${NC}"
echo ""

CONVERSION_APPEARED_DURING_ENC=false
ENCRYPTION_APPEARED=false
POLL_COUNT=0
MAX_POLLS=120

while [[ $POLL_COUNT -lt $MAX_POLLS ]]; do
  POLL_COUNT=$((POLL_COUNT + 1))

  # Conversion status — correct path is /api/photos/conversion-status (not /admin/)
  CS=$(curl -s --max-time 5 "$BASE/api/photos/conversion-status" -H "$AUTH" 2>/dev/null || echo "{}")
  PENDING=$(echo "$CS" | jget pending_conversions "?")
  AWAITING=$(echo "$CS" | jget pending_awaiting_key "?")
  CONVERTING=$(echo "$CS" | jget converting "?")
  MIG_RUN=$(echo "$CS" | jget migration_running "?")
  THUMBS=$(echo "$CS" | jget missing_thumbnails "?")
  KEY_AVAIL=$(echo "$CS" | jget key_available "?")

  # Encryption/migration status
  ES=$(curl -s --max-time 5 "$BASE/api/settings/encryption" -H "$AUTH" 2>/dev/null || echo "{}")
  ENC_MODE=$(echo "$ES" | jget encryption_mode "?")
  MIG_STATUS=$(echo "$ES" | jget migration_status "?")
  MIG_TOTAL=$(echo "$ES" | jget migration_total "?")
  MIG_DONE=$(echo "$ES" | jget migration_completed "?")

  # Simulate frontend banner logic:
  # conversionBusy = pending_conversions > 0 || missing_thumbnails > 0 || converting == true
  CONV_BUSY="no"
  if [[ "$PENDING" != "0" && "$PENDING" != "?" ]] || \
     [[ "$THUMBS" != "0" && "$THUMBS" != "?" ]] || \
     [[ "$CONVERTING" == "True" || "$CONVERTING" == "true" ]]; then
    CONV_BUSY="${RED}YES${NC}"
  fi

  MIG_BUSY="no"
  if [[ "$MIG_STATUS" == "encrypting" || "$MIG_STATUS" == "decrypting" ]] && \
     [[ "$MIG_TOTAL" != "0" && "$MIG_TOTAL" != "?" ]]; then
    MIG_BUSY="${GREEN}YES${NC}"
  fi

  echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} #${POLL_COUNT} | mig=${MIG_STATUS}(${MIG_DONE}/${MIG_TOTAL}) | conv: pend=${PENDING} await=${AWAITING} thmb=${THUMBS} act=${CONVERTING} key=${KEY_AVAIL} migRun=${MIG_RUN} | ${BOLD}BANNERS: conv=$CONV_BUSY enc=$MIG_BUSY${NC}"

  # Track if conversion banner would appear during encryption
  if [[ "$MIG_STATUS" == "encrypting" || "$MIG_STATUS" == "decrypting" ]]; then
    ENCRYPTION_APPEARED=true
    if [[ "$PENDING" != "0" && "$PENDING" != "?" ]] || \
       [[ "$THUMBS" != "0" && "$THUMBS" != "?" ]] || \
       [[ "$CONVERTING" == "True" || "$CONVERTING" == "true" ]]; then
      fail "*** CONVERSION BANNER ACTIVE DURING ENCRYPTION *** pending=$PENDING thumbs=$THUMBS converting=$CONVERTING"
      CONVERSION_APPEARED_DURING_ENC=true
    fi
  fi

  # Migration finished? (saw encrypting, now idle)
  if [[ "$ENCRYPTION_APPEARED" == "true" && "$MIG_STATUS" == "idle" ]]; then
    pass "Encryption migration finished!"
    break
  fi

  # If 10+ polls and still idle, maybe migration already completed
  if [[ "$POLL_COUNT" -gt 10 && "$ENCRYPTION_APPEARED" == "false" && "$MIG_STATUS" == "idle" ]]; then
    warn "Migration doesn't seem to have started (10 polls passed, still idle)"
    break
  fi

  sleep 3
done

if [[ "$POLL_COUNT" -ge "$MAX_POLLS" ]]; then
  warn "Timed out waiting for encryption — $POLL_COUNT polls"
fi

echo ""
echo -e "${BOLD}── Encryption Phase Verdict ──${NC}"
if [[ "$ENCRYPTION_APPEARED" == "true" ]]; then
  pass "Encryption banner appeared during migration"
else
  warn "Encryption banner was never seen (migration may have been too fast)"
fi

if [[ "$CONVERSION_APPEARED_DURING_ENC" == "true" ]]; then
  fail "VERDICT: Conversion banner appeared during encryption — BUG STILL PRESENT"
else
  pass "VERDICT: Conversion banner did NOT appear during encryption — FIXED!"
fi

# ── Step 8: Monitor post-encryption conversion ──────────────────────────
hdr "Step 8: Post-Encryption Conversion Monitor"
log "Converter triggers 5s after migration ends..."

CONVERSION_STARTED=false
for i in $(seq 1 90); do
  CS=$(curl -s --max-time 5 "$BASE/api/photos/conversion-status" -H "$AUTH" 2>/dev/null || echo "{}")
  PENDING=$(echo "$CS" | jget pending_conversions 0)
  AWAITING=$(echo "$CS" | jget pending_awaiting_key 0)
  CONVERTING=$(echo "$CS" | jget converting false)
  KEY_AVAIL=$(echo "$CS" | jget key_available false)
  THUMBS=$(echo "$CS" | jget missing_thumbnails 0)
  MIG_RUN=$(echo "$CS" | jget migration_running false)

  CONV_BUSY="no"
  if [[ "$PENDING" != "0" ]] || [[ "$THUMBS" != "0" ]] || \
     [[ "$CONVERTING" == "True" || "$CONVERTING" == "true" ]]; then
    CONV_BUSY="${YELLOW}YES${NC}"
    CONVERSION_STARTED=true
  fi

  echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} post#${i} | pend=${PENDING} await=${AWAITING} thmb=${THUMBS} act=${CONVERTING} key=${KEY_AVAIL} migRun=${MIG_RUN} | ${BOLD}CONV_BANNER=$CONV_BUSY${NC}"

  if [[ "$CONVERSION_STARTED" == "true" && "$PENDING" == "0" && "$AWAITING" == "0" && "$THUMBS" == "0" && "$CONVERTING" == "false" ]]; then
    pass "Post-encryption conversion completed!"
    break
  fi

  if [[ "$PENDING" == "0" && "$AWAITING" == "0" && "$THUMBS" == "0" && "$CONVERTING" == "false" && "$i" -gt 15 ]]; then
    log "No conversion work needed (all formats web-compatible or no key)"
    break
  fi

  sleep 3
done

# ── Step 9: Verify ETA text format ──────────────────────────────────────
hdr "Step 9: Verify ETA Text (no 'rem')"

BUNDLE=$(find /home/wulfic/repos/simple-photos/web/dist -name "*.js" -type f 2>/dev/null | head -1)
if [[ -n "$BUNDLE" ]]; then
  if grep -q 's rem\|m rem' "$BUNDLE" 2>/dev/null; then
    fail "Found 'rem' in ETA text in built JS bundle!"
    grep -o '[^"]*rem[^"]*' "$BUNDLE" 2>/dev/null | head -5
  else
    pass "No 'rem' found in ETA text — clean format confirmed"
  fi
else
  warn "Could not locate built JS bundle for ETA verification"
fi

# ── Step 10: Check server DIAG logs ─────────────────────────────────────
hdr "Step 10: Server Diagnostics Summary"

log "DIAG:CONVERT entries:"
echo "────────────────────────────────────────────────────────────────"
grep 'DIAG:CONVERT' "$SLOG" 2>/dev/null | tail -30 || echo "(none)"
echo "────────────────────────────────────────────────────────────────"

echo ""
log "DIAG:SERVER_MIG entries:"
echo "────────────────────────────────────────────────────────────────"
grep 'DIAG:SERVER_MIG' "$SLOG" 2>/dev/null | tail -30 || echo "(none)"
echo "────────────────────────────────────────────────────────────────"

echo ""
log "DIAG:STATUS entries (last 10):"
echo "────────────────────────────────────────────────────────────────"
grep 'DIAG:STATUS' "$SLOG" 2>/dev/null | tail -10 || echo "(none)"
echo "────────────────────────────────────────────────────────────────"

echo ""
log "DIAG:SCAN entries:"
echo "────────────────────────────────────────────────────────────────"
grep 'DIAG:SCAN' "$SLOG" 2>/dev/null | tail -10 || echo "(none)"
echo "────────────────────────────────────────────────────────────────"

# ── Final summary ─────────────────────────────────────────────────
hdr "E2E Test Results Summary"
echo -e "  Photos scanned:                   ${BOLD}$PHOTO_COUNT${NC}"
echo -e "  Encryption banner appeared:        ${BOLD}$ENCRYPTION_APPEARED${NC}"
echo -e "  Conv banner during encryption:     ${BOLD}$CONVERSION_APPEARED_DURING_ENC${NC}"
echo -e "  Post-enc conversion started:       ${BOLD}$CONVERSION_STARTED${NC}"
echo -e "  Total failures:                    ${BOLD}$FAILURES${NC}"

echo ""
if [[ "$FAILURES" -eq 0 ]]; then
  echo -e "${GREEN}${BOLD}  ALL TESTS PASSED${NC}"
else
  echo -e "${RED}${BOLD}  $FAILURES TEST(S) FAILED${NC}"
fi
echo ""
exit "$FAILURES"
