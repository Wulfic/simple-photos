#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Module 03: SSL/TLS Certificate Management E2E Tests
# ══════════════════════════════════════════════════════════════════════════════
# Tests SSL/TLS configuration API endpoints:
#
#   1.  SSL Status — GET /api/admin/ssl (default = disabled)
#   2.  Self-Signed Cert Generation — create test certs with openssl
#   3.  Enable TLS — PUT /api/admin/ssl with valid cert + key paths
#   4.  Validation — missing paths, nonexistent files, empty paths
#   5.  Disable TLS — PUT /api/admin/ssl {enabled: false}
#   6.  Non-Admin Access — regular user blocked from SSL endpoints
#   7.  Config Persistence — verify config.toml is updated
#   8.  Backup SSL Limitation — document that sync is HTTP-only
#   9.  Cleanup — remove generated certs
#
# NOTE: We cannot actually restart the server mid-test, so we validate the
#       API layer (config writes, validation, auth) without testing live TLS
#       handshakes. The server logs "restart required" — that's expected.
#
# Prerequisites:
#   - Server running: sudo bash reset-server.sh
#   - openssl available in PATH
#
# Usage:
#   bash e2e/modules/03_ssl.sh [--verbose]
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/helpers.sh"
parse_common_args "$@"
setup_module_log "ssl"

module_timer_start "SSL/TLS Certificate Tests"

echo -e "${BOLD}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  SSL/TLS Certificate E2E Test Suite — Simple Photos           ║${NC}"
echo -e "${BOLD}╚════════════════════════════════════════════════════════════════╝${NC}"

# ── Locate server data directory for cert placement ──────────────────────────
# Certs must be readable by the server process. We'll place them in the
# server's certs/ directory so the path validation in ssl.rs succeeds.
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SERVER_CERTS_DIR="$REPO_ROOT/server/certs"
mkdir -p "$SERVER_CERTS_DIR"

SSL_CERT="$SERVER_CERTS_DIR/e2e_test.crt"
SSL_KEY="$SERVER_CERTS_DIR/e2e_test.key"
SSL_CA_KEY="$SERVER_CERTS_DIR/e2e_ca.key"
SSL_CA_CERT="$SERVER_CERTS_DIR/e2e_ca.crt"
CONFIG_TOML="$REPO_ROOT/server/config.toml"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 1: SSL STATUS (DEFAULT)
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 1: SSL Status (Default)"

subhdr "Ensure Server Initialized"
ensure_server_initialized "$MAIN_API" "$ADMIN_USER" "$ADMIN_PASS"

subhdr "Login as Admin"
TOKEN=$(login_and_get_token "$MAIN_API" "$ADMIN_USER" "$ADMIN_PASS" "fatal")
AUTH="Authorization: Bearer $TOKEN"

subhdr "Get SSL Status (should be disabled)"
SSL_STATUS=$(curl -s --max-time "$CURL_MAX_TIME" "$MAIN_API/admin/ssl" -H "$AUTH")
assert_json "SSL is disabled by default" "$SSL_STATUS" "enabled" "false"
assert_contains "SSL status has message" "$SSL_STATUS" "message"
assert_contains "Message mentions disabled/HTTP" "$SSL_STATUS" "disabled"

SSL_CERT_PATH=$(echo "$SSL_STATUS" | jget cert_path "null")
SSL_KEY_PATH=$(echo "$SSL_STATUS" | jget key_path "null")
log "Current cert_path: $SSL_CERT_PATH"
log "Current key_path:  $SSL_KEY_PATH"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 2: SELF-SIGNED CERTIFICATE GENERATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 2: Self-Signed Certificate Generation"

subhdr "Check openssl Availability"
if command -v openssl &>/dev/null; then
  OPENSSL_VER=$(openssl version 2>/dev/null || echo "unknown")
  pass "openssl is available: $OPENSSL_VER"
else
  fail "openssl not found in PATH — cannot generate test certs"
  warn "Skipping remaining SSL tests"
  module_timer_stop > /dev/null
  print_summary "SSL E2E"
  exit "$FAILURES"
fi

subhdr "Generate CA Key + Certificate"
openssl genrsa -out "$SSL_CA_KEY" 2048 2>/dev/null
openssl req -new -x509 -key "$SSL_CA_KEY" -out "$SSL_CA_CERT" -days 1 \
  -subj "/C=US/ST=Test/L=E2E/O=SimplePhotos/CN=E2E Test CA" 2>/dev/null

if [[ -f "$SSL_CA_KEY" && -f "$SSL_CA_CERT" ]]; then
  pass "CA key and certificate generated"
  CA_SIZE=$(stat -c%s "$SSL_CA_CERT" 2>/dev/null || stat -f%z "$SSL_CA_CERT" 2>/dev/null || echo 0)
  log "  CA cert size: ${CA_SIZE} bytes"
else
  fail "Failed to generate CA files"
fi

subhdr "Generate Server Key + Certificate (signed by CA)"
openssl genrsa -out "$SSL_KEY" 2048 2>/dev/null
openssl req -new -key "$SSL_KEY" -out "$SERVER_CERTS_DIR/e2e_test.csr" \
  -subj "/C=US/ST=Test/L=E2E/O=SimplePhotos/CN=localhost" 2>/dev/null

# Create SAN extension config for localhost
cat > "$SERVER_CERTS_DIR/e2e_san.cnf" <<EOF
[v3_req]
subjectAltName = @alt_names
[alt_names]
DNS.1 = localhost
IP.1 = 127.0.0.1
EOF

openssl x509 -req -in "$SERVER_CERTS_DIR/e2e_test.csr" \
  -CA "$SSL_CA_CERT" -CAkey "$SSL_CA_KEY" -CAcreateserial \
  -out "$SSL_CERT" -days 1 \
  -extfile "$SERVER_CERTS_DIR/e2e_san.cnf" -extensions v3_req 2>/dev/null

if [[ -f "$SSL_CERT" && -f "$SSL_KEY" ]]; then
  pass "Server certificate and key generated (signed by test CA)"
  CERT_SIZE=$(stat -c%s "$SSL_CERT" 2>/dev/null || stat -f%z "$SSL_CERT" 2>/dev/null || echo 0)
  KEY_SIZE=$(stat -c%s "$SSL_KEY" 2>/dev/null || stat -f%z "$SSL_KEY" 2>/dev/null || echo 0)
  log "  cert size: ${CERT_SIZE} bytes, key size: ${KEY_SIZE} bytes"
else
  fail "Failed to generate server certificate"
fi

subhdr "Verify Certificate Details"
CERT_SUBJECT=$(openssl x509 -in "$SSL_CERT" -noout -subject 2>/dev/null)
CERT_ISSUER=$(openssl x509 -in "$SSL_CERT" -noout -issuer 2>/dev/null)
CERT_DATES=$(openssl x509 -in "$SSL_CERT" -noout -dates 2>/dev/null)

if echo "$CERT_SUBJECT" | grep -qi "localhost"; then
  pass "Certificate CN = localhost"
else
  fail "Certificate CN does not contain 'localhost': $CERT_SUBJECT"
fi

if echo "$CERT_ISSUER" | grep -qi "E2E Test CA"; then
  pass "Certificate issued by E2E Test CA"
else
  fail "Certificate issuer unexpected: $CERT_ISSUER"
fi
log "  $CERT_DATES"

subhdr "Verify Key/Cert Pair Match"
CERT_MOD=$(openssl x509 -in "$SSL_CERT" -noout -modulus 2>/dev/null | sha256sum)
KEY_MOD=$(openssl rsa -in "$SSL_KEY" -noout -modulus 2>/dev/null | sha256sum)
if [[ "$CERT_MOD" == "$KEY_MOD" ]]; then
  pass "Certificate and key modulus match"
else
  fail "Certificate and key modulus DO NOT match"
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 3: VALIDATION — BAD INPUTS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 3: SSL Validation — Bad Inputs"

subhdr "Enable SSL with Empty Paths → 400"
BAD_EMPTY=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"cert_path":"","key_path":""}')
BAD_EMPTY_STATUS=$(echo "$BAD_EMPTY" | jget "error" "")
if echo "$BAD_EMPTY" | grep -qi "required\|path"; then
  pass "Empty paths rejected when enabling TLS"
else
  # Also check via HTTP status
  EMPTY_HTTP=$(http_status -X PUT "$MAIN_API/admin/ssl" \
    -H "$AUTH" -H 'Content-Type: application/json' \
    -d '{"enabled":true,"cert_path":"","key_path":""}')
  if [[ "$EMPTY_HTTP" == "400" ]]; then
    pass "Empty paths rejected (HTTP 400)"
  else
    fail "Empty paths should be rejected, got HTTP $EMPTY_HTTP: $BAD_EMPTY"
  fi
fi

subhdr "Enable SSL with Missing cert_path → 400"
MISS_CERT_STATUS=$(http_status -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"key_path":"'$SSL_KEY'"}')
if [[ "$MISS_CERT_STATUS" == "400" ]]; then
  pass "Missing cert_path rejected (HTTP 400)"
else
  fail "Missing cert_path should return 400, got $MISS_CERT_STATUS"
fi

subhdr "Enable SSL with Missing key_path → 400"
MISS_KEY_STATUS=$(http_status -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"cert_path":"'$SSL_CERT'"}')
if [[ "$MISS_KEY_STATUS" == "400" ]]; then
  pass "Missing key_path rejected (HTTP 400)"
else
  fail "Missing key_path should return 400, got $MISS_KEY_STATUS"
fi

subhdr "Enable SSL with Non-existent Cert File → 400"
NOFILE_CERT=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"cert_path":"/nonexistent/cert.pem","key_path":"'$SSL_KEY'"}')
NOFILE_CERT_HTTP=$(http_status -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"cert_path":"/nonexistent/cert.pem","key_path":"'$SSL_KEY'"}')
if [[ "$NOFILE_CERT_HTTP" == "400" ]]; then
  pass "Non-existent cert file rejected (HTTP 400)"
  assert_contains "Error mentions 'not found'" "$NOFILE_CERT" "not found"
else
  fail "Non-existent cert should return 400, got $NOFILE_CERT_HTTP"
fi

subhdr "Enable SSL with Non-existent Key File → 400"
NOFILE_KEY_HTTP=$(http_status -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"cert_path":"'$SSL_CERT'","key_path":"/nonexistent/key.pem"}')
if [[ "$NOFILE_KEY_HTTP" == "400" ]]; then
  pass "Non-existent key file rejected (HTTP 400)"
else
  fail "Non-existent key should return 400, got $NOFILE_KEY_HTTP"
fi

subhdr "Enable SSL with null Paths → 400"
NULL_PATHS_HTTP=$(http_status -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":true,"cert_path":null,"key_path":null}')
if [[ "$NULL_PATHS_HTTP" == "400" ]]; then
  pass "null paths rejected when enabling (HTTP 400)"
else
  fail "null paths should return 400, got $NULL_PATHS_HTTP"
fi

subhdr "Disable SSL (no paths needed) → 200"
DISABLE_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":false}')
assert_json "Disable TLS accepted" "$DISABLE_RESP" "enabled" "false"
assert_contains "Response mentions restart" "$DISABLE_RESP" "restart"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 4: ENABLE TLS WITH VALID CERTS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 4: Enable TLS with Valid Certificates"

subhdr "Enable TLS with Generated Certs"
ENABLE_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"enabled\":true,\"cert_path\":\"$SSL_CERT\",\"key_path\":\"$SSL_KEY\"}")
assert_json "TLS enabled successfully" "$ENABLE_RESP" "enabled" "true"
assert_contains "Response mentions restart" "$ENABLE_RESP" "restart"

RESP_CERT=$(echo "$ENABLE_RESP" | jget cert_path "")
RESP_KEY=$(echo "$ENABLE_RESP" | jget key_path "")

if [[ "$RESP_CERT" == "$SSL_CERT" ]]; then
  pass "Returned cert_path matches submitted path"
else
  fail "Returned cert_path mismatch: got '$RESP_CERT'"
fi

if [[ "$RESP_KEY" == "$SSL_KEY" ]]; then
  pass "Returned key_path matches submitted path"
else
  fail "Returned key_path mismatch: got '$RESP_KEY'"
fi

subhdr "Verify SSL Status Reflects Enabled"
# NOTE: The GET endpoint reads from in-memory state which is NOT updated
# until a server restart. The PUT only persists to config.toml.
# We verify the config file was updated instead of the GET response.
SSL_STATUS2=$(curl -s --max-time "$CURL_MAX_TIME" "$MAIN_API/admin/ssl" -H "$AUTH")
GET_ENABLED=$(echo "$SSL_STATUS2" | jget enabled "")
if [[ "$GET_ENABLED" == "true" ]]; then
  pass "SSL GET reflects enabled (in-memory updated)"
else
  # Expected: server requires restart for in-memory state to update
  warn "SSL GET still shows enabled=false (expected — restart required)"
  log "  Verifying config.toml was persisted instead..."
fi
assert_contains "Status response contains enabled field" "$SSL_STATUS2" "enabled"

# Verify config.toml was actually updated by the PUT
CONFIG_FILE="${CONFIG_TOML:-server/config.toml}"
if [[ -f "$CONFIG_FILE" ]]; then
  if grep -q 'enabled = true' "$CONFIG_FILE" 2>/dev/null; then
    pass "config.toml has TLS enabled=true"
  else
    fail "config.toml does not reflect TLS enabled"
  fi
  if grep -q "$SSL_CERT" "$CONFIG_FILE" 2>/dev/null; then
    pass "config.toml has correct cert_path"
  else
    fail "config.toml cert_path not found"
  fi
  if grep -q "$SSL_KEY" "$CONFIG_FILE" 2>/dev/null; then
    pass "config.toml has correct key_path"
  else
    fail "config.toml key_path not found"
  fi
else
  warn "Cannot verify config.toml — file not found at $CONFIG_FILE"
fi

subhdr "Re-enable (idempotent) — same paths"
REENABLE_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"enabled\":true,\"cert_path\":\"$SSL_CERT\",\"key_path\":\"$SSL_KEY\"}")
assert_json "Re-enable returns enabled=true" "$REENABLE_RESP" "enabled" "true"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 5: DISABLE TLS
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 5: Disable TLS"

subhdr "Disable TLS"
DISABLE_RESP2=$(curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":false}')
assert_json "TLS disabled" "$DISABLE_RESP2" "enabled" "false"

subhdr "Verify SSL Status Reflects Disabled"
SSL_STATUS3=$(curl -s --max-time "$CURL_MAX_TIME" "$MAIN_API/admin/ssl" -H "$AUTH")
assert_json "SSL status shows disabled" "$SSL_STATUS3" "enabled" "false"

subhdr "Server Still Responds on HTTP After Config Change"
HEALTH=$(curl -s --max-time 5 "$MAIN_BASE/health")
assert_contains "Server still healthy" "$HEALTH" "ok"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 6: NON-ADMIN ACCESS CONTROL
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 6: Non-Admin Access Control"

subhdr "Create Non-Admin User"
REG_RESP=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$MAIN_API/auth/register" \
  -H 'Content-Type: application/json' \
  -d '{"username":"sslnonadmin","password":"SslNonAdmin1!"}')
REG_LOGIN=$(curl -s --max-time "$CURL_MAX_TIME" -X POST "$MAIN_API/auth/login" \
  -H 'Content-Type: application/json' \
  -d '{"username":"sslnonadmin","password":"SslNonAdmin1!"}')
REG_TOKEN=$(echo "$REG_LOGIN" | jget access_token "")

if [[ -n "$REG_TOKEN" && "$REG_TOKEN" != "__MISSING__" ]]; then
  REG_AUTH="Authorization: Bearer $REG_TOKEN"
  pass "Non-admin user created and logged in"

  subhdr "GET /api/admin/ssl as Non-Admin → 403"
  assert_status "Non-admin GET ssl blocked" "403" \
    "$MAIN_API/admin/ssl" -H "$REG_AUTH"

  subhdr "PUT /api/admin/ssl as Non-Admin → 403"
  assert_status "Non-admin PUT ssl blocked" "403" \
    -X PUT "$MAIN_API/admin/ssl" -H "$REG_AUTH" \
    -H 'Content-Type: application/json' \
    -d '{"enabled":false}'

  # Cleanup: delete the test user
  REG_USER_ID=$(echo "$REG_RESP" | jget user_id "")
  if [[ -n "$REG_USER_ID" && "$REG_USER_ID" != "__MISSING__" ]]; then
    curl -s -X DELETE "$MAIN_API/admin/users/$REG_USER_ID" -H "$AUTH" > /dev/null 2>&1
  fi
else
  warn "Could not create non-admin user for SSL auth tests"
fi

subhdr "SSL Endpoints Without Auth → 401"
assert_status "No-auth GET ssl" "401" "$MAIN_API/admin/ssl"
assert_status "No-auth PUT ssl" "401" -X PUT "$MAIN_API/admin/ssl" \
  -H 'Content-Type: application/json' -d '{"enabled":false}'

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 7: CONFIG PERSISTENCE VERIFICATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 7: Config Persistence Verification"

subhdr "Enable TLS (for persistence check)"
curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d "{\"enabled\":true,\"cert_path\":\"$SSL_CERT\",\"key_path\":\"$SSL_KEY\"}" > /dev/null

subhdr "Read config.toml for TLS Section"
CONFIG_FILE="$REPO_ROOT/server/config.toml"
if [[ -f "$CONFIG_FILE" ]]; then
  if grep -q '\[tls\]' "$CONFIG_FILE"; then
    pass "config.toml has [tls] section"
  else
    fail "config.toml missing [tls] section"
  fi

  TLS_ENABLED_LINE=$(grep -E '^\s*enabled\s*=' "$CONFIG_FILE" | tail -1)
  if echo "$TLS_ENABLED_LINE" | grep -qi "true"; then
    pass "config.toml shows tls enabled = true"
  else
    fail "config.toml tls enabled not set to true: $TLS_ENABLED_LINE"
  fi

  if grep -q "e2e_test.crt" "$CONFIG_FILE"; then
    pass "config.toml contains cert path"
  else
    fail "config.toml missing cert path"
  fi

  if grep -q "e2e_test.key" "$CONFIG_FILE"; then
    pass "config.toml contains key path"
  else
    fail "config.toml missing key path"
  fi
else
  warn "Cannot read config.toml at $CONFIG_FILE"
fi

subhdr "Disable TLS (restore clean state)"
curl -s --max-time "$CURL_MAX_TIME" -X PUT "$MAIN_API/admin/ssl" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"enabled":false}' > /dev/null

subhdr "Verify config.toml Updated to Disabled"
if [[ -f "$CONFIG_FILE" ]]; then
  # The [tls] section should still exist but enabled = false
  TLS_FINAL=$(grep -A3 '\[tls\]' "$CONFIG_FILE" | grep -E '^\s*enabled')
  if echo "$TLS_FINAL" | grep -qi "false"; then
    pass "config.toml shows tls enabled = false after disable"
  else
    fail "config.toml not updated after disable: $TLS_FINAL"
  fi
fi

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 8: BACKUP SYNC SSL LIMITATION
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 8: Backup Sync SSL Limitation"

subhdr "Document: Backup Sync Uses HTTP Only"
# The backup sync code (server/src/backup/sync.rs) hardcodes:
#   format!("http://{}/api", server.address)
# This means backup sync between main and backup servers always uses HTTP,
# regardless of TLS settings. This is a known limitation.
warn "Backup sync hardcodes HTTP — TLS not used for server-to-server sync"
log "  This is a known limitation in sync.rs"
log "  Mixed encryption (TLS main + HTTP backup) works because sync bypasses TLS"
log "  For production: consider adding TLS support to sync.rs or using a reverse proxy"

# Verify the main server still works after all our SSL toggling
subhdr "Final Health Check After SSL Tests"
FINAL_HEALTH=$(curl -s --max-time 5 "$MAIN_BASE/health")
assert_contains "Server healthy after SSL tests" "$FINAL_HEALTH" "ok"

# ══════════════════════════════════════════════════════════════════════════════
# MODULE 9: CLEANUP
# ══════════════════════════════════════════════════════════════════════════════
hdr "Module 9: Cleanup"

subhdr "Remove Generated Certificates"
CLEANUP_FILES=(
  "$SSL_CERT" "$SSL_KEY" "$SSL_CA_KEY" "$SSL_CA_CERT"
  "$SERVER_CERTS_DIR/e2e_test.csr"
  "$SERVER_CERTS_DIR/e2e_san.cnf"
  "$SERVER_CERTS_DIR/e2e_ca.srl"
)
CLEANED=0
for f in "${CLEANUP_FILES[@]}"; do
  if [[ -f "$f" ]]; then
    rm -f "$f"
    CLEANED=$((CLEANED + 1))
  fi
done
pass "Removed $CLEANED generated cert files"

subhdr "Verify Certs Removed"
if [[ ! -f "$SSL_CERT" && ! -f "$SSL_KEY" ]]; then
  pass "Test certificates cleaned up successfully"
else
  fail "Some test certificates remain"
fi

# ══════════════════════════════════════════════════════════════════════════════
# FINAL SUMMARY
# ══════════════════════════════════════════════════════════════════════════════
module_timer_stop > /dev/null
print_summary "SSL E2E"
exit "$FAILURES"
