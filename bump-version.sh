#!/usr/bin/env bash
# Cross-platform thin wrapper around bump-version.ps1.
#
# Requires PowerShell (Linux/macOS: `pwsh` from
# https://github.com/PowerShell/PowerShell). The actual logic lives in
# bump-version.ps1 so the two stay in lock-step.
#
# Usage: ./bump-version.sh 1.0.1               # edit only
#        ./bump-version.sh 1.0.1 --push        # edit, commit, tag, push
set -euo pipefail

if ! command -v pwsh >/dev/null 2>&1; then
  echo "error: pwsh (PowerShell 7+) not found on PATH." >&2
  echo "Install: https://github.com/PowerShell/PowerShell" >&2
  exit 1
fi

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <version> [--commit|--tag|--push|--dry-run]" >&2
  exit 1
fi

version="$1"; shift
args=( -Version "$version" )
for flag in "$@"; do
  case "$flag" in
    --commit)  args+=( -Commit ) ;;
    --tag)     args+=( -Tag ) ;;
    --push)    args+=( -Push ) ;;
    --dry-run|-n) args+=( -DryRun ) ;;
    *) echo "unknown flag: $flag" >&2; exit 1 ;;
  esac
done

script_dir="$(cd "$(dirname "$0")" && pwd)"
exec pwsh -NoProfile -ExecutionPolicy Bypass -File "$script_dir/bump-version.ps1" "${args[@]}"
