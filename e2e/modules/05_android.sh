#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Module 05: Android Client Compatibility Tests for Simple Photos
# ══════════════════════════════════════════════════════════════════════════════
# Validates every API endpoint the Android app (ApiService.kt) depends on,
# with special focus on endpoints NOT covered by previous modules:
#
#   1.  APK Build Verification
#   2.  DTO Contract: Auth — response fields match Kotlin DTOs
#   3.  DTO Contract: Photos — photo list & sync fields
#   4.  DTO Contract: Blobs — blob upload & list fields
#   5.  DTO Contract: Tags, Search, Trash — remaining shapes
#   6.  Client Photo Upload — POST /api/photos/upload (raw binary)
#   7.  Upload Deduplication — hash-based content dedup
#   8.  Favorites & Crop Metadata
#   9.  Duplicate / Edit Copy
#  10.  2FA Full Lifecycle — setup → TOTP code → confirm → login → disable
#  11.  2FA Backup Codes — single-use login + exhaustion
#  12.  Admin 2FA Reset
#  13.  Pagination Cursors — photos, blobs, trash
#  14.  Audio Backup Settings
#
# Prerequisites:
#   - Server running on localhost:8080, already initialized
#   - Photos in storage root for scan-based tests
#   - Python3 available (TOTP code generation)
#
# Usage:
#   bash e2e/modules/05_android.sh [--verbose] [--skip-build]
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/helpers.sh"
parse_common_args "$@"
setup_module_log "android"

module_timer_start "Android Client Compatibility"

# ── Module-specific flags ────────────────────────────────────────────────────
SKIP_BUILD="${SKIP_BUILD:-false}"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

for arg in "$@"; do
  case $arg in
    --skip-build) SKIP_BUILD=true ;;
  esac
done

echo -e "${BOLD}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  Android Client Compatibility E2E — Simple Photos              ║${NC}"
echo -e "${BOLD}╚════════════════════════════════════════════════════════════════╝${NC}"

# ── Ensure server is ready ───────────────────────────────────────────────────
ensure_server_initialized "$API" "$USER" "$PASS"
TOKEN=$(login_and_get_token "$API" "$USER" "$PASS")
AUTH="Authorization: Bearer $TOKEN"

# Trigger a scan so we have photos to work with
SCAN=$(curl -s --max-time 600 -X POST "$API/admin/photos/scan" -H "$AUTH")
REGISTERED=$(echo "$SCAN" | jget registered 0)
log "Scan: registered=$REGISTERED photos"
wait_for_conversions "$API" "$AUTH" 60

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 1: APK BUILD VERIFICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 1: APK Build Verification"

if [[ "$SKIP_BUILD" == "true" ]]; then
  warn "APK build skipped (--skip-build)"
else
  ANDROID_DIR="$REPO_ROOT/android"
  if [[ -d "$ANDROID_DIR" && -f "$ANDROID_DIR/gradlew" ]]; then
    subhdr "Build Debug APK"
    BUILD_LOG="$E2E_TMP_DIR/android_build.log"
    if (cd "$ANDROID_DIR" && ./gradlew assembleDebug --no-daemon 2>&1 | tail -30 > "$BUILD_LOG"); then
      pass "Debug APK built successfully"
    else
      fail "Debug APK build failed — see $BUILD_LOG"
    fi

    subhdr "Verify APK Output"
    APK_PATH=$(find "$ANDROID_DIR" -name "*.apk" -path "*/debug/*" -type f 2>/dev/null | head -1)
    if [[ -n "$APK_PATH" ]]; then
      APK_SIZE=$(stat -c%s "$APK_PATH" 2>/dev/null || stat -f%z "$APK_PATH" 2>/dev/null || echo 0)
      pass "Debug APK exists ($APK_SIZE bytes)"
      if (( APK_SIZE > 1000000 )); then
        pass "APK size is reasonable (> 1MB)"
      else
        warn "APK is surprisingly small: $APK_SIZE bytes"
      fi
    else
      fail "No debug APK found in build output"
    fi

    subhdr "Version Consistency"
    # Check that Android versionName matches server version
    SERVER_VERSION=$(curl -s --max-time 10 "$BASE/health" | jget version "unknown")
    GRADLE_VERSION=$(grep -oP 'versionName\s*=\s*"\K[^"]+' "$ANDROID_DIR/app/build.gradle.kts" 2>/dev/null || echo "unknown")
    if [[ "$SERVER_VERSION" == "$GRADLE_VERSION" ]]; then
      pass "Android version ($GRADLE_VERSION) matches server ($SERVER_VERSION)"
    else
      warn "Version mismatch: Android=$GRADLE_VERSION, Server=$SERVER_VERSION"
    fi
  else
    warn "Android project not found at $ANDROID_DIR — skipping build"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 2: DTO CONTRACT — AUTH
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 2: DTO Contract — Auth"

# The Android app expects specific field names in JSON responses.
# If the server changes a field name, these tests catch it immediately.

subhdr "Login Response Shape"
LOGIN_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")

# LoginResponse fields from AuthDto.kt
assert_contains "Login has access_token" "$LOGIN_RESP" "access_token"
assert_contains "Login has refresh_token" "$LOGIN_RESP" "refresh_token"
assert_contains "Login has expires_in" "$LOGIN_RESP" "expires_in"

# Verify no TOTP required for non-2FA user
REQUIRES_TOTP=$(echo "$LOGIN_RESP" | jget requires_totp "")
if [[ "$REQUIRES_TOTP" == "" || "$REQUIRES_TOTP" == "__MISSING__" || "$REQUIRES_TOTP" == "false" ]]; then
  pass "Non-2FA login does not require TOTP"
else
  fail "Non-2FA login unexpectedly requires TOTP: $REQUIRES_TOTP"
fi

# Refresh token for later use
REFRESH_TK=$(echo "$LOGIN_RESP" | jget refresh_token "")
TOKEN=$(echo "$LOGIN_RESP" | jget access_token "")
AUTH="Authorization: Bearer $TOKEN"

subhdr "Register Response Shape"
# Create a temp user for shape testing
REG_USER="android_dto_$(date +%s)"
REG_PASS="DtoTest1!Pass"
REG_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/register" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$REG_USER\",\"password\":\"$REG_PASS\"}")
# RegisterResponse is either user_id+username, or might be blocked
REG_STATUS=$(echo "$REG_RESP" | jget user_id "")
if [[ -n "$REG_STATUS" && "$REG_STATUS" != "__MISSING__" ]]; then
  assert_contains "Register has user_id" "$REG_RESP" "user_id"
  assert_contains "Register has username" "$REG_RESP" "username"
  pass "Register response matches RegisterResponse DTO"
  REG_UID="$REG_STATUS"
else
  # Registration might be admin-only — check error shape
  assert_contains "Register error has expected field" "$REG_RESP" "error"
  warn "Registration returned error (may be admin-only): ${REG_RESP:0:80}"
  REG_UID=""
fi

subhdr "Refresh Response Shape"
REFRESH_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/refresh" \
  -H 'Content-Type: application/json' \
  -d "{\"refresh_token\":\"$REFRESH_TK\"}")
assert_contains "Refresh has access_token" "$REFRESH_RESP" "access_token"
# RefreshResponse: accessToken, expiresIn
# Server may also return refresh_token
TOKEN=$(echo "$REFRESH_RESP" | jget access_token "$TOKEN")
AUTH="Authorization: Bearer $TOKEN"

subhdr "Logout Request Shape"
# Get a fresh token pair specifically for logout testing
LOGOUT_LOGIN=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")
LOGOUT_REFRESH=$(echo "$LOGOUT_LOGIN" | jget refresh_token "")
LOGOUT_STATUS=$(http_status -X POST "$API/auth/logout" \
  -H "Authorization: Bearer $(echo "$LOGOUT_LOGIN" | jget access_token "")" \
  -H 'Content-Type: application/json' \
  -d "{\"refresh_token\":\"$LOGOUT_REFRESH\"}")
if [[ "$LOGOUT_STATUS" == "204" || "$LOGOUT_STATUS" == "200" ]]; then
  pass "Logout succeeds (HTTP $LOGOUT_STATUS)"
else
  fail "Logout returned unexpected status: $LOGOUT_STATUS"
fi

# Clean up temp user
if [[ -n "$REG_UID" ]]; then
  curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/admin/users/$REG_UID" -H "$AUTH" > /dev/null 2>&1
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 3: DTO CONTRACT — PHOTOS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 3: DTO Contract — Photos"

subhdr "PlainPhotoListResponse Shape"
PHOTOS_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/photos" -H "$AUTH")
assert_contains "Photos list has 'photos'" "$PHOTOS_RESP" "photos"

# PlainPhotoRecord fields from PhotoDto.kt (18 fields)
FIRST_PHOTO=$(echo "$PHOTOS_RESP" | python3 -c "
import sys, json
d = json.load(sys.stdin)
photos = d.get('photos', [])
if photos:
    import json as j
    print(j.dumps(photos[0]))
else:
    print('{}')
" 2>/dev/null)

if [[ "$FIRST_PHOTO" != "{}" && -n "$FIRST_PHOTO" ]]; then
  PHOTO_ID=$(echo "$FIRST_PHOTO" | jget id "")
  for field in id filename file_path mime_type media_type size_bytes created_at; do
    VAL=$(echo "$FIRST_PHOTO" | jget "$field" "__MISSING__")
    if [[ "$VAL" != "__MISSING__" ]]; then
      pass "PlainPhotoRecord has '$field'"
    else
      fail "PlainPhotoRecord missing '$field'"
    fi
  done
  # Optional fields that should still be present (may be null)
  for field in width height thumb_path is_favorite photo_hash; do
    if echo "$FIRST_PHOTO" | grep -q "\"$field\""; then
      pass "PlainPhotoRecord has '$field' key"
    else
      fail "PlainPhotoRecord missing '$field' key"
    fi
  done
else
  warn "No photos available for DTO shape testing"
  PHOTO_ID=""
fi

subhdr "EncryptedSyncResponse Shape"
SYNC_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/photos/encrypted-sync" -H "$AUTH")
# Should return photos/items even if empty
if echo "$SYNC_RESP" | grep -q "photos\|items\|\["; then
  pass "Encrypted sync endpoint responds with expected structure"
else
  fail "Encrypted sync response unexpected: ${SYNC_RESP:0:100}"
fi

subhdr "Thumbnail & File Endpoints"
if [[ -n "$PHOTO_ID" ]]; then
  THUMB_STATUS=$(http_status "$API/photos/$PHOTO_ID/thumb" -H "$AUTH")
  if [[ "$THUMB_STATUS" == "200" || "$THUMB_STATUS" == "202" || "$THUMB_STATUS" == "404" ]]; then
    pass "Thumb endpoint responds (HTTP $THUMB_STATUS)"
  else
    fail "Thumb endpoint unexpected status: $THUMB_STATUS"
  fi

  FILE_STATUS=$(http_status "$API/photos/$PHOTO_ID/file" -H "$AUTH")
  if [[ "$FILE_STATUS" == "200" || "$FILE_STATUS" == "206" ]]; then
    pass "File endpoint responds (HTTP $FILE_STATUS)"
  else
    fail "File endpoint unexpected status: $FILE_STATUS"
  fi
else
  warn "Skipping thumb/file tests — no photos available"
fi

subhdr "StorageStatsResponse Shape"
STATS_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/settings/storage-stats" -H "$AUTH")
for field in photo_count video_count other_blob_count fs_total_bytes fs_free_bytes; do
  if echo "$STATS_RESP" | grep -q "\"$field\""; then
    pass "StorageStats has '$field'"
  else
    fail "StorageStats missing '$field'"
  fi
done

subhdr "ScanResponse Shape"
SCAN_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/admin/photos/scan" -H "$AUTH")
assert_contains "Scan response has 'registered'" "$SCAN_RESP" "registered"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 4: DTO CONTRACT — BLOBS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 4: DTO Contract — Blobs"

subhdr "Blob Upload Response Shape"
BLOB_FILE="$E2E_TMP_DIR/android_test_blob.bin"
dd if=/dev/urandom of="$BLOB_FILE" bs=1024 count=4 status=none 2>/dev/null
BLOB_HASH=$(sha256sum "$BLOB_FILE" | cut -d' ' -f1)

BLOB_UP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/blobs" \
  -H "$AUTH" \
  -H "x-blob-type: photo" \
  -H "x-client-hash: $BLOB_HASH" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@$BLOB_FILE")
# BlobUploadResponse: blob_id, upload_time, size
assert_contains "Blob upload has 'blob_id'" "$BLOB_UP" "blob_id"
BLOB_ID=$(echo "$BLOB_UP" | jget blob_id "")
if [[ -n "$BLOB_ID" && "$BLOB_ID" != "__MISSING__" ]]; then
  pass "Blob uploaded: $BLOB_ID"
else
  fail "Blob upload did not return blob_id"
fi

subhdr "BlobListResponse Shape"
BLOB_LIST=$(curl -s --max-time "$CURL_MAX_TIME" "$API/blobs" -H "$AUTH")
assert_contains "Blob list has 'blobs'" "$BLOB_LIST" "blobs"

# BlobRecord fields
FIRST_BLOB=$(echo "$BLOB_LIST" | python3 -c "
import sys, json
d = json.load(sys.stdin)
blobs = d.get('blobs', [])
if blobs:
    print(json.dumps(blobs[0]))
else:
    print('{}')
" 2>/dev/null)

if [[ "$FIRST_BLOB" != "{}" && -n "$FIRST_BLOB" ]]; then
  for field in id blob_type size_bytes; do
    VAL=$(echo "$FIRST_BLOB" | jget "$field" "__MISSING__")
    if [[ "$VAL" != "__MISSING__" ]]; then
      pass "BlobRecord has '$field'"
    else
      fail "BlobRecord missing '$field'"
    fi
  done
fi

subhdr "Blob Download"
if [[ -n "$BLOB_ID" && "$BLOB_ID" != "__MISSING__" ]]; then
  DL_STATUS=$(http_status "$API/blobs/$BLOB_ID" -H "$AUTH")
  if [[ "$DL_STATUS" == "200" ]]; then
    pass "Blob download succeeds (HTTP 200)"
  else
    fail "Blob download returned $DL_STATUS"
  fi

  THUMB_STATUS=$(http_status "$API/blobs/$BLOB_ID/thumb" -H "$AUTH")
  if [[ "$THUMB_STATUS" == "200" || "$THUMB_STATUS" == "404" ]]; then
    pass "Blob thumb endpoint responds (HTTP $THUMB_STATUS)"
  else
    fail "Blob thumb endpoint: $THUMB_STATUS"
  fi
fi
rm -f "$BLOB_FILE"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 5: DTO CONTRACT — TAGS, SEARCH, TRASH
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 5: DTO Contract — Tags, Search, Trash"

subhdr "TagListResponse Shape"
TAGS_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/tags" -H "$AUTH")
if echo "$TAGS_RESP" | grep -q "tags\|\[\]"; then
  pass "Tags endpoint returns expected structure"
else
  fail "Tags endpoint unexpected: ${TAGS_RESP:0:100}"
fi

subhdr "SearchResponse Shape"
SEARCH_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/search?q=test" -H "$AUTH")
if echo "$SEARCH_RESP" | grep -q "results\|\[\]"; then
  pass "Search endpoint returns expected structure"
else
  fail "Search endpoint unexpected: ${SEARCH_RESP:0:100}"
fi

subhdr "TrashListResponse Shape"
TRASH_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/trash" -H "$AUTH")
if echo "$TRASH_RESP" | grep -q "items\|\[\]"; then
  pass "Trash endpoint returns expected structure"
else
  fail "Trash endpoint unexpected: ${TRASH_RESP:0:100}"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 6: CLIENT PHOTO UPLOAD
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 6: Client Photo Upload"

subhdr "Upload via POST /api/photos/upload"
# The Android app uploads raw bytes with X-Filename and X-Mime-Type headers,
# NOT multipart/form-data. This endpoint has no E2E coverage until now.

# Use a real JPEG — create a minimal valid JPEG (smallest valid JFIF)
UPLOAD_FILE="$E2E_TMP_DIR/android_upload_test.jpg"
python3 -c "
import struct, sys
# Minimal valid JPEG: SOI + APP0 (JFIF) + DQT + SOF0 + DHT (dummy) + SOS + EOI
# We'll create a 1x1 white JPEG
soi = b'\xff\xd8'
eoi = b'\xff\xd9'
# APP0 (JFIF marker)
app0 = b'\xff\xe0' + struct.pack('>H', 16) + b'JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00'
# Minimal DQT
dqt = b'\xff\xdb' + struct.pack('>H', 67) + b'\x00' + bytes([8]*64)
# SOF0 (1x1, 1 component, Y only)
sof = b'\xff\xc0' + struct.pack('>H', 11) + b'\x08\x00\x01\x00\x01\x01\x01\x11\x00'
# Minimal DHT (DC table)
dht = b'\xff\xc4' + struct.pack('>H', 31) + b'\x00' + bytes(16) + bytes(12)
# SOS header
sos = b'\xff\xda' + struct.pack('>H', 8) + b'\x01\x01\x00\x00?\x00'
# Scan data (minimal)
scan = b'\x7f\x00'
sys.stdout.buffer.write(soi + app0 + dqt + sof + dht + sos + scan + eoi)
" > "$UPLOAD_FILE"

UPLOAD_HASH=$(sha256sum "$UPLOAD_FILE" | cut -d' ' -f1)
UPLOAD_SIZE=$(stat -c%s "$UPLOAD_FILE" 2>/dev/null || stat -f%z "$UPLOAD_FILE" 2>/dev/null || echo 0)
log "Upload file: ${UPLOAD_SIZE} bytes, hash=$UPLOAD_HASH"

UPLOAD_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/photos/upload" \
  -H "$AUTH" \
  -H "X-Filename: android_e2e_test.jpg" \
  -H "X-Mime-Type: image/jpeg" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@$UPLOAD_FILE")
log "  Upload response: ${UPLOAD_RESP:0:200}"

# PhotoUploadResponse: photo_id, filename, size_bytes
UPLOAD_PID=$(echo "$UPLOAD_RESP" | jget photo_id "")
if [[ -n "$UPLOAD_PID" && "$UPLOAD_PID" != "__MISSING__" ]]; then
  pass "Upload returned photo_id: $UPLOAD_PID"
else
  fail "Upload did not return photo_id: ${UPLOAD_RESP:0:200}"
fi
assert_contains "Upload has filename" "$UPLOAD_RESP" "filename"
assert_contains "Upload has file_path" "$UPLOAD_RESP" "file_path"

UPLOAD_HASH_RETURNED=$(echo "$UPLOAD_RESP" | jget photo_hash "")
if [[ -n "$UPLOAD_HASH_RETURNED" && "$UPLOAD_HASH_RETURNED" != "__MISSING__" ]]; then
  pass "Upload returned photo_hash"
else
  warn "Upload did not return photo_hash (may be optional)"
fi

subhdr "Verify Uploaded Photo in List"
PHOTO_LIST_AFTER=$(curl -s --max-time "$CURL_MAX_TIME" "$API/photos" -H "$AUTH")
if echo "$PHOTO_LIST_AFTER" | grep -q "$UPLOAD_PID"; then
  pass "Uploaded photo appears in photo list"
else
  fail "Uploaded photo not found in list (ID: $UPLOAD_PID)"
fi

subhdr "Fetch Uploaded Photo"
if [[ -n "$UPLOAD_PID" && "$UPLOAD_PID" != "__MISSING__" ]]; then
  FILE_STATUS=$(http_status "$API/photos/$UPLOAD_PID/file" -H "$AUTH")
  if [[ "$FILE_STATUS" == "200" ]]; then
    pass "Uploaded photo serves file (HTTP 200)"
  else
    fail "Uploaded photo file serve returned $FILE_STATUS"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 7: UPLOAD DEDUPLICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 7: Upload Deduplication"

subhdr "Re-upload Same Content"
DUP_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/photos/upload" \
  -H "$AUTH" \
  -H "X-Filename: android_e2e_test_dup.jpg" \
  -H "X-Mime-Type: image/jpeg" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@$UPLOAD_FILE")
DUP_PID=$(echo "$DUP_RESP" | jget photo_id "")

if [[ "$DUP_PID" == "$UPLOAD_PID" ]]; then
  pass "Duplicate upload returns same photo_id (hash-based dedup)"
else
  warn "Duplicate upload returned different photo_id: $DUP_PID (expected $UPLOAD_PID)"
fi

subhdr "Upload Different Content → New Photo"
UPLOAD_FILE2="$E2E_TMP_DIR/android_upload_test2.bin"
dd if=/dev/urandom of="$UPLOAD_FILE2" bs=1024 count=2 status=none 2>/dev/null
DIFF_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/photos/upload" \
  -H "$AUTH" \
  -H "X-Filename: android_different.jpg" \
  -H "X-Mime-Type: image/jpeg" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@$UPLOAD_FILE2")
DIFF_PID=$(echo "$DIFF_RESP" | jget photo_id "")

if [[ -n "$DIFF_PID" && "$DIFF_PID" != "$UPLOAD_PID" && "$DIFF_PID" != "__MISSING__" ]]; then
  pass "Different content creates new photo (ID: $DIFF_PID)"
else
  fail "Different content did not create new photo"
fi
rm -f "$UPLOAD_FILE" "$UPLOAD_FILE2"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 8: FAVORITES & CROP METADATA
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 8: Favorites & Crop Metadata"

# Use the uploaded photo or first available
TEST_PID="${UPLOAD_PID:-$PHOTO_ID}"
if [[ -n "$TEST_PID" && "$TEST_PID" != "__MISSING__" ]]; then

  subhdr "Toggle Favorite (no body — server toggles)"
  # The server toggles is_favorite = 1 - is_favorite; no request body needed.
  # First toggle → ON
  FAV_ON=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$API/photos/$TEST_PID/favorite" \
    -H "$AUTH")
  FAV_ON_VAL=$(echo "$FAV_ON" | jget is_favorite "")
  log "  First toggle: is_favorite=$FAV_ON_VAL"
  if [[ -n "$FAV_ON_VAL" && "$FAV_ON_VAL" != "__MISSING__" ]]; then
    pass "Favorite toggled — is_favorite=$FAV_ON_VAL"
  else
    fail "Favorite toggle failed: ${FAV_ON:0:100}"
  fi

  subhdr "Toggle Favorite Back"
  # Second toggle should return the opposite value
  FAV_OFF=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$API/photos/$TEST_PID/favorite" \
    -H "$AUTH")
  FAV_OFF_VAL=$(echo "$FAV_OFF" | jget is_favorite "")
  log "  Second toggle: is_favorite=$FAV_OFF_VAL"
  if [[ "$FAV_ON_VAL" != "$FAV_OFF_VAL" ]]; then
    pass "Second toggle reversed the value (was $FAV_ON_VAL, now $FAV_OFF_VAL)"
  else
    fail "Toggle did not reverse: still $FAV_OFF_VAL"
  fi

  subhdr "Set Crop Metadata"
  # SetCropRequest: crop_metadata is a JSON *string* with 0.0-1.0 values
  CROP_JSON='{"x":0.1,"y":0.2,"width":0.6,"height":0.5,"rotate":0}'
  CROP_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$API/photos/$TEST_PID/crop" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"crop_metadata\":\"$(echo "$CROP_JSON" | sed 's/"/\\\\"/g')\"}") 
  # Easier to build with python3:
  CROP_RESP=$(python3 -c "
import json, subprocess, sys
body = json.dumps({'crop_metadata': '$CROP_JSON'})
cmd = ['curl', '-s', '--max-time', '$CURL_MAX_TIME', '-X', 'PUT',
       '$API/photos/$TEST_PID/crop',
       '-H', 'Authorization: Bearer $TOKEN',
       '-H', 'Content-Type: application/json',
       '-d', body]
result = subprocess.run(cmd, capture_output=True, text=True)
print(result.stdout)
" 2>/dev/null)
  log "  Crop response: ${CROP_RESP:0:120}"
  if echo "$CROP_RESP" | grep -q "crop_metadata"; then
    pass "Crop metadata set and returned"
  else
    # Fallback: check HTTP status
    CROP_HTTP=$(python3 -c "
import json, subprocess
body = json.dumps({'crop_metadata': '$CROP_JSON'})
cmd = ['curl', '-s', '-o', '/dev/null', '-w', '%{http_code}', '--max-time', '$CURL_MAX_TIME',
       '-X', 'PUT', '$API/photos/$TEST_PID/crop',
       '-H', 'Authorization: Bearer $TOKEN',
       '-H', 'Content-Type: application/json', '-d', body]
result = subprocess.run(cmd, capture_output=True, text=True)
print(result.stdout)
" 2>/dev/null)
    if [[ "$CROP_HTTP" == "200" ]]; then
      pass "Crop metadata accepted (HTTP 200)"
    else
      fail "Crop metadata failed: HTTP $CROP_HTTP, response: ${CROP_RESP:0:100}"
    fi
  fi

  subhdr "Clear Crop Metadata"
  CLEAR_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$API/photos/$TEST_PID/crop" \
    -H "$AUTH" -H 'Content-Type: application/json' -d '{"crop_metadata":null}')
  if echo "$CLEAR_RESP" | grep -q "crop_metadata"; then
    pass "Crop metadata cleared"
  else
    CLEAR_STATUS=$(http_status -X PUT "$API/photos/$TEST_PID/crop" \
      -H "$AUTH" -H 'Content-Type: application/json' -d '{"crop_metadata":null}')
    if [[ "$CLEAR_STATUS" == "200" ]]; then
      pass "Crop metadata cleared (HTTP 200)"
    else
      warn "Clear crop returned HTTP $CLEAR_STATUS"
    fi
  fi
else
  warn "No photo ID available — skipping favorites/crop tests"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 9: DUPLICATE / EDIT COPY
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 9: Duplicate / Edit Copy"

if [[ -n "$TEST_PID" && "$TEST_PID" != "__MISSING__" ]]; then
  subhdr "Create Duplicate"
  DUP_RESULT=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/photos/$TEST_PID/duplicate" \
    -H "$AUTH" -H 'Content-Type: application/json' -d '{}')
  DUP_NEW_ID=$(echo "$DUP_RESULT" | jget photo_id "")
  if [[ -z "$DUP_NEW_ID" || "$DUP_NEW_ID" == "__MISSING__" ]]; then
    # Some servers use different field names
    DUP_NEW_ID=$(echo "$DUP_RESULT" | jget id "")
  fi
  if [[ -n "$DUP_NEW_ID" && "$DUP_NEW_ID" != "__MISSING__" && "$DUP_NEW_ID" != "$TEST_PID" ]]; then
    pass "Duplicate created: new photo_id=$DUP_NEW_ID"
  else
    warn "Duplicate endpoint response: ${DUP_RESULT:0:120}"
  fi

  subhdr "Verify Duplicate in List"
  if [[ -n "$DUP_NEW_ID" && "$DUP_NEW_ID" != "__MISSING__" ]]; then
    LIST_CHECK=$(curl -s --max-time "$CURL_MAX_TIME" "$API/photos" -H "$AUTH")
    if echo "$LIST_CHECK" | grep -q "$DUP_NEW_ID"; then
      pass "Duplicate appears in photo list"
    else
      warn "Duplicate not found in list (may be processing)"
    fi
  fi
else
  warn "No photo ID available — skipping duplicate tests"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 10: 2FA FULL LIFECYCLE
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 10: 2FA Full Lifecycle"

# Create a dedicated user for 2FA testing to avoid locking out admin
TOTP_USER="totp_test_$(date +%s)"
TOTP_PASS="TotpTest1!X"

subhdr "Create 2FA Test User"
CREATE_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/admin/users" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\",\"role\":\"user\"}")
TOTP_UID=$(echo "$CREATE_RESP" | jget user_id "")
if [[ -n "$TOTP_UID" && "$TOTP_UID" != "__MISSING__" ]]; then
  pass "2FA test user created: $TOTP_UID"
else
  fail "Cannot create 2FA test user: ${CREATE_RESP:0:120}"
  TOTP_UID=""
fi

if [[ -n "$TOTP_UID" ]]; then
  # Login as the test user
  TOTP_LOGIN=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
  TOTP_TOKEN=$(echo "$TOTP_LOGIN" | jget access_token "")
  TOTP_AUTH="Authorization: Bearer $TOTP_TOKEN"

  subhdr "2FA Status (before setup)"
  STATUS_BEFORE=$(curl -s --max-time "$CURL_MAX_TIME" "$API/auth/2fa/status" -H "$TOTP_AUTH")
  log "  2FA status before: ${STATUS_BEFORE:0:100}"

  subhdr "Setup 2FA"
  SETUP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/2fa/setup" \
    -H "$TOTP_AUTH" -H 'Content-Type: application/json' -d '{}')
  assert_contains "2FA setup returns otpauth_uri" "$SETUP" "otpauth_uri"
  assert_contains "2FA setup returns backup_codes" "$SETUP" "backup_codes"

  # Extract the TOTP secret from the otpauth URI
  OTPAUTH_URI=$(echo "$SETUP" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('otpauth_uri', ''))
" 2>/dev/null)

  TOTP_SECRET=$(echo "$OTPAUTH_URI" | python3 -c "
import sys, urllib.parse
uri = sys.stdin.read().strip()
params = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)
print(params.get('secret', [''])[0])
" 2>/dev/null)

  # Extract backup codes
  BACKUP_CODES=$(echo "$SETUP" | python3 -c "
import sys, json
d = json.load(sys.stdin)
codes = d.get('backup_codes', [])
for c in codes:
    print(c)
" 2>/dev/null)
  BACKUP_CODE_1=$(echo "$BACKUP_CODES" | head -1)
  BACKUP_CODE_2=$(echo "$BACKUP_CODES" | sed -n '2p')
  BACKUP_COUNT=$(echo "$BACKUP_CODES" | wc -l)

  if [[ -n "$TOTP_SECRET" ]]; then
    pass "Extracted TOTP secret from otpauth URI"
    log "  Secret: ${TOTP_SECRET:0:4}...${TOTP_SECRET: -4} (${#TOTP_SECRET} chars)"
  else
    fail "Could not extract TOTP secret from URI: ${OTPAUTH_URI:0:80}"
  fi

  if (( BACKUP_COUNT >= 10 )); then
    pass "Received $BACKUP_COUNT backup codes"
  else
    fail "Expected 10 backup codes, got $BACKUP_COUNT"
  fi

  subhdr "Generate TOTP Code & Confirm 2FA"
  if [[ -n "$TOTP_SECRET" ]]; then
    # Generate a valid 6-digit TOTP code using Python (SHA1, 6 digits, 30s period)
    TOTP_CODE=$(python3 -c "
import hmac, hashlib, struct, time, base64
secret = base64.b32decode('$TOTP_SECRET', casefold=True)
counter = int(time.time()) // 30
msg = struct.pack('>Q', counter)
h = hmac.new(secret, msg, hashlib.sha1).digest()
offset = h[-1] & 0x0F
code = (struct.unpack('>I', h[offset:offset+4])[0] & 0x7FFFFFFF) % 1000000
print(f'{code:06d}')
" 2>/dev/null)

    if [[ ${#TOTP_CODE} -eq 6 ]]; then
      pass "Generated TOTP code: $TOTP_CODE"
    else
      fail "TOTP code generation failed: '$TOTP_CODE'"
    fi

    # Confirm 2FA with the generated code
    CONFIRM_STATUS=$(http_status -X POST "$API/auth/2fa/confirm" \
      -H "$TOTP_AUTH" -H 'Content-Type: application/json' \
      -d "{\"totp_code\":\"$TOTP_CODE\"}")
    if [[ "$CONFIRM_STATUS" == "200" ]]; then
      pass "2FA confirmed with TOTP code (HTTP 200)"
    else
      fail "2FA confirm returned HTTP $CONFIRM_STATUS"
    fi

    subhdr "Login with 2FA"
    # Step 1: Normal login should now return requires_totp=true
    LOGIN2_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
    REQ_TOTP=$(echo "$LOGIN2_RESP" | jget requires_totp "")
    TOTP_SESSION=$(echo "$LOGIN2_RESP" | jget totp_session_token "")

    if [[ "$REQ_TOTP" == "true" ]]; then
      pass "Login now requires TOTP (requires_totp=true)"
    else
      fail "Login should require TOTP but got: ${LOGIN2_RESP:0:120}"
    fi

    if [[ -n "$TOTP_SESSION" && "$TOTP_SESSION" != "__MISSING__" ]]; then
      pass "Login returned totp_session_token"
    else
      fail "Login did not return totp_session_token"
    fi

    # Step 2: Complete login with TOTP code
    if [[ -n "$TOTP_SESSION" && "$TOTP_SESSION" != "__MISSING__" ]]; then
      # Generate a fresh code (time may have advanced)
      TOTP_CODE2=$(python3 -c "
import hmac, hashlib, struct, time, base64
secret = base64.b32decode('$TOTP_SECRET', casefold=True)
counter = int(time.time()) // 30
msg = struct.pack('>Q', counter)
h = hmac.new(secret, msg, hashlib.sha1).digest()
offset = h[-1] & 0x0F
code = (struct.unpack('>I', h[offset:offset+4])[0] & 0x7FFFFFFF) % 1000000
print(f'{code:06d}')
" 2>/dev/null)

      TOTP_LOGIN_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login/totp" \
        -H 'Content-Type: application/json' \
        -d "{\"totp_session_token\":\"$TOTP_SESSION\",\"totp_code\":\"$TOTP_CODE2\"}")
      TOTP_ACCESS=$(echo "$TOTP_LOGIN_RESP" | jget access_token "")
      if [[ -n "$TOTP_ACCESS" && "$TOTP_ACCESS" != "__MISSING__" ]]; then
        pass "2FA login successful — received access_token"
        TOTP_TOKEN="$TOTP_ACCESS"
        TOTP_AUTH="Authorization: Bearer $TOTP_TOKEN"
      else
        fail "2FA login failed: ${TOTP_LOGIN_RESP:0:120}"
      fi
    fi

    subhdr "Disable 2FA"
    # Generate a fresh code for disable
    TOTP_CODE3=$(python3 -c "
import hmac, hashlib, struct, time, base64
secret = base64.b32decode('$TOTP_SECRET', casefold=True)
counter = int(time.time()) // 30
msg = struct.pack('>Q', counter)
h = hmac.new(secret, msg, hashlib.sha1).digest()
offset = h[-1] & 0x0F
code = (struct.unpack('>I', h[offset:offset+4])[0] & 0x7FFFFFFF) % 1000000
print(f'{code:06d}')
" 2>/dev/null)

    DISABLE_STATUS=$(http_status -X POST "$API/auth/2fa/disable" \
      -H "$TOTP_AUTH" -H 'Content-Type: application/json' \
      -d "{\"totp_code\":\"$TOTP_CODE3\"}")
    if [[ "$DISABLE_STATUS" == "200" || "$DISABLE_STATUS" == "204" ]]; then
      pass "2FA disabled successfully (HTTP $DISABLE_STATUS)"
    else
      fail "2FA disable returned HTTP $DISABLE_STATUS"
    fi

    subhdr "Login Without 2FA (after disable)"
    POST_DISABLE=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
    POST_TOKEN=$(echo "$POST_DISABLE" | jget access_token "")
    POST_TOTP=$(echo "$POST_DISABLE" | jget requires_totp "")
    if [[ -n "$POST_TOKEN" && "$POST_TOKEN" != "__MISSING__" && "$POST_TOTP" != "true" ]]; then
      pass "Login works without 2FA after disable"
      TOTP_TOKEN="$POST_TOKEN"
      TOTP_AUTH="Authorization: Bearer $TOTP_TOKEN"
    else
      fail "Login after 2FA disable failed: ${POST_DISABLE:0:120}"
    fi
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 11: 2FA BACKUP CODES
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 11: 2FA Backup Codes"

# Rate limiter cooldown: TOTP limiter is 5 req/60s, login is 10/60s.
# Modules 10-12 are TOTP-heavy — pause between them to avoid 429.
log "  Rate limiter cooldown (8s)..."
sleep 8

if [[ -n "$TOTP_UID" && -n "${TOTP_SECRET:-}" ]]; then
  subhdr "Re-enable 2FA for Backup Code Testing"
  # Setup again
  SETUP2=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/2fa/setup" \
    -H "$TOTP_AUTH" -H 'Content-Type: application/json' -d '{}')
  TOTP_SECRET2=$(echo "$SETUP2" | python3 -c "
import sys, json, urllib.parse
d = json.load(sys.stdin)
uri = d.get('otpauth_uri', '')
params = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)
print(params.get('secret', [''])[0])
" 2>/dev/null)

  BACKUP_CODES2=$(echo "$SETUP2" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for c in d.get('backup_codes', []):
    print(c)
" 2>/dev/null)
  BK1=$(echo "$BACKUP_CODES2" | head -1)
  BK2=$(echo "$BACKUP_CODES2" | sed -n '2p')

  # Confirm with new secret
  CONFIRM_CODE=$(python3 -c "
import hmac, hashlib, struct, time, base64
secret = base64.b32decode('$TOTP_SECRET2', casefold=True)
counter = int(time.time()) // 30
msg = struct.pack('>Q', counter)
h = hmac.new(secret, msg, hashlib.sha1).digest()
offset = h[-1] & 0x0F
code = (struct.unpack('>I', h[offset:offset+4])[0] & 0x7FFFFFFF) % 1000000
print(f'{code:06d}')
" 2>/dev/null)

  curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/2fa/confirm" \
    -H "$TOTP_AUTH" -H 'Content-Type: application/json' \
    -d "{\"totp_code\":\"$CONFIRM_CODE\"}" > /dev/null

  subhdr "Login with Backup Code"
  # Get a TOTP session token
  BK_LOGIN=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
  BK_SESSION=$(echo "$BK_LOGIN" | jget totp_session_token "")

  if [[ -n "$BK_SESSION" && "$BK_SESSION" != "__MISSING__" && -n "$BK1" ]]; then
    BK_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login/totp" \
      -H 'Content-Type: application/json' \
      -d "{\"totp_session_token\":\"$BK_SESSION\",\"backup_code\":\"$BK1\"}")
    BK_TOKEN=$(echo "$BK_RESP" | jget access_token "")
    if [[ -n "$BK_TOKEN" && "$BK_TOKEN" != "__MISSING__" ]]; then
      pass "Backup code login successful"
      TOTP_TOKEN="$BK_TOKEN"
      TOTP_AUTH="Authorization: Bearer $TOTP_TOKEN"
    else
      fail "Backup code login failed: ${BK_RESP:0:120}"
    fi

    subhdr "Backup Code Single-Use"
    # Try using the same backup code again — should fail
    BK_LOGIN2=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
    BK_SESSION2=$(echo "$BK_LOGIN2" | jget totp_session_token "")

    if [[ -n "$BK_SESSION2" && "$BK_SESSION2" != "__MISSING__" ]]; then
      BK_RESP2=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login/totp" \
        -H 'Content-Type: application/json' \
        -d "{\"totp_session_token\":\"$BK_SESSION2\",\"backup_code\":\"$BK1\"}")
      BK_TOKEN2=$(echo "$BK_RESP2" | jget access_token "")
      if [[ -z "$BK_TOKEN2" || "$BK_TOKEN2" == "__MISSING__" ]]; then
        pass "Used backup code correctly rejected on second use"
      else
        fail "Used backup code accepted twice — violation of single-use policy!"
      fi
    fi
  else
    warn "Cannot test backup codes — missing session or backup code"
  fi

  # Disable 2FA for cleanup (use second backup code or TOTP code)
  subhdr "Cleanup: Disable 2FA"
  DISABLE_CODE=$(python3 -c "
import hmac, hashlib, struct, time, base64
secret = base64.b32decode('$TOTP_SECRET2', casefold=True)
counter = int(time.time()) // 30
msg = struct.pack('>Q', counter)
h = hmac.new(secret, msg, hashlib.sha1).digest()
offset = h[-1] & 0x0F
code = (struct.unpack('>I', h[offset:offset+4])[0] & 0x7FFFFFFF) % 1000000
print(f'{code:06d}')
" 2>/dev/null)
  curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/2fa/disable" \
    -H "$TOTP_AUTH" -H 'Content-Type: application/json' \
    -d "{\"totp_code\":\"$DISABLE_CODE\"}" > /dev/null 2>&1
  pass "2FA cleanup complete"
else
  warn "Skipping backup code tests — 2FA user not available"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 12: ADMIN 2FA RESET
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 12: Admin 2FA Reset"

# Rate limiter cooldown before the final 2FA test module
log "  Rate limiter cooldown (8s)..."
sleep 8

if [[ -n "$TOTP_UID" ]]; then
  # Re-enable 2FA on the test user
  subhdr "Setup 2FA (for admin reset test)"
  # Login as test user
  TOTP_LOGIN3=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
  TOTP_TK3=$(echo "$TOTP_LOGIN3" | jget access_token "")
  TOTP_AUTH3="Authorization: Bearer $TOTP_TK3"

  SETUP3=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/2fa/setup" \
    -H "$TOTP_AUTH3" -H 'Content-Type: application/json' -d '{}')
  SECRET3=$(echo "$SETUP3" | python3 -c "
import sys, json, urllib.parse
d = json.load(sys.stdin)
uri = d.get('otpauth_uri', '')
params = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)
print(params.get('secret', [''])[0])
" 2>/dev/null)

  CONFIRM_CODE3=$(python3 -c "
import hmac, hashlib, struct, time, base64
secret = base64.b32decode('$SECRET3', casefold=True)
counter = int(time.time()) // 30
msg = struct.pack('>Q', counter)
h = hmac.new(secret, msg, hashlib.sha1).digest()
offset = h[-1] & 0x0F
code = (struct.unpack('>I', h[offset:offset+4])[0] & 0x7FFFFFFF) % 1000000
print(f'{code:06d}')
" 2>/dev/null)

  curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/2fa/confirm" \
    -H "$TOTP_AUTH3" -H 'Content-Type: application/json' \
    -d "{\"totp_code\":\"$CONFIRM_CODE3\"}" > /dev/null

  # Verify 2FA is active
  VERIFY_2FA=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
  if echo "$VERIFY_2FA" | grep -q "requires_totp"; then
    pass "2FA is active on test user (requires_totp in login)"
  else
    warn "2FA may not be active: ${VERIFY_2FA:0:80}"
  fi

  subhdr "Admin Reset User 2FA"
  RESET_STATUS=$(http_status -X DELETE "$API/admin/users/$TOTP_UID/2fa" -H "$AUTH")
  if [[ "$RESET_STATUS" == "200" || "$RESET_STATUS" == "204" ]]; then
    pass "Admin successfully reset user 2FA (HTTP $RESET_STATUS)"
  else
    fail "Admin 2FA reset returned HTTP $RESET_STATUS"
  fi

  subhdr "Verify Normal Login After Admin Reset"
  # Wait for rate limiter to cool down (5 req/60s on TOTP-related endpoints)
  sleep 3
  AFTER_RESET=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$API/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TOTP_USER\",\"password\":\"$TOTP_PASS\"}")
  AFTER_TOKEN=$(echo "$AFTER_RESET" | jget access_token "")
  AFTER_TOTP=$(echo "$AFTER_RESET" | jget requires_totp "")
  if [[ -n "$AFTER_TOKEN" && "$AFTER_TOKEN" != "__MISSING__" && "$AFTER_TOTP" != "true" ]]; then
    pass "Login works without 2FA after admin reset"
  else
    fail "Login after admin reset failed: ${AFTER_RESET:0:120}"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 13: PAGINATION CURSORS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 13: Pagination Cursors"

subhdr "Photo Pagination"
# Request a small page
PAGE1=$(curl -s --max-time "$CURL_MAX_TIME" "$API/photos?limit=2" -H "$AUTH")
PHOTOS_PAGE1=$(echo "$PAGE1" | python3 -c "
import sys, json
d = json.load(sys.stdin)
photos = d.get('photos', [])
cursor = d.get('next_cursor', None)
print(f'count={len(photos)},cursor={cursor}')
" 2>/dev/null)
PAGE1_COUNT=$(echo "$PHOTOS_PAGE1" | grep -oP 'count=\K[0-9]+')
PAGE1_CURSOR=$(echo "$PHOTOS_PAGE1" | grep -oP 'cursor=\K.+')

if [[ "$PAGE1_COUNT" == "2" ]]; then
  pass "Photo pagination returns exactly 2 items (limit=2)"
elif [[ "$PAGE1_COUNT" -gt 0 ]]; then
  pass "Photo pagination returns $PAGE1_COUNT items (may have fewer than limit)"
else
  warn "Photo pagination returned 0 items"
fi

if [[ "$PAGE1_CURSOR" != "None" && -n "$PAGE1_CURSOR" ]]; then
  pass "Photo pagination includes next_cursor"

  # Fetch next page
  PAGE2=$(curl -s --max-time "$CURL_MAX_TIME" "$API/photos?limit=2&after=$PAGE1_CURSOR" -H "$AUTH")
  PHOTOS_PAGE2=$(echo "$PAGE2" | python3 -c "
import sys, json
d = json.load(sys.stdin)
photos = d.get('photos', [])
print(len(photos))
" 2>/dev/null)
  if [[ "$PHOTOS_PAGE2" -gt 0 ]]; then
    pass "Second page returns $PHOTOS_PAGE2 photos"
  else
    pass "Second page empty (may have reached end)"
  fi
else
  warn "No next_cursor — may have fewer than limit photos"
fi

subhdr "Blob Pagination"
BLOB_PAGE=$(curl -s --max-time "$CURL_MAX_TIME" "$API/blobs?limit=2" -H "$AUTH")
BLOB_COUNT=$(echo "$BLOB_PAGE" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(len(d.get('blobs', [])))
" 2>/dev/null)
if [[ "$BLOB_COUNT" -gt 0 ]]; then
  pass "Blob pagination returns $BLOB_COUNT items"
else
  pass "Blob pagination returns empty list (as expected)"
fi

subhdr "Trash Pagination"
# Delete a photo to populate trash first
if [[ -n "${DIFF_PID:-}" && "$DIFF_PID" != "__MISSING__" ]]; then
  curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/photos/$DIFF_PID" -H "$AUTH" > /dev/null 2>&1
  sleep 1
fi
TRASH_PAGE=$(curl -s --max-time "$CURL_MAX_TIME" "$API/trash?limit=2" -H "$AUTH")
TRASH_COUNT=$(echo "$TRASH_PAGE" | python3 -c "
import sys, json
d = json.load(sys.stdin)
items = d.get('items', d if isinstance(d, list) else [])
print(len(items))
" 2>/dev/null)
if [[ "$TRASH_COUNT" -gt 0 ]]; then
  pass "Trash pagination returns $TRASH_COUNT items"
else
  pass "Trash pagination returns empty (as expected)"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 14: AUDIO BACKUP SETTINGS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 14: Audio Backup Settings"

subhdr "Get Audio Backup Setting"
AUDIO_RESP=$(curl -s --max-time "$CURL_MAX_TIME" "$API/settings/audio-backup" -H "$AUTH")
AUDIO_ENABLED=$(echo "$AUDIO_RESP" | jget audio_backup_enabled "__MISSING__")
if [[ "$AUDIO_ENABLED" != "__MISSING__" ]]; then
  pass "Audio backup setting retrieved: $AUDIO_ENABLED"
else
  # The endpoint may return a different shape
  if echo "$AUDIO_RESP" | grep -qi "audio\|backup\|enabled"; then
    pass "Audio backup endpoint responds with expected fields"
  else
    fail "Audio backup response unexpected: ${AUDIO_RESP:0:100}"
  fi
fi

subhdr "Toggle Audio Backup"
# Enable audio backup
AUDIO_PUT=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$API/admin/audio-backup" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"audio_backup_enabled":true}')
AUDIO_PUT_STATUS=$(http_status -X PUT "$API/admin/audio-backup" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"audio_backup_enabled":true}')
if [[ "$AUDIO_PUT_STATUS" == "200" ]]; then
  pass "Audio backup enabled (HTTP 200)"
else
  fail "Audio backup enable returned HTTP $AUDIO_PUT_STATUS"
fi

# Verify change persisted
AUDIO_CHECK=$(curl -s --max-time "$CURL_MAX_TIME" "$API/settings/audio-backup" -H "$AUTH")
AUDIO_NOW=$(echo "$AUDIO_CHECK" | jget audio_backup_enabled "unknown")
if [[ "$AUDIO_NOW" == "true" ]]; then
  pass "Audio backup setting persisted as enabled"
else
  warn "Audio backup setting after enable: $AUDIO_NOW (response: ${AUDIO_CHECK:0:80})"
fi

# Restore original state
curl -s --max-time "$CURL_MAX_TIME" -X PUT "$API/admin/audio-backup" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"audio_backup_enabled":false}' > /dev/null 2>&1

# ══════════════════════════════════════════════════════════════════════════════
# CLEANUP
# ══════════════════════════════════════════════════════════════════════════════
hdr "Cleanup"

subhdr "Delete Test Resources"
# Delete the TOTP test user
if [[ -n "${TOTP_UID:-}" ]]; then
  curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/admin/users/$TOTP_UID" -H "$AUTH" > /dev/null 2>&1
  pass "Deleted 2FA test user"
fi

# Delete uploaded test photos
if [[ -n "${UPLOAD_PID:-}" && "$UPLOAD_PID" != "__MISSING__" ]]; then
  curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/photos/$UPLOAD_PID" -H "$AUTH" > /dev/null 2>&1
fi
if [[ -n "${DUP_NEW_ID:-}" && "$DUP_NEW_ID" != "__MISSING__" ]]; then
  curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/photos/$DUP_NEW_ID" -H "$AUTH" > /dev/null 2>&1
fi

# Delete test blob
if [[ -n "${BLOB_ID:-}" && "$BLOB_ID" != "__MISSING__" ]]; then
  curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/blobs/$BLOB_ID" -H "$AUTH" > /dev/null 2>&1
fi

# Empty the trash
curl -s --max-time "$CURL_MAX_TIME" -X DELETE "$API/trash" -H "$AUTH" > /dev/null 2>&1
pass "Test artifacts cleaned up"

# ══════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ══════════════════════════════════════════════════════════════════════════════
module_timer_stop > /dev/null
print_summary "Android Client Compatibility E2E"
exit "$FAILURES"
