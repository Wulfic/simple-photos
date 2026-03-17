#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# Performance Timing Utilities for Simple Photos E2E Tests
# ══════════════════════════════════════════════════════════════════════════════
# Provides nanosecond-precision timers for per-section and per-module timing.
# Sourced by helpers.sh. Do NOT execute directly.
# ══════════════════════════════════════════════════════════════════════════════

# Associative arrays for named timers
declare -gA _TIMER_STARTS
declare -gA _TIMER_RESULTS

# Global module timer
_MODULE_START_NS=0
_MODULE_NAME=""

# Section timing log (ordered list for final report)
declare -ga _TIMING_LOG_NAMES
declare -ga _TIMING_LOG_MS

# ── Timer Functions ──────────────────────────────────────────────────────────

# Start a named timer.
#   timer_start "section_name"
timer_start() {
  local name="$1"
  _TIMER_STARTS["$name"]=$(date +%s%N)
}

# Stop a named timer and record the elapsed time.
# Returns elapsed milliseconds on stdout.
#   elapsed=$(timer_stop "section_name")
timer_stop() {
  local name="$1"
  local end_ns=$(date +%s%N)
  local start_ns="${_TIMER_STARTS[$name]:-$end_ns}"
  local elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
  _TIMER_RESULTS["$name"]=$elapsed_ms
  _TIMING_LOG_NAMES+=("$name")
  _TIMING_LOG_MS+=("$elapsed_ms")
  echo "$elapsed_ms"
}

# Get elapsed time without stopping the timer.
#   elapsed=$(timer_elapsed "section_name")
timer_elapsed() {
  local name="$1"
  local now_ns=$(date +%s%N)
  local start_ns="${_TIMER_STARTS[$name]:-$now_ns}"
  echo $(( (now_ns - start_ns) / 1000000 ))
}

# Format milliseconds into human-readable string.
#   format_duration 12345  →  "12.3s"
#   format_duration 345    →  "345ms"
#   format_duration 65432  →  "1m 5.4s"
format_duration() {
  local ms=$1
  if (( ms < 1000 )); then
    echo "${ms}ms"
  elif (( ms < 60000 )); then
    python3 -c "print(f'{$ms/1000:.1f}s')"
  else
    local mins=$(( ms / 60000 ))
    local remainder_ms=$(( ms % 60000 ))
    local secs
    secs=$(python3 -c "print(f'{$remainder_ms/1000:.1f}')")
    echo "${mins}m ${secs}s"
  fi
}

# ── Module-Level Timing ─────────────────────────────────────────────────────

# Call at the start of a module to begin module-level timing.
#   module_timer_start "Core Tests"
module_timer_start() {
  _MODULE_NAME="$1"
  _MODULE_START_NS=$(date +%s%N)
}

# Call at the end of a module. Prints elapsed time and returns ms.
#   module_timer_stop
module_timer_stop() {
  local end_ns=$(date +%s%N)
  local elapsed_ms=$(( (end_ns - _MODULE_START_NS) / 1000000 ))
  local formatted
  formatted=$(format_duration "$elapsed_ms")
  echo -e "${CYAN}[TIMING]${NC} Module '${_MODULE_NAME}' completed in ${BOLD}${formatted}${NC}"
  echo "$elapsed_ms"
}

# ── Timing Report ────────────────────────────────────────────────────────────

# Print a detailed timing breakdown for all recorded sections.
print_timing_report() {
  local count=${#_TIMING_LOG_NAMES[@]}
  if (( count == 0 )); then
    return
  fi

  echo ""
  echo -e "${BOLD}════════════════════════════════════════════════════════════════${NC}"
  echo -e "${BOLD}  Performance Timing Report${NC}"
  echo -e "${BOLD}════════════════════════════════════════════════════════════════${NC}"
  echo ""

  # Find the longest section name for alignment
  local max_len=0
  for name in "${_TIMING_LOG_NAMES[@]}"; do
    (( ${#name} > max_len )) && max_len=${#name}
  done

  # Calculate total for percentage display
  local total_ms=0
  for ms in "${_TIMING_LOG_MS[@]}"; do
    total_ms=$(( total_ms + ms ))
  done

  printf "  ${BOLD}%-${max_len}s  %10s  %6s${NC}\n" "Section" "Duration" "Share"
  printf "  %${max_len}s  %10s  %6s\n" "$(printf '%0.s─' $(seq 1 $max_len))" "──────────" "──────"

  for i in $(seq 0 $((count - 1))); do
    local name="${_TIMING_LOG_NAMES[$i]}"
    local ms="${_TIMING_LOG_MS[$i]}"
    local formatted
    formatted=$(format_duration "$ms")
    local pct
    if (( total_ms > 0 )); then
      pct=$(python3 -c "print(f'{$ms/$total_ms*100:.1f}%')")
    else
      pct="--"
    fi

    # Color code: red if > 30s, yellow if > 10s, green otherwise
    local color="$GREEN"
    (( ms > 10000 )) && color="$YELLOW"
    (( ms > 30000 )) && color="$RED"

    printf "  %-${max_len}s  ${color}%10s${NC}  %6s\n" "$name" "$formatted" "$pct"
  done

  echo ""
  local total_formatted
  total_formatted=$(format_duration "$total_ms")
  printf "  ${BOLD}%-${max_len}s  %10s${NC}\n" "TOTAL" "$total_formatted"
  echo ""
}
