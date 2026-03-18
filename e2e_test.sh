#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Comprehensive End-to-End Test Suite for Simple Photos Server
# ══════════════════════════════════════════════════════════════════════════════
# Tests EVERY API endpoint across ALL server modules:
#   1.  Health check
#   2.  Setup & initialization
#   3.  Auth (login, register, refresh, logout, password, 2FA)
#   4.  Admin user management
#   5.  Admin config (storage, port, SSL)
#   6.  Photo scan & conversion
#   7.  Photos (list, serve, favorite, crop, metadata)
#   8.  Tags & search
#   9.  Photo copies & duplicates
#  10.  Trash (soft-delete, list, restore, permanent delete, empty)
#  11.  Shared albums
#  12.  Secure galleries
#  13.  Blob upload/download/delete (encrypted mode prep)
#  14.  Encryption settings
#  15.  Storage stats & cleanup status
#  16.  Backup server management
#  17.  Client logs
#  18.  Diagnostics & audit logs
#  19.  External diagnostics (Basic Auth)
#  20.  Downloads (Android APK — 404 expected)
#  21.  Security headers verification
#  22.  Logout & cleanup
#
# Prerequisites:
#   - Server built: cd server && cargo build --release
#   - Reset & start: sudo bash reset-server.sh
#   - Photos exist at the configured storage root (for scan tests)
#
# Usage:
#   bash e2e_test.sh
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail  # no -e: we handle errors manually

BASE="http://localhost:8080"
API="$BASE/api"
USER="testadmin"
PASS='TestPass123!'
USER2="testuser2"
PASS2='SecondUser1!'
SLOG="${TMPDIR:-/tmp}/simple-photos-server.log"

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
    elif isinstance(v, list):
        print(json.dumps(v))
    else:
        print(v)
except:
    print('$2')
"
}

# HTTP status code checker
http_status() {
  curl -s -o /dev/null -w "%{http_code}" --max-time 10 "$@"
}

# Assert HTTP response contains expected value
assert_contains() {
  local desc="$1" response="$2" expected="$3"
  if echo "$response" | grep -qi "$expected"; then
    pass "$desc"
  else
    fail "$desc (expected '$expected' in response)"
    log "  Response: ${response:0:200}"
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

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 1: HEALTH CHECK
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 1: Health Check"

HEALTH=$(curl -s --max-time 10 "$BASE/health")
assert_json "Health endpoint returns status=ok" "$HEALTH" "status" "ok"
assert_contains "Health response includes version" "$HEALTH" "version"
assert_contains "Health response includes service name" "$HEALTH" "simple-photos"

# Verify compression header (Accept-Encoding: gzip)
COMPRESS_HEADERS=$(curl -sI --max-time 10 -H "Accept-Encoding: gzip" "$BASE/health")
if echo "$COMPRESS_HEADERS" | grep -qi "content-encoding"; then
  pass "Compression headers present"
else
  warn "Compression headers not detected (may be too small to compress)"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 2: SETUP & INITIALIZATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 2: Setup & Initialization"

subhdr "Setup Status (before init)"
STATUS=$(curl -s --max-time 10 "$API/setup/status")
assert_json "Setup not yet complete" "$STATUS" "setup_complete" "false"
assert_contains "Status includes version" "$STATUS" "version"

subhdr "Initialize Admin User"
INIT=$(curl -s --max-time 10 -X POST "$API/setup/init" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")
assert_contains "Init creates user_id" "$INIT" "user_id"
assert_json "Init returns correct username" "$INIT" "username" "$USER"
ADMIN_USER_ID=$(echo "$INIT" | jget user_id "")

subhdr "Setup Status (after init)"
STATUS2=$(curl -s --max-time 10 "$API/setup/status")
assert_json "Setup now complete" "$STATUS2" "setup_complete" "true"

subhdr "Double-init blocked"
INIT2_STATUS=$(http_status -X POST "$API/setup/init" \
  -H 'Content-Type: application/json' \
  -d '{"username":"hacker","password":"HackPass1!"}')
if [[ "$INIT2_STATUS" == "403" ]]; then
  pass "Double-init correctly rejected (HTTP 403)"
else
  fail "Double-init should return 403, got $INIT2_STATUS"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 3: AUTHENTICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 3: Authentication"

subhdr "Login"
LOGIN=$(curl -s --max-time 10 -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")
TOKEN=$(echo "$LOGIN" | jget access_token "")
REFRESH=$(echo "$LOGIN" | jget refresh_token "")
if [[ -n "$TOKEN" && "$TOKEN" != "__MISSING__" ]]; then
  pass "Login successful (token: ${TOKEN:0:20}...)"
else
  fail "Login failed: $LOGIN"
  echo "FATAL: Cannot continue without auth token"
  exit 1
fi
AUTH="Authorization: Bearer $TOKEN"

subhdr "Login with wrong password"
BAD_LOGIN_STATUS=$(http_status -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"WrongPass1!\"}")
if [[ "$BAD_LOGIN_STATUS" == "401" ]]; then
  pass "Wrong password correctly rejected (HTTP 401)"
else
  fail "Wrong password should return 401, got $BAD_LOGIN_STATUS"
fi

subhdr "Refresh Token"
REFRESH_RESP=$(curl -s --max-time 10 -X POST "$API/auth/refresh" \
  -H 'Content-Type: application/json' \
  -d "{\"refresh_token\":\"$REFRESH\"}")
NEW_TOKEN=$(echo "$REFRESH_RESP" | jget access_token "")
NEW_REFRESH=$(echo "$REFRESH_RESP" | jget refresh_token "")
if [[ -n "$NEW_TOKEN" && "$NEW_TOKEN" != "__MISSING__" ]]; then
  pass "Token refresh successful"
  TOKEN="$NEW_TOKEN"
  REFRESH="$NEW_REFRESH"
  AUTH="Authorization: Bearer $TOKEN"
else
  fail "Token refresh failed: $REFRESH_RESP"
fi

subhdr "Verify Password"
VERIFY_PASS_STATUS=$(http_status -X POST "$API/auth/verify-password" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"password\":\"$PASS\"}")
if [[ "$VERIFY_PASS_STATUS" == "200" || "$VERIFY_PASS_STATUS" == "204" ]]; then
  pass "Verify-password accepts correct password (HTTP $VERIFY_PASS_STATUS)"
else
  fail "Verify-password returned $VERIFY_PASS_STATUS (expected 200/204)"
fi

subhdr "Change Password"
CHANGE_PASS_STATUS=$(http_status -X PUT "$API/auth/password" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"current_password\":\"$PASS\",\"new_password\":\"NewPass456!\"}")
if [[ "$CHANGE_PASS_STATUS" == "200" || "$CHANGE_PASS_STATUS" == "204" ]]; then
  pass "Password change accepted (HTTP $CHANGE_PASS_STATUS)"
else
  fail "Password change returned $CHANGE_PASS_STATUS (expected 200/204)"
fi

# Change it back
curl -s --max-time 10 -X PUT "$API/auth/password" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"current_password\":\"NewPass456!\",\"new_password\":\"$PASS\"}" > /dev/null

subhdr "2FA Status"
TFA_STATUS=$(curl -s --max-time 10 "$API/auth/2fa/status" -H "$AUTH")
assert_json "2FA is disabled by default" "$TFA_STATUS" "totp_enabled" "false"

subhdr "2FA Setup"
TFA_SETUP=$(curl -s --max-time 10 -X POST "$API/auth/2fa/setup" -H "$AUTH")
if echo "$TFA_SETUP" | grep -q "otpauth_uri"; then
  pass "2FA setup returns otpauth URI"
  assert_contains "2FA setup returns backup codes" "$TFA_SETUP" "backup_codes"
else
  warn "2FA setup returned unexpected response: ${TFA_SETUP:0:100}"
fi
# Note: We do NOT confirm 2FA (would lock us out without a real TOTP app)

subhdr "Unauthenticated Access Blocked"
UNAUTH_STATUS=$(http_status "$API/photos")
if [[ "$UNAUTH_STATUS" == "401" ]]; then
  pass "Photos endpoint rejects unauthenticated requests"
else
  fail "Photos endpoint should return 401 without auth, got $UNAUTH_STATUS"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 4: ADMIN USER MANAGEMENT
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 4: Admin User Management"

subhdr "Create Second User"
CREATE_USER=$(curl -s --max-time 10 -X POST "$API/admin/users" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER2\",\"password\":\"$PASS2\",\"role\":\"user\"}")
USER2_ID=$(echo "$CREATE_USER" | jget user_id "")
assert_contains "Second user created" "$CREATE_USER" "user_id"
assert_json "Second user has role=user" "$CREATE_USER" "role" "user"

subhdr "List Users"
USERS=$(curl -s --max-time 10 "$API/admin/users" -H "$AUTH")
if echo "$USERS" | python3 -c "import sys,json; d=json.load(sys.stdin); assert len(d)>=2" 2>/dev/null; then
  pass "List users returns at least 2 users"
else
  fail "List users should return at least 2 users"
fi

subhdr "Update User Role"
if [[ -n "$USER2_ID" && "$USER2_ID" != "__MISSING__" ]]; then
  ROLE_UPDATE=$(curl -s --max-time 10 -X PUT "$API/admin/users/$USER2_ID/role" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"role":"admin"}')
  assert_contains "Role update succeeded" "$ROLE_UPDATE" "role"

  # Change back to user for remaining tests
  curl -s --max-time 10 -X PUT "$API/admin/users/$USER2_ID/role" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"role":"user"}' > /dev/null
fi

subhdr "Admin Reset Password"
if [[ -n "$USER2_ID" && "$USER2_ID" != "__MISSING__" ]]; then
  ADMIN_RESET=$(curl -s --max-time 10 -X PUT "$API/admin/users/$USER2_ID/password" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"new_password\":\"$PASS2\"}")
  assert_contains "Admin password reset succeeded" "$ADMIN_RESET" "message"
fi

subhdr "Non-admin Access Blocked"
# Login as user2 and try admin endpoint
LOGIN2=$(curl -s --max-time 10 -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER2\",\"password\":\"$PASS2\"}")
TOKEN2=$(echo "$LOGIN2" | jget access_token "")
if [[ -n "$TOKEN2" && "$TOKEN2" != "__MISSING__" ]]; then
  pass "Second user can log in"
  AUTH2="Authorization: Bearer $TOKEN2"
  REFRESH2=$(echo "$LOGIN2" | jget refresh_token "")

  NON_ADMIN_STATUS=$(http_status "$API/admin/users" -H "$AUTH2")
  if [[ "$NON_ADMIN_STATUS" == "403" ]]; then
    pass "Non-admin user blocked from admin endpoints (HTTP 403)"
  else
    fail "Non-admin should get 403 from admin endpoints, got $NON_ADMIN_STATUS"
  fi
else
  fail "Second user login failed"
  AUTH2=""
  REFRESH2=""
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 5: ADMIN CONFIGURATION (Storage, Port, SSL)
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 5: Admin Configuration"

subhdr "Get Storage"
STORAGE=$(curl -s --max-time 10 "$API/admin/storage" -H "$AUTH")
assert_contains "Storage path returned" "$STORAGE" "storage_path"
CURRENT_STORAGE=$(echo "$STORAGE" | jget storage_path "")
log "Current storage: $CURRENT_STORAGE"

subhdr "Browse Directory"
BROWSE=$(curl -s --max-time 10 "$API/admin/browse?path=/" -H "$AUTH")
assert_contains "Browse returns directories" "$BROWSE" "directories"

subhdr "Get Port"
PORT_RESP=$(curl -s --max-time 10 "$API/admin/port" -H "$AUTH")
assert_json "Port is 8080" "$PORT_RESP" "port" "8080"

subhdr "Get SSL Status"
SSL_RESP=$(curl -s --max-time 10 "$API/admin/ssl" -H "$AUTH")
assert_json "SSL is disabled" "$SSL_RESP" "enabled" "false"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 6: PHOTO SCAN & CONVERSION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 6: Photo Scan & Conversion"

subhdr "Trigger Photo Scan"
log "Scan may take several minutes (FFmpeg thumbnails for videos)..."
SCAN=$(curl -s --max-time 600 -X POST "$API/admin/photos/scan" -H "$AUTH")
log "Scan result: ${SCAN:0:200}"
REGISTERED=$(echo "$SCAN" | jget registered 0)
log "New photos registered: $REGISTERED"

# Check total photos in DB
PHOTOS_LIST=$(curl -s --max-time 10 "$API/photos" -H "$AUTH")
PHOTO_COUNT=$(echo "$PHOTOS_LIST" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    photos = d.get('photos', d) if isinstance(d, dict) else d
    print(len(photos))
except:
    print('0')
" 2>/dev/null)

if [[ "$PHOTO_COUNT" -gt 0 ]]; then
  pass "Photos found in database: $PHOTO_COUNT"
else
  warn "No photos found after scan (ensure storage root has media files)"
fi

subhdr "Conversion Status"
CONV=$(curl -s --max-time 10 "$API/photos/conversion-status" -H "$AUTH")
assert_contains "Conversion status has pending_conversions field" "$CONV" "pending_conversions"
assert_contains "Conversion status has converting field" "$CONV" "converting"
assert_contains "Conversion status has missing_thumbnails field" "$CONV" "missing_thumbnails"

subhdr "Trigger Convert"
TRIGGER_CONV=$(curl -s --max-time 10 -X POST "$API/admin/photos/convert" -H "$AUTH")
assert_contains "Convert trigger accepted" "$TRIGGER_CONV" "message"

subhdr "Wait for Conversions (up to 3 min)"
CONV_DONE=false
for i in $(seq 1 60); do
  CS=$(curl -s --max-time 5 "$API/photos/conversion-status" -H "$AUTH" 2>/dev/null || echo "{}")
  P=$(echo "$CS" | jget pending_conversions 0)
  A=$(echo "$CS" | jget converting false)
  T=$(echo "$CS" | jget missing_thumbnails 0)

  if [[ "$i" -eq 1 ]] || (( i % 10 == 0 )); then
    log "  [$i] pending=$P converting=$A thumbs=$T"
  fi

  if [[ "$P" == "0" && "$A" == "false" && "$T" == "0" ]]; then
    pass "Conversions complete (waited ~$((i*3))s)"
    CONV_DONE=true
    break
  fi
  sleep 3
done
if [[ "$CONV_DONE" != "true" ]]; then
  warn "Conversions still running after 3 min — continuing"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 7: PHOTOS (List, Serve, Favorite, Crop, Metadata)
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 7: Photos"

subhdr "List Photos"
PHOTOS=$(curl -s --max-time 10 "$API/photos" -H "$AUTH")
assert_contains "Photos response has 'photos' array" "$PHOTOS" "photos"

# Get first photo ID for further tests
FIRST_PHOTO_ID=$(echo "$PHOTOS" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    photos = d.get('photos', [])
    if photos:
        print(photos[0]['id'])
    else:
        print('')
except:
    print('')
" 2>/dev/null)

if [[ -n "$FIRST_PHOTO_ID" ]]; then
  log "Testing with photo ID: $FIRST_PHOTO_ID"

  subhdr "List Photos with Pagination"
  PHOTOS_LIM=$(curl -s --max-time 10 "$API/photos?limit=2" -H "$AUTH")
  assert_contains "Paginated photos response" "$PHOTOS_LIM" "photos"

  subhdr "List Photos by Media Type"
  PHOTOS_TYPE=$(curl -s --max-time 10 "$API/photos?media_type=photo" -H "$AUTH")
  assert_contains "Media type filter works" "$PHOTOS_TYPE" "photos"

  subhdr "Serve Photo File"
  SERVE_STATUS=$(http_status "$API/photos/$FIRST_PHOTO_ID/file" -H "$AUTH")
  if [[ "$SERVE_STATUS" == "200" || "$SERVE_STATUS" == "206" ]]; then
    pass "Serve photo returns 200/206"
  else
    fail "Serve photo returned unexpected status: $SERVE_STATUS"
  fi

  subhdr "Serve Thumbnail"
  THUMB_STATUS=$(http_status "$API/photos/$FIRST_PHOTO_ID/thumb" -H "$AUTH")
  if [[ "$THUMB_STATUS" == "200" || "$THUMB_STATUS" == "404" ]]; then
    pass "Serve thumbnail returns 200/404 (HTTP $THUMB_STATUS)"
  else
    fail "Serve thumbnail returned unexpected status: $THUMB_STATUS"
  fi

  subhdr "Serve Web Preview"
  WEB_STATUS=$(http_status "$API/photos/$FIRST_PHOTO_ID/web" -H "$AUTH")
  if [[ "$WEB_STATUS" == "200" || "$WEB_STATUS" == "404" || "$WEB_STATUS" == "302" ]]; then
    pass "Serve web preview returns expected status (HTTP $WEB_STATUS)"
  else
    fail "Serve web preview returned unexpected status: $WEB_STATUS"
  fi

  subhdr "Toggle Favorite"
  FAV=$(curl -s --max-time 10 -X PUT "$API/photos/$FIRST_PHOTO_ID/favorite" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"is_favorite":true}')
  assert_json "Favorite toggled on" "$FAV" "is_favorite" "true"

  # Toggle back off
  FAV_OFF=$(curl -s --max-time 10 -X PUT "$API/photos/$FIRST_PHOTO_ID/favorite" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"is_favorite":false}')
  assert_json "Favorite toggled off" "$FAV_OFF" "is_favorite" "false"

  subhdr "List Favorites Only"
  # First favorite a photo, then query
  curl -s --max-time 10 -X PUT "$API/photos/$FIRST_PHOTO_ID/favorite" \
    -H "$AUTH" -H 'Content-Type: application/json' -d '{"is_favorite":true}' > /dev/null
  FAVS=$(curl -s --max-time 10 "$API/photos?favorites_only=true" -H "$AUTH")
  FAV_COUNT=$(echo "$FAVS" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d.get('photos', [])))
except:
    print('0')
" 2>/dev/null)
  if [[ "$FAV_COUNT" -gt 0 ]]; then
    pass "Favorites filter returns $FAV_COUNT favorite(s)"
  else
    fail "Favorites filter returned 0 — expected at least 1"
  fi
  # Un-favorite for clean state
  curl -s --max-time 10 -X PUT "$API/photos/$FIRST_PHOTO_ID/favorite" \
    -H "$AUTH" -H 'Content-Type: application/json' -d '{"is_favorite":false}' > /dev/null

  subhdr "Set Crop Metadata"
  CROP=$(curl -s --max-time 10 -X PUT "$API/photos/$FIRST_PHOTO_ID/crop" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"crop_metadata":"{\"x\":10,\"y\":20,\"w\":100,\"h\":100}"}')
  assert_contains "Crop metadata set" "$CROP" "crop_metadata"

  # Clear crop
  curl -s --max-time 10 -X PUT "$API/photos/$FIRST_PHOTO_ID/crop" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"crop_metadata":null}' > /dev/null

  subhdr "Get Photo Metadata (import endpoint)"
  META_STATUS=$(http_status "$API/photos/$FIRST_PHOTO_ID/metadata" -H "$AUTH")
  # May return 200 with metadata or 404 if no sidecar has been imported
  if [[ "$META_STATUS" == "200" || "$META_STATUS" == "404" ]]; then
    pass "Photo metadata endpoint returns expected status (HTTP $META_STATUS)"
  else
    fail "Photo metadata returned unexpected status: $META_STATUS"
  fi

  subhdr "ETag/If-None-Match Support"
  # Fetch with full headers to get ETag
  ETAG_HEADERS=$(curl -sI --max-time 10 "$API/photos/$FIRST_PHOTO_ID/file" -H "$AUTH")
  ETAG=$(echo "$ETAG_HEADERS" | grep -i "^etag:" | sed 's/[Ee][Tt][Aa][Gg]:\s*//' | tr -d '\r\n')
  if [[ -n "$ETAG" ]]; then
    CACHED_STATUS=$(http_status "$API/photos/$FIRST_PHOTO_ID/file" -H "$AUTH" -H "If-None-Match: $ETAG")
    if [[ "$CACHED_STATUS" == "304" ]]; then
      pass "ETag caching returns 304 Not Modified"
    else
      warn "ETag caching returned $CACHED_STATUS (expected 304)"
    fi
  else
    warn "No ETag header returned for photo file"
  fi

else
  warn "No photos available — skipping photo-specific tests"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 8: TAGS & SEARCH
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 8: Tags & Search"

subhdr "List Tags (initially empty)"
TAGS=$(curl -s --max-time 10 "$API/tags" -H "$AUTH")
assert_contains "Tags response has 'tags' field" "$TAGS" "tags"

if [[ -n "$FIRST_PHOTO_ID" ]]; then
  subhdr "Add Tag to Photo"
  ADD_TAG_STATUS=$(http_status -X POST "$API/photos/$FIRST_PHOTO_ID/tags" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"tag":"sunset"}')
  if [[ "$ADD_TAG_STATUS" == "201" ]]; then
    pass "Tag 'sunset' added (HTTP 201)"
  else
    fail "Add tag returned $ADD_TAG_STATUS (expected 201)"
  fi

  # Add a second tag
  http_status -X POST "$API/photos/$FIRST_PHOTO_ID/tags" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"tag":"nature"}' > /dev/null

  subhdr "Get Photo Tags"
  PHOTO_TAGS=$(curl -s --max-time 10 "$API/photos/$FIRST_PHOTO_ID/tags" -H "$AUTH")
  assert_contains "Photo tags include 'sunset'" "$PHOTO_TAGS" "sunset"

  subhdr "List All Tags"
  ALL_TAGS=$(curl -s --max-time 10 "$API/tags" -H "$AUTH")
  assert_contains "Global tag list includes 'sunset'" "$ALL_TAGS" "sunset"
  assert_contains "Global tag list includes 'nature'" "$ALL_TAGS" "nature"

  subhdr "Search by Tag"
  SEARCH=$(curl -s --max-time 10 "$API/search?q=sunset" -H "$AUTH")
  assert_contains "Search returns results" "$SEARCH" "results"
  SEARCH_COUNT=$(echo "$SEARCH" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d.get('results', [])))
except:
    print('0')
" 2>/dev/null)
  if [[ "$SEARCH_COUNT" -gt 0 ]]; then
    pass "Search for 'sunset' found $SEARCH_COUNT result(s)"
  else
    fail "Search for 'sunset' returned 0 results"
  fi

  subhdr "Search with Limit"
  SEARCH_LIM=$(curl -s --max-time 10 "$API/search?q=sunset&limit=1" -H "$AUTH")
  assert_contains "Limited search returns results" "$SEARCH_LIM" "results"

  subhdr "Remove Tag"
  REMOVE_TAG_STATUS=$(http_status -X DELETE "$API/photos/$FIRST_PHOTO_ID/tags" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"tag":"nature"}')
  if [[ "$REMOVE_TAG_STATUS" == "204" ]]; then
    pass "Tag 'nature' removed (HTTP 204)"
  else
    fail "Remove tag returned $REMOVE_TAG_STATUS (expected 204)"
  fi

  # Verify removal
  PHOTO_TAGS2=$(curl -s --max-time 10 "$API/photos/$FIRST_PHOTO_ID/tags" -H "$AUTH")
  if echo "$PHOTO_TAGS2" | grep -q "nature"; then
    fail "Tag 'nature' still present after removal"
  else
    pass "Tag 'nature' successfully removed"
  fi

  # Clean up remaining tag
  curl -s -X DELETE "$API/photos/$FIRST_PHOTO_ID/tags" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"tag":"sunset"}' > /dev/null
else
  warn "No photos — skipping tag tests"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 9: PHOTO COPIES & DUPLICATES
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 9: Photo Copies & Duplicates"

if [[ -n "$FIRST_PHOTO_ID" ]]; then
  subhdr "Duplicate Photo"
  DUP=$(curl -s --max-time 30 -X POST "$API/photos/$FIRST_PHOTO_ID/duplicate" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{}')
  DUP_ID=$(echo "$DUP" | jget id "")
  assert_contains "Duplicate created" "$DUP" "id"
  assert_contains "Duplicate references source" "$DUP" "source_photo_id"

  subhdr "Create Edit Copy"
  EDIT_COPY=$(curl -s --max-time 10 -X POST "$API/photos/$FIRST_PHOTO_ID/copies" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"name":"Brightness +50","edit_metadata":"{\"brightness\":50}"}')
  COPY_ID=$(echo "$EDIT_COPY" | jget id "")
  assert_contains "Edit copy created" "$EDIT_COPY" "edit_metadata"

  subhdr "List Edit Copies"
  COPIES=$(curl -s --max-time 10 "$API/photos/$FIRST_PHOTO_ID/copies" -H "$AUTH")
  assert_contains "Edit copies list returned" "$COPIES" "copies"

  subhdr "Delete Edit Copy"
  if [[ -n "$COPY_ID" && "$COPY_ID" != "__MISSING__" ]]; then
    DEL_COPY_STATUS=$(http_status -X DELETE "$API/photos/$FIRST_PHOTO_ID/copies/$COPY_ID" -H "$AUTH")
    if [[ "$DEL_COPY_STATUS" == "204" || "$DEL_COPY_STATUS" == "200" ]]; then
      pass "Edit copy deleted (HTTP $DEL_COPY_STATUS)"
    else
      fail "Delete edit copy returned $DEL_COPY_STATUS (expected 204)"
    fi
  fi

  # Clean up duplicate
  if [[ -n "$DUP_ID" && "$DUP_ID" != "__MISSING__" ]]; then
    curl -s -X DELETE "$API/photos/$DUP_ID" -H "$AUTH" > /dev/null
    # Also empty trash of duplicate
    sleep 1
    curl -s -X DELETE "$API/trash" -H "$AUTH" > /dev/null
  fi
else
  warn "No photos — skipping copy/duplicate tests"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 10: TRASH (Soft-Delete, List, Restore, Permanent Delete, Empty)
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 10: Trash"

if [[ -n "$FIRST_PHOTO_ID" ]]; then
  # We need a sacrificial photo — duplicate the first one
  DUP_TRASH=$(curl -s --max-time 30 -X POST "$API/photos/$FIRST_PHOTO_ID/duplicate" \
    -H "$AUTH" -H 'Content-Type: application/json' -d '{}')
  TRASH_TEST_ID=$(echo "$DUP_TRASH" | jget id "")

  if [[ -n "$TRASH_TEST_ID" && "$TRASH_TEST_ID" != "__MISSING__" ]]; then
    subhdr "Soft Delete Photo"
    DEL_STATUS=$(http_status -X DELETE "$API/photos/$TRASH_TEST_ID" -H "$AUTH")
    if [[ "$DEL_STATUS" == "204" ]]; then
      pass "Photo soft-deleted (HTTP 204)"
    else
      fail "Soft delete returned $DEL_STATUS (expected 204)"
    fi

    subhdr "List Trash"
    TRASH=$(curl -s --max-time 10 "$API/trash" -H "$AUTH")
    assert_contains "Trash contains items" "$TRASH" "items"
    TRASH_COUNT=$(echo "$TRASH" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d.get('items', [])))
except:
    print('0')
" 2>/dev/null)
    if [[ "$TRASH_COUNT" -gt 0 ]]; then
      pass "Trash has $TRASH_COUNT item(s)"
    else
      fail "Trash should have at least 1 item"
    fi

    # Get trash item ID
    TRASH_ITEM_ID=$(echo "$TRASH" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    items = d.get('items', [])
    print(items[0]['id'])
except:
    print('')
" 2>/dev/null)

    subhdr "Serve Trash Thumbnail"
    if [[ -n "$TRASH_ITEM_ID" ]]; then
      TRASH_THUMB_STATUS=$(http_status "$API/trash/$TRASH_ITEM_ID/thumb" -H "$AUTH")
      if [[ "$TRASH_THUMB_STATUS" == "200" || "$TRASH_THUMB_STATUS" == "404" ]]; then
        pass "Trash thumbnail returns expected status (HTTP $TRASH_THUMB_STATUS)"
      else
        fail "Trash thumbnail returned $TRASH_THUMB_STATUS"
      fi
    fi

    subhdr "Restore from Trash"
    if [[ -n "$TRASH_ITEM_ID" ]]; then
      RESTORE_STATUS=$(http_status -X POST "$API/trash/$TRASH_ITEM_ID/restore" -H "$AUTH")
      if [[ "$RESTORE_STATUS" == "200" || "$RESTORE_STATUS" == "204" ]]; then
        pass "Photo restored from trash (HTTP $RESTORE_STATUS)"
      else
        fail "Restore returned $RESTORE_STATUS (expected 200/204)"
      fi
    fi

    # Verify the photo is back in the photos list
    PHOTOS_AFTER_RESTORE=$(curl -s --max-time 10 "$API/photos" -H "$AUTH")
    if echo "$PHOTOS_AFTER_RESTORE" | grep -q "$TRASH_TEST_ID"; then
      pass "Restored photo appears in photos list"
    else
      warn "Restored photo not found in photos list (may have different ID)"
    fi

    # Re-delete for permanent delete test
    curl -s -X DELETE "$API/photos/$TRASH_TEST_ID" -H "$AUTH" > /dev/null 2>&1
    sleep 1

    subhdr "Permanent Delete"
    TRASH2=$(curl -s --max-time 10 "$API/trash" -H "$AUTH")
    TRASH_ITEM_ID2=$(echo "$TRASH2" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    items = d.get('items', [])
    print(items[0]['id'] if items else '')
except:
    print('')
" 2>/dev/null)
    if [[ -n "$TRASH_ITEM_ID2" ]]; then
      PERM_DEL_STATUS=$(http_status -X DELETE "$API/trash/$TRASH_ITEM_ID2" -H "$AUTH")
      if [[ "$PERM_DEL_STATUS" == "204" || "$PERM_DEL_STATUS" == "200" ]]; then
        pass "Permanent delete succeeded (HTTP $PERM_DEL_STATUS)"
      else
        fail "Permanent delete returned $PERM_DEL_STATUS"
      fi
    else
      warn "No trash items for permanent delete test"
    fi
  else
    warn "Could not create duplicate for trash tests"
  fi

  subhdr "Empty Trash"
  # Add something to trash first, then empty
  DUP_EMPTY=$(curl -s --max-time 30 -X POST "$API/photos/$FIRST_PHOTO_ID/duplicate" \
    -H "$AUTH" -H 'Content-Type: application/json' -d '{}')
  EMPTY_DUP_ID=$(echo "$DUP_EMPTY" | jget id "")
  if [[ -n "$EMPTY_DUP_ID" && "$EMPTY_DUP_ID" != "__MISSING__" ]]; then
    curl -s -X DELETE "$API/photos/$EMPTY_DUP_ID" -H "$AUTH" > /dev/null
    sleep 1
    EMPTY_STATUS=$(http_status -X DELETE "$API/trash" -H "$AUTH")
    if [[ "$EMPTY_STATUS" == "200" || "$EMPTY_STATUS" == "204" ]]; then
      pass "Empty trash succeeded (HTTP $EMPTY_STATUS)"
    else
      fail "Empty trash returned $EMPTY_STATUS"
    fi
  fi

  # Verify trash is empty
  TRASH_FINAL=$(curl -s --max-time 10 "$API/trash" -H "$AUTH")
  TRASH_FINAL_COUNT=$(echo "$TRASH_FINAL" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d.get('items', [])))
except:
    print('0')
" 2>/dev/null)
  if [[ "$TRASH_FINAL_COUNT" == "0" ]]; then
    pass "Trash is empty after empty-all"
  else
    warn "Trash still has $TRASH_FINAL_COUNT items after empty"
  fi
else
  warn "No photos — skipping trash tests"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 11: SHARED ALBUMS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 11: Shared Albums"

subhdr "List Shared Albums (initially empty)"
SHARED=$(curl -s --max-time 10 "$API/sharing/albums" -H "$AUTH")
# Should be an empty array
if echo "$SHARED" | python3 -c "import sys,json; d=json.load(sys.stdin); assert isinstance(d, list)" 2>/dev/null; then
  pass "Shared albums returns array"
else
  fail "Shared albums should return array"
fi

subhdr "Create Shared Album"
CREATE_ALBUM=$(curl -s --max-time 10 -X POST "$API/sharing/albums" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"name":"Family Vacation"}')
ALBUM_ID=$(echo "$CREATE_ALBUM" | jget id "")
if [[ -n "$ALBUM_ID" && "$ALBUM_ID" != "__MISSING__" ]]; then
  pass "Shared album created: $ALBUM_ID"
else
  fail "Failed to create shared album: $CREATE_ALBUM"
fi

subhdr "List Users for Sharing"
SHARE_USERS=$(curl -s --max-time 10 "$API/sharing/users" -H "$AUTH")
if echo "$SHARE_USERS" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Share users list returned valid JSON"
else
  fail "Share users list returned invalid JSON"
fi

if [[ -n "$ALBUM_ID" && "$ALBUM_ID" != "__MISSING__" ]]; then
  subhdr "Add Member to Album"
  if [[ -n "$USER2_ID" && "$USER2_ID" != "__MISSING__" ]]; then
    ADD_MEMBER=$(curl -s --max-time 10 -X POST "$API/sharing/albums/$ALBUM_ID/members" \
      -H "$AUTH" -H 'Content-Type: application/json' \
      -d "{\"user_id\":\"$USER2_ID\"}")
    assert_contains "Member added to album" "$ADD_MEMBER" "member_id"
  fi

  subhdr "List Album Members"
  MEMBERS=$(curl -s --max-time 10 "$API/sharing/albums/$ALBUM_ID/members" -H "$AUTH")
  # Response is a bare array of member objects
  assert_contains "Members list contains user" "$MEMBERS" "user_id"

  subhdr "Add Photo to Album"
  if [[ -n "$FIRST_PHOTO_ID" ]]; then
    ADD_PHOTO_STATUS=$(http_status -X POST "$API/sharing/albums/$ALBUM_ID/photos" \
      -H "$AUTH" -H 'Content-Type: application/json' \
      -d "{\"photo_ref\":\"$FIRST_PHOTO_ID\",\"ref_type\":\"plain\"}")
    if [[ "$ADD_PHOTO_STATUS" == "201" || "$ADD_PHOTO_STATUS" == "200" ]]; then
      pass "Photo added to shared album (HTTP $ADD_PHOTO_STATUS)"
    else
      fail "Add photo to album returned $ADD_PHOTO_STATUS"
    fi
  fi

  subhdr "List Album Photos"
  ALBUM_PHOTOS=$(curl -s --max-time 10 "$API/sharing/albums/$ALBUM_ID/photos" -H "$AUTH")
  # Response is a bare array of photo objects
  assert_contains "Album photos list returned" "$ALBUM_PHOTOS" "photo_ref"

  subhdr "Remove Photo from Album"
  if [[ -n "$FIRST_PHOTO_ID" ]]; then
    RM_PHOTO_STATUS=$(http_status -X DELETE "$API/sharing/albums/$ALBUM_ID/photos/$FIRST_PHOTO_ID" -H "$AUTH")
    if [[ "$RM_PHOTO_STATUS" == "204" || "$RM_PHOTO_STATUS" == "200" ]]; then
      pass "Photo removed from shared album (HTTP $RM_PHOTO_STATUS)"
    else
      fail "Remove album photo returned $RM_PHOTO_STATUS"
    fi
  fi

  subhdr "Remove Member from Album"
  if [[ -n "$USER2_ID" && "$USER2_ID" != "__MISSING__" ]]; then
    RM_MEMBER_STATUS=$(http_status -X DELETE "$API/sharing/albums/$ALBUM_ID/members/$USER2_ID" -H "$AUTH")
    if [[ "$RM_MEMBER_STATUS" == "204" || "$RM_MEMBER_STATUS" == "200" ]]; then
      pass "Member removed from album (HTTP $RM_MEMBER_STATUS)"
    else
      fail "Remove member returned $RM_MEMBER_STATUS"
    fi
  fi

  subhdr "Delete Shared Album"
  DEL_ALBUM_STATUS=$(http_status -X DELETE "$API/sharing/albums/$ALBUM_ID" -H "$AUTH")
  if [[ "$DEL_ALBUM_STATUS" == "204" || "$DEL_ALBUM_STATUS" == "200" ]]; then
    pass "Shared album deleted (HTTP $DEL_ALBUM_STATUS)"
  else
    fail "Delete shared album returned $DEL_ALBUM_STATUS"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 12: SECURE GALLERIES
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 12: Secure Galleries"

subhdr "List Secure Galleries (initially empty)"
GALLERIES=$(curl -s --max-time 10 "$API/galleries/secure" -H "$AUTH")
assert_contains "Galleries response has 'galleries' field" "$GALLERIES" "galleries"

subhdr "Create Secure Gallery"
CREATE_GALLERY=$(curl -s --max-time 10 -X POST "$API/galleries/secure" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"name":"Secret Album"}')
GALLERY_ID=$(echo "$CREATE_GALLERY" | jget gallery_id "")
if [[ -n "$GALLERY_ID" && "$GALLERY_ID" != "__MISSING__" ]]; then
  pass "Secure gallery created: $GALLERY_ID"
else
  fail "Failed to create secure gallery: $CREATE_GALLERY"
fi

subhdr "Unlock Secure Galleries"
UNLOCK=$(curl -s --max-time 10 -X POST "$API/galleries/secure/unlock" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"password\":\"$PASS\"}")
GALLERY_TOKEN=$(echo "$UNLOCK" | jget gallery_token "")
if [[ -n "$GALLERY_TOKEN" && "$GALLERY_TOKEN" != "__MISSING__" ]]; then
  pass "Gallery unlocked (token: ${GALLERY_TOKEN:0:20}...)"
else
  fail "Gallery unlock failed: $UNLOCK"
fi

subhdr "Unlock with Wrong Password"
BAD_UNLOCK_STATUS=$(http_status -X POST "$API/galleries/secure/unlock" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"password":"WrongPass!"}')
if [[ "$BAD_UNLOCK_STATUS" == "401" ]]; then
  pass "Wrong gallery password rejected (HTTP 401)"
else
  fail "Wrong gallery password should return 401, got $BAD_UNLOCK_STATUS"
fi

if [[ -n "$GALLERY_ID" && "$GALLERY_ID" != "__MISSING__" && -n "$GALLERY_TOKEN" ]]; then
  subhdr "List Gallery Items (empty)"
  ITEMS=$(curl -s --max-time 10 "$API/galleries/secure/$GALLERY_ID/items" \
    -H "$AUTH" -H "X-Gallery-Token: $GALLERY_TOKEN")
  assert_contains "Gallery items response" "$ITEMS" "items"

  subhdr "Add Item to Gallery"
  if [[ -n "$FIRST_PHOTO_ID" ]]; then
    ADD_ITEM_STATUS=$(http_status -X POST "$API/galleries/secure/$GALLERY_ID/items" \
      -H "$AUTH" -H "X-Gallery-Token: $GALLERY_TOKEN" \
      -H 'Content-Type: application/json' \
      -d "{\"blob_id\":\"$FIRST_PHOTO_ID\"}")
    if [[ "$ADD_ITEM_STATUS" == "201" || "$ADD_ITEM_STATUS" == "200" ]]; then
      pass "Item added to gallery (HTTP $ADD_ITEM_STATUS)"
    else
      warn "Add gallery item returned $ADD_ITEM_STATUS"
    fi
  fi

  subhdr "List Secure Blob IDs"
  BLOB_IDS=$(curl -s --max-time 10 "$API/galleries/secure/blob-ids" \
    -H "$AUTH" -H "X-Gallery-Token: $GALLERY_TOKEN")
  if echo "$BLOB_IDS" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
    pass "Secure blob IDs endpoint returns valid JSON"
  else
    warn "Secure blob IDs response: ${BLOB_IDS:0:100}"
  fi

  subhdr "Delete Secure Gallery"
  DEL_GALLERY_STATUS=$(http_status -X DELETE "$API/galleries/secure/$GALLERY_ID" -H "$AUTH")
  if [[ "$DEL_GALLERY_STATUS" == "204" || "$DEL_GALLERY_STATUS" == "200" ]]; then
    pass "Secure gallery deleted (HTTP $DEL_GALLERY_STATUS)"
  else
    fail "Delete gallery returned $DEL_GALLERY_STATUS"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 13: BLOB UPLOAD/DOWNLOAD/DELETE
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 13: Blob Management"

subhdr "Upload Blob"
# Create a small test file to upload
TEST_BLOB="${TMPDIR:-/tmp}/e2e_test_blob.bin"
dd if=/dev/urandom of="$TEST_BLOB" bs=1024 count=4 status=none 2>/dev/null
TEST_HASH=$(sha256sum "$TEST_BLOB" | cut -d' ' -f1)

UPLOAD_RESP=$(curl -s --max-time 10 -X POST "$API/blobs" \
  -H "$AUTH" \
  -H "x-blob-type: photo" \
  -H "x-client-hash: $TEST_HASH" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@$TEST_BLOB")
BLOB_ID=$(echo "$UPLOAD_RESP" | jget blob_id "")
if [[ -n "$BLOB_ID" && "$BLOB_ID" != "__MISSING__" ]]; then
  pass "Blob uploaded: $BLOB_ID"
else
  fail "Blob upload failed: $UPLOAD_RESP"
fi

subhdr "List Blobs"
BLOBS=$(curl -s --max-time 10 "$API/blobs" -H "$AUTH")
assert_contains "Blobs list returned" "$BLOBS" "blobs"

if [[ -n "$BLOB_ID" && "$BLOB_ID" != "__MISSING__" ]]; then
  subhdr "Download Blob"
  DL_STATUS=$(http_status "$API/blobs/$BLOB_ID" -H "$AUTH")
  if [[ "$DL_STATUS" == "200" ]]; then
    pass "Blob downloaded (HTTP 200)"
  else
    fail "Blob download returned $DL_STATUS"
  fi

  subhdr "Download Blob Thumbnail"
  THUMB_BLOB_STATUS=$(http_status "$API/blobs/$BLOB_ID/thumb" -H "$AUTH")
  # Thumbnail may not exist for raw blob
  if [[ "$THUMB_BLOB_STATUS" == "200" || "$THUMB_BLOB_STATUS" == "404" ]]; then
    pass "Blob thumbnail returns expected status (HTTP $THUMB_BLOB_STATUS)"
  else
    fail "Blob thumbnail returned $THUMB_BLOB_STATUS"
  fi

  subhdr "Soft-Delete Blob to Trash"
  TRASH_BLOB=$(curl -s --max-time 10 -X POST "$API/blobs/$BLOB_ID/trash" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"filename\":\"test.bin\",\"mime_type\":\"application/octet-stream\"}")
  assert_contains "Blob moved to trash" "$TRASH_BLOB" "trash_id"

  # Clean up trash
  curl -s -X DELETE "$API/trash" -H "$AUTH" > /dev/null
fi
rm -f "$TEST_BLOB"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 14: ENCRYPTION SETTINGS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 14: Encryption Settings"

subhdr "Get Encryption Settings"
ENC=$(curl -s --max-time 10 "$API/settings/encryption" -H "$AUTH")
assert_json "Default mode is plain" "$ENC" "encryption_mode" "plain"
assert_json "Migration status is idle" "$ENC" "migration_status" "idle"
assert_contains "Migration total field present" "$ENC" "migration_total"
assert_contains "Migration completed field present" "$ENC" "migration_completed"

# Note: We don't actually enable encryption in this test to avoid
# requiring key derivation and risking data loss.

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 15: STORAGE STATS & CLEANUP STATUS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 15: Storage Stats & Cleanup"

subhdr "Storage Stats"
STATS=$(curl -s --max-time 10 "$API/settings/storage-stats" -H "$AUTH")
assert_contains "Storage stats has fs_total_bytes" "$STATS" "fs_total_bytes"
assert_contains "Storage stats has fs_free_bytes" "$STATS" "fs_free_bytes"

subhdr "Cleanup Status"
CLEANUP=$(curl -s --max-time 10 "$API/photos/cleanup-status" -H "$AUTH")
assert_contains "Cleanup status has cleanable_count" "$CLEANUP" "cleanable_count"
assert_contains "Cleanup status has cleanable_bytes" "$CLEANUP" "cleanable_bytes"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 16: BACKUP SERVER MANAGEMENT
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 16: Backup Server Management"

subhdr "Get Backup Mode"
BK_MODE=$(curl -s --max-time 10 "$API/admin/backup/mode" -H "$AUTH")
assert_contains "Backup mode has 'mode' field" "$BK_MODE" "mode"

subhdr "List Backup Servers (initially empty)"
BK_SERVERS=$(curl -s --max-time 10 "$API/admin/backup/servers" -H "$AUTH")
assert_contains "Backup servers response" "$BK_SERVERS" "servers"

subhdr "Add Backup Server"
ADD_BK=$(curl -s --max-time 10 -X POST "$API/admin/backup/servers" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"name":"Test Backup","address":"http://192.168.1.99:8080","sync_frequency_hours":24}')
BK_SERVER_ID=$(echo "$ADD_BK" | jget id "")
if [[ -n "$BK_SERVER_ID" && "$BK_SERVER_ID" != "__MISSING__" ]]; then
  pass "Backup server added: $BK_SERVER_ID"
else
  fail "Backup server add failed: $ADD_BK"
fi

if [[ -n "$BK_SERVER_ID" && "$BK_SERVER_ID" != "__MISSING__" ]]; then
  subhdr "Update Backup Server"
  UPDATE_BK=$(curl -s --max-time 10 -X PUT "$API/admin/backup/servers/$BK_SERVER_ID" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"name":"Updated Backup","enabled":false}')
  assert_contains "Backup server updated" "$UPDATE_BK" "message"

  subhdr "Check Backup Server Status"
  BK_STATUS=$(curl -s --max-time 10 "$API/admin/backup/servers/$BK_SERVER_ID/status" -H "$AUTH")
  assert_contains "Backup server status has 'reachable'" "$BK_STATUS" "reachable"

  subhdr "Get Sync Logs"
  SYNC_LOGS=$(curl -s --max-time 10 "$API/admin/backup/servers/$BK_SERVER_ID/logs" -H "$AUTH")
  if echo "$SYNC_LOGS" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
    pass "Sync logs endpoint returns valid JSON"
  else
    fail "Sync logs returned invalid JSON"
  fi

  subhdr "Delete Backup Server"
  DEL_BK_STATUS=$(http_status -X DELETE "$API/admin/backup/servers/$BK_SERVER_ID" -H "$AUTH")
  if [[ "$DEL_BK_STATUS" == "204" || "$DEL_BK_STATUS" == "200" ]]; then
    pass "Backup server deleted (HTTP $DEL_BK_STATUS)"
  else
    fail "Delete backup server returned $DEL_BK_STATUS"
  fi
fi

subhdr "Audio Backup Setting"
AUDIO_BK=$(curl -s --max-time 10 "$API/settings/audio-backup" -H "$AUTH")
assert_contains "Audio backup setting response" "$AUDIO_BK" "audio_backup"

subhdr "Update Audio Backup Setting"
UPDATE_AUDIO=$(curl -s --max-time 10 -X PUT "$API/admin/audio-backup" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true}')
assert_contains "Audio backup update response" "$UPDATE_AUDIO" "audio_backup"

subhdr "Discover Backup Servers"
DISCOVER=$(curl -s --max-time 15 "$API/admin/backup/discover" -H "$AUTH")
if echo "$DISCOVER" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Backup discover endpoint returns valid JSON"
else
  warn "Backup discover may have timed out or failed"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 17: CLIENT LOGS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 17: Client Logs"

subhdr "Submit Client Logs"
CLIENT_LOG_RESP=$(curl -s --max-time 10 -X POST "$API/client-logs" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{
    "session_id": "e2e-test-session-001",
    "entries": [
      {
        "level": "info",
        "tag": "BackupService",
        "message": "E2E test log entry 1",
        "client_ts": "2024-01-01T12:00:00Z"
      },
      {
        "level": "error",
        "tag": "NetworkError",
        "message": "E2E test error entry",
        "context": {"code": 500, "retry": true},
        "client_ts": "2024-01-01T12:01:00Z"
      }
    ]
  }')
assert_contains "Client logs submitted" "$CLIENT_LOG_RESP" "inserted"

subhdr "List Client Logs (admin)"
CLIENT_LOGS=$(curl -s --max-time 10 "$API/admin/client-logs" -H "$AUTH")
assert_contains "Client logs list has 'logs' field" "$CLIENT_LOGS" "logs"
LOG_COUNT=$(echo "$CLIENT_LOGS" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(len(d.get('logs', [])))
except:
    print('0')
" 2>/dev/null)
if [[ "$LOG_COUNT" -gt 0 ]]; then
  pass "Client logs contains $LOG_COUNT entries"
else
  fail "Client logs should contain entries after submission"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 18: DIAGNOSTICS & AUDIT LOGS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 18: Diagnostics & Audit Logs"

subhdr "Get Diagnostics Config"
DIAG_CONFIG=$(curl -s --max-time 10 "$API/admin/diagnostics/config" -H "$AUTH")
assert_contains "Diagnostics config returned" "$DIAG_CONFIG" "diagnostics_enabled"

subhdr "Enable Diagnostics"
UPDATE_DIAG=$(curl -s --max-time 10 -X PUT "$API/admin/diagnostics/config" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"diagnostics_enabled":true}')
assert_contains "Diagnostics config updated" "$UPDATE_DIAG" "diagnostics_enabled"

subhdr "Get Full Diagnostics"
DIAG=$(curl -s --max-time 10 "$API/admin/diagnostics" -H "$AUTH")
assert_contains "Diagnostics has server info" "$DIAG" "server"
assert_contains "Diagnostics has database stats" "$DIAG" "database"
assert_contains "Diagnostics has photo stats" "$DIAG" "photos"
assert_contains "Diagnostics has user stats" "$DIAG" "users"
assert_contains "Diagnostics has performance stats" "$DIAG" "performance"

subhdr "List Audit Logs"
AUDIT=$(curl -s --max-time 10 "$API/admin/audit-logs" -H "$AUTH")
assert_contains "Audit logs has 'logs' field" "$AUDIT" "logs"
assert_contains "Audit logs has 'total' field" "$AUDIT" "total"
AUDIT_COUNT=$(echo "$AUDIT" | jget total 0)
if [[ "$AUDIT_COUNT" -gt 0 ]]; then
  pass "Audit log has $AUDIT_COUNT entries"
else
  warn "Audit log empty — events may not have been logged"
fi

subhdr "Filter Audit Logs"
AUDIT_FILTERED=$(curl -s --max-time 10 "$API/admin/audit-logs?event_type=login&limit=5" -H "$AUTH")
assert_contains "Filtered audit logs returned" "$AUDIT_FILTERED" "logs"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 19: EXTERNAL DIAGNOSTICS (Basic Auth)
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 19: External Diagnostics"

subhdr "External Health (no auth → 401)"
EXT_HEALTH_STATUS=$(http_status "$API/external/diagnostics/health")
if [[ "$EXT_HEALTH_STATUS" == "401" ]]; then
  pass "External health rejects unauthenticated requests"
else
  warn "External health returned $EXT_HEALTH_STATUS (expected 401)"
fi

subhdr "External Diagnostics Endpoints"
# These require HTTP Basic Auth configured via environment — test auth rejection
for endpoint in "external/diagnostics" "external/diagnostics/health" "external/diagnostics/storage" "external/diagnostics/audit"; do
  STATUS=$(http_status "$API/$endpoint")
  if [[ "$STATUS" == "401" || "$STATUS" == "403" ]]; then
    pass "External /$endpoint rejects unauthenticated (HTTP $STATUS)"
  else
    warn "External /$endpoint returned $STATUS (expected 401/403)"
  fi
done

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 20: DOWNLOADS & MISCELLANEOUS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 20: Downloads & Miscellaneous"

subhdr "Android APK Download"
APK_STATUS=$(http_status "$API/downloads/android")
if [[ "$APK_STATUS" == "404" || "$APK_STATUS" == "200" ]]; then
  pass "Android download endpoint responds (HTTP $APK_STATUS)"
else
  fail "Android download returned unexpected status: $APK_STATUS"
fi

subhdr "Auto-Scan Trigger"
AUTOSCAN=$(curl -s --max-time 10 -X POST "$API/admin/photos/auto-scan" -H "$AUTH")
if echo "$AUTOSCAN" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Auto-scan trigger returns valid JSON"
else
  warn "Auto-scan trigger response: ${AUTOSCAN:0:100}"
fi

subhdr "Import Scan"
IMPORT_SCAN=$(curl -s --max-time 10 "$API/admin/import/scan" -H "$AUTH")
if echo "$IMPORT_SCAN" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Import scan returns valid JSON"
else
  warn "Import scan response: ${IMPORT_SCAN:0:100}"
fi

subhdr "Encrypted Sync Endpoint"
ENC_SYNC=$(curl -s --max-time 10 "$API/photos/encrypted-sync" -H "$AUTH")
if echo "$ENC_SYNC" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Encrypted sync returns valid JSON"
else
  warn "Encrypted sync response: ${ENC_SYNC:0:100}"
fi

subhdr "Reconvert Trigger"
# Reconvert requires key_hex — without encryption enabled, expect a 400 error
RECONVERT_STATUS=$(http_status -X POST "$API/admin/photos/reconvert" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"key_hex":"0000000000000000000000000000000000000000000000000000000000000000"}')
if [[ "$RECONVERT_STATUS" == "200" || "$RECONVERT_STATUS" == "202" || "$RECONVERT_STATUS" == "400" || "$RECONVERT_STATUS" == "404" ]]; then
  pass "Reconvert trigger responds as expected (HTTP $RECONVERT_STATUS)"
else
  fail "Reconvert returned unexpected status: $RECONVERT_STATUS"
fi

subhdr "Setup Discover"
SETUP_DISCOVER=$(curl -s --max-time 15 "$API/setup/discover")
if echo "$SETUP_DISCOVER" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
  pass "Setup discover returns valid JSON"
else
  warn "Setup discover response: ${SETUP_DISCOVER:0:100}"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 21: SECURITY HEADERS VERIFICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 21: Security Headers"

HEADERS=$(curl -sI --max-time 10 "$API/photos" -H "$AUTH")

subhdr "Cache-Control Header"
if echo "$HEADERS" | grep -qi "cache-control"; then
  pass "Cache-Control header present"
  CACHE_VAL=$(echo "$HEADERS" | grep -i "cache-control" | head -1)
  log "  $CACHE_VAL"
else
  fail "Cache-Control header missing"
fi

subhdr "X-Content-Type-Options"
if echo "$HEADERS" | grep -qi "x-content-type-options"; then
  pass "X-Content-Type-Options header present"
else
  fail "X-Content-Type-Options header missing"
fi

subhdr "X-Frame-Options"
if echo "$HEADERS" | grep -qi "x-frame-options"; then
  pass "X-Frame-Options header present"
else
  fail "X-Frame-Options header missing"
fi

subhdr "X-Request-Id"
if echo "$HEADERS" | grep -qi "x-request-id"; then
  pass "X-Request-Id header present"
  REQ_ID=$(echo "$HEADERS" | grep -i "x-request-id" | head -1)
  log "  $REQ_ID"
else
  warn "X-Request-Id header not found"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 22: LOGOUT & CLEANUP
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 22: Logout & Cleanup"

subhdr "Logout User 2"
if [[ -n "$AUTH2" && -n "$REFRESH2" ]]; then
  LOGOUT2_STATUS=$(http_status -X POST "$API/auth/logout" \
    -H "$AUTH2" -H 'Content-Type: application/json' \
    -d "{\"refresh_token\":\"$REFRESH2\"}")
  if [[ "$LOGOUT2_STATUS" == "200" || "$LOGOUT2_STATUS" == "204" ]]; then
    pass "User 2 logged out (HTTP $LOGOUT2_STATUS)"
  else
    fail "User 2 logout returned $LOGOUT2_STATUS"
  fi
fi

subhdr "Delete User 2"
if [[ -n "$USER2_ID" && "$USER2_ID" != "__MISSING__" ]]; then
  DEL_USER_STATUS=$(http_status -X DELETE "$API/admin/users/$USER2_ID" -H "$AUTH")
  if [[ "$DEL_USER_STATUS" == "204" || "$DEL_USER_STATUS" == "200" ]]; then
    pass "User 2 deleted (HTTP $DEL_USER_STATUS)"
  else
    fail "Delete user 2 returned $DEL_USER_STATUS"
  fi
fi

subhdr "Logout Admin"
LOGOUT_STATUS=$(http_status -X POST "$API/auth/logout" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"refresh_token\":\"$REFRESH\"}")
if [[ "$LOGOUT_STATUS" == "200" || "$LOGOUT_STATUS" == "204" ]]; then
  pass "Admin logged out (HTTP $LOGOUT_STATUS)"
else
  fail "Admin logout returned $LOGOUT_STATUS"
fi

subhdr "Verify Token Revoked"
POST_LOGOUT_STATUS=$(http_status "$API/photos" -H "$AUTH")
if [[ "$POST_LOGOUT_STATUS" == "401" ]]; then
  pass "Token correctly rejected after logout"
else
  # JWT tokens may still be valid until expiry — this is expected behavior
  warn "Token still accepted after logout (JWT expiry-based — expected)"
fi

# ══════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ══════════════════════════════════════════════════════════════════════════════
hdr "E2E Test Results Summary"
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
exit "$FAILURES"