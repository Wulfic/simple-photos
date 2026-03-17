#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Master E2E Test Runner for Simple Photos
# ══════════════════════════════════════════════════════════════════════════════
# Runs all (or selected) E2E test modules sequentially, captures their output,
# provides per-module timing, and produces an aggregate summary.
#
# All sub-script output is piped through this single terminal — no need for
# multiple terminals.
#
# Usage:
#   bash e2e/run_all.sh                           # Run all modules
#   bash e2e/run_all.sh --only=core,ssl           # Run specific modules
#   bash e2e/run_all.sh --skip=concurrent         # Skip specific modules
#   bash e2e/run_all.sh --verbose                 # Pass --verbose to modules
#   bash e2e/run_all.sh --list                    # List available modules
#
# Prerequisites:
#   - Server built: cd server && cargo build --release
#   - Server running: sudo bash reset-server.sh
#   - Docker backup instances (for backup module):
#       cd docker-instances && docker compose up -d
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail

# ── Resolve paths ────────────────────────────────────────────────────────────
RUNNER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULES_DIR="$RUNNER_DIR/modules"

# Source the shared libraries (for colors, timing, etc.)
source "$RUNNER_DIR/lib/helpers.sh"

# Tell modules NOT to set up their own logging — we handle it here
export _E2E_MASTER_RUNNER=1

# ── Master Log File (overwritten each run) ───────────────────────────────────
MASTER_LOG="$E2E_LOG_DIR/e2e_results.log"
MASTER_LOG_RAW="$E2E_TMP_DIR/.master_raw_$$.log"

# Tee all output: terminal gets colors, raw file gets everything
exec > >(tee "$MASTER_LOG_RAW") 2>&1

# On exit: strip ANSI codes and write the clean log
_cleanup_master_log() {
  sleep 0.3  # let tee flush
  if [[ -f "$MASTER_LOG_RAW" ]]; then
    sed 's/\x1b\[[0-9;]*m//g' "$MASTER_LOG_RAW" > "$MASTER_LOG" 2>/dev/null
    rm -f "$MASTER_LOG_RAW"
  fi
}
trap '_cleanup_master_log' EXIT

# ── Module Registry ──────────────────────────────────────────────────────────
# Each entry: "shortname:filename:description"
declare -a MODULE_REGISTRY=(
  "core:01_core.sh:Core API tests (auth, photos, tags, trash, albums, etc.)"
  "backup:02_backup.sh:Backup server tests (sync, recovery, multi-backup)"
  "ssl:03_ssl.sh:SSL/TLS certificate management tests"
  "concurrent:04_concurrent.sh:Concurrent multi-user stress tests"
)

# ── Parse Arguments ──────────────────────────────────────────────────────────
ONLY_MODULES=""
SKIP_MODULES=""
MODULE_ARGS=()  # Args to pass through to modules
LIST_ONLY=false

for arg in "$@"; do
  case $arg in
    --only=*)
      ONLY_MODULES="${arg#--only=}"
      ;;
    --skip=*)
      SKIP_MODULES="${arg#--skip=}"
      ;;
    --list)
      LIST_ONLY=true
      ;;
    --verbose|--skip-reset|--skip-recovery)
      MODULE_ARGS+=("$arg")
      ;;
    --help|-h)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --only=mod1,mod2    Run only specified modules (comma-separated)"
      echo "  --skip=mod1,mod2    Skip specified modules"
      echo "  --verbose           Pass --verbose to all modules"
      echo "  --skip-reset        Pass --skip-reset to backup module"
      echo "  --skip-recovery     Pass --skip-recovery to backup module"
      echo "  --list              List available modules and exit"
      echo ""
      echo "Available modules:"
      for entry in "${MODULE_REGISTRY[@]}"; do
        local_name="${entry%%:*}"
        local_rest="${entry#*:}"
        local_desc="${local_rest#*:}"
        printf "  %-12s %s\n" "$local_name" "$local_desc"
      done
      exit 0
      ;;
    *)
      echo "Unknown option: $arg (use --help for usage)"
      exit 1
      ;;
  esac
done

# ── List Mode ────────────────────────────────────────────────────────────────
if $LIST_ONLY; then
  echo "Available E2E test modules:"
  echo ""
  for entry in "${MODULE_REGISTRY[@]}"; do
    local_name="${entry%%:*}"
    local_rest="${entry#*:}"
    local_file="${local_rest%%:*}"
    local_desc="${local_rest#*:}"
    printf "  ${BOLD}%-12s${NC} %-20s %s\n" "$local_name" "($local_file)" "$local_desc"
  done
  echo ""
  echo "Run with: $0 --only=core,backup"
  exit 0
fi

# ── Determine Which Modules to Run ───────────────────────────────────────────
should_run_module() {
  local name="$1"

  # If --only is set, only run listed modules
  if [[ -n "$ONLY_MODULES" ]]; then
    if echo ",$ONLY_MODULES," | grep -q ",$name,"; then
      return 0
    fi
    return 1
  fi

  # If --skip is set, skip listed modules
  if [[ -n "$SKIP_MODULES" ]]; then
    if echo ",$SKIP_MODULES," | grep -q ",$name,"; then
      return 1
    fi
  fi

  return 0
}

# ── Banner ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}╔══════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║                                                                  ║${NC}"
echo -e "${BOLD}║   Simple Photos — Master E2E Test Runner                         ║${NC}"
echo -e "${BOLD}║                                                                  ║${NC}"
echo -e "${BOLD}╚══════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} Starting test run at $(date '+%Y-%m-%d %H:%M:%S')"

# List modules being run
echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} Modules to run:"
RUN_COUNT=0
for entry in "${MODULE_REGISTRY[@]}"; do
  local_name="${entry%%:*}"
  local_rest="${entry#*:}"
  local_desc="${local_rest#*:}"
  if should_run_module "$local_name"; then
    echo -e "  ${GREEN}●${NC} $local_name — $local_desc"
    RUN_COUNT=$((RUN_COUNT + 1))
  else
    echo -e "  ${DIM}○ $local_name — $local_desc (skipped)${NC}"
  fi
done

if [[ $RUN_COUNT -eq 0 ]]; then
  echo -e "\n${RED}No modules selected to run. Use --list to see available modules.${NC}"
  exit 1
fi
echo ""

# ── Run Modules ──────────────────────────────────────────────────────────────
GLOBAL_START=$(date +%s%N)

declare -a RAN_MODULES
declare -a MODULE_EXITS
declare -a MODULE_TIMES_MS
declare -a MODULE_TOTALS
declare -a MODULE_PASSES
declare -a MODULE_FAILURES
declare -a MODULE_WARNINGS

MODULE_INDEX=0

for entry in "${MODULE_REGISTRY[@]}"; do
  local_name="${entry%%:*}"
  local_rest="${entry#*:}"
  local_file="${local_rest%%:*}"
  local_desc="${local_rest#*:}"

  if ! should_run_module "$local_name"; then
    continue
  fi

  MODULE_SCRIPT="$MODULES_DIR/$local_file"
  if [[ ! -f "$MODULE_SCRIPT" ]]; then
    echo -e "${YELLOW}[WARN]${NC} Module script not found: $MODULE_SCRIPT — skipping"
    continue
  fi

  echo ""
  echo -e "${BOLD}${BLUE}┌──────────────────────────────────────────────────────────────────┐${NC}"
  echo -e "${BOLD}${BLUE}│  Running: $local_name ($local_file)${NC}"
  echo -e "${BOLD}${BLUE}│  $local_desc${NC}"
  echo -e "${BOLD}${BLUE}└──────────────────────────────────────────────────────────────────┘${NC}"

  MOD_START=$(date +%s%N)
  MOD_LOG="$E2E_TMP_DIR/${local_name}.log"

  # Run the module in a subshell, capturing output with tee.
  # The module's stdout/stderr is both displayed and saved to a log file.
  # We use a prefix for visual clarity.
  (
    bash "$MODULE_SCRIPT" "${MODULE_ARGS[@]}" 2>&1
  ) 2>&1 | tee "$MOD_LOG" | sed "s/^/  ${DIM}[$local_name]${NC} /"

  # Capture the exit code (from PIPESTATUS since we used a pipe)
  MOD_EXIT=${PIPESTATUS[0]}

  MOD_END=$(date +%s%N)
  MOD_ELAPSED_MS=$(( (MOD_END - MOD_START) / 1000000 ))
  MOD_ELAPSED_FMT=$(format_duration "$MOD_ELAPSED_MS")

  # Parse the machine-readable result line from the module's output
  RESULT_LINE=$(grep "^E2E_RESULT:" "$MOD_LOG" 2>/dev/null | tail -1)
  MOD_TOTAL=0
  MOD_PASSES=0
  MOD_FAILURES=0
  MOD_WARNINGS=0

  if [[ -n "$RESULT_LINE" ]]; then
    MOD_TOTAL=$(echo "$RESULT_LINE" | sed 's/.*total=\([0-9]*\).*/\1/')
    MOD_PASSES=$(echo "$RESULT_LINE" | sed 's/.*passes=\([0-9]*\).*/\1/')
    MOD_FAILURES=$(echo "$RESULT_LINE" | sed 's/.*failures=\([0-9]*\).*/\1/')
    MOD_WARNINGS=$(echo "$RESULT_LINE" | sed 's/.*warnings=\([0-9]*\).*/\1/')
  fi

  # Record results
  RAN_MODULES+=("$local_name")
  MODULE_EXITS+=("$MOD_EXIT")
  MODULE_TIMES_MS+=("$MOD_ELAPSED_MS")
  MODULE_TOTALS+=("$MOD_TOTAL")
  MODULE_PASSES+=("$MOD_PASSES")
  MODULE_FAILURES+=("$MOD_FAILURES")
  MODULE_WARNINGS+=("$MOD_WARNINGS")

  # Print module result
  if [[ "$MOD_EXIT" -eq 0 ]]; then
    echo -e "\n${GREEN}  ✓ $local_name completed successfully${NC} (${MOD_ELAPSED_FMT})"
  else
    echo -e "\n${RED}  ✗ $local_name FAILED (exit code $MOD_EXIT)${NC} (${MOD_ELAPSED_FMT})"
  fi

  MODULE_INDEX=$((MODULE_INDEX + 1))
done

GLOBAL_END=$(date +%s%N)
GLOBAL_MS=$(( (GLOBAL_END - GLOBAL_START) / 1000000 ))
GLOBAL_FMT=$(format_duration "$GLOBAL_MS")

# ── Aggregate Summary ────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}╔══════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  AGGREGATE E2E TEST RESULTS                                      ║${NC}"
echo -e "${BOLD}╠══════════════════════════════════════════════════════════════════╣${NC}"

AGG_TOTAL=0
AGG_PASSES=0
AGG_FAILURES=0
AGG_WARNINGS=0

for i in $(seq 0 $((${#RAN_MODULES[@]} - 1))); do
  name="${RAN_MODULES[$i]}"
  exit_code="${MODULE_EXITS[$i]}"
  elapsed="${MODULE_TIMES_MS[$i]}"
  total="${MODULE_TOTALS[$i]}"
  passes="${MODULE_PASSES[$i]}"
  failures="${MODULE_FAILURES[$i]}"
  warnings="${MODULE_WARNINGS[$i]}"

  AGG_TOTAL=$((AGG_TOTAL + total))
  AGG_PASSES=$((AGG_PASSES + passes))
  AGG_FAILURES=$((AGG_FAILURES + failures))
  AGG_WARNINGS=$((AGG_WARNINGS + warnings))

  elapsed_fmt=$(format_duration "$elapsed")

  if [[ "$exit_code" -eq 0 ]]; then
    icon="${GREEN}✓${NC}"
  else
    icon="${RED}✗${NC}"
  fi

  printf "${BOLD}║${NC}  $icon %-14s  %4s tests  ${GREEN}%4s passed${NC}  ${RED}%3s failed${NC}  ${YELLOW}%3s warns${NC}  %10s ${BOLD}║${NC}\n" \
    "$name" "$total" "$passes" "$failures" "$warnings" "$elapsed_fmt"
done

echo -e "${BOLD}╠══════════════════════════════════════════════════════════════════╣${NC}"
printf "${BOLD}║  TOTAL         %5s tests  ${GREEN}%4s passed${NC}  ${RED}%3s failed${NC}  ${YELLOW}%3s warns${NC}  %10s ${BOLD}║${NC}\n" \
  "$AGG_TOTAL" "$AGG_PASSES" "$AGG_FAILURES" "$AGG_WARNINGS" "$GLOBAL_FMT"

if [[ "$AGG_FAILURES" -eq 0 ]]; then
  echo -e "${BOLD}║                                                                  ║${NC}"
  echo -e "${BOLD}║  ${GREEN}✓ ALL $AGG_PASSES TESTS PASSED ACROSS ALL MODULES${NC}${BOLD}                ║${NC}"
else
  echo -e "${BOLD}║                                                                  ║${NC}"
  echo -e "${BOLD}║  ${RED}✗ $AGG_FAILURES TOTAL FAILURES ACROSS ALL MODULES${NC}${BOLD}                  ║${NC}"
fi
echo -e "${BOLD}╚══════════════════════════════════════════════════════════════════╝${NC}"

# ── Per-Module Timing Breakdown ──────────────────────────────────────────────
echo ""
echo -e "${BOLD}  Module Timing Breakdown:${NC}"
for i in $(seq 0 $((${#RAN_MODULES[@]} - 1))); do
  name="${RAN_MODULES[$i]}"
  elapsed="${MODULE_TIMES_MS[$i]}"
  elapsed_fmt=$(format_duration "$elapsed")
  pct="--"
  if (( GLOBAL_MS > 0 )); then
    pct=$(python3 -c "print(f'{$elapsed/$GLOBAL_MS*100:.1f}%')")
  fi

  # Color code timing
  color="$GREEN"
  (( elapsed > 30000 )) && color="$YELLOW"
  (( elapsed > 120000 )) && color="$RED"

  printf "    %-14s ${color}%10s${NC}  (%s)\n" "$name" "$elapsed_fmt" "$pct"
done
echo ""

echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} Test run completed at $(date '+%Y-%m-%d %H:%M:%S')"
echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} Per-module logs: $E2E_TMP_DIR/"
echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} ${BOLD}Master log: $MASTER_LOG${NC} (overwritten each run)"
echo ""

# Exit with total failures
exit "$AGG_FAILURES"
