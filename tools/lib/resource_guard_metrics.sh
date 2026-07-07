#!/usr/bin/env bash
# Helpers for tools/resource_guard.sh that need to live outside the main
# script for two reasons:
#   1) Each helper is independently testable (mock /proc files, mock event
#      log, mock RSS values) without spinning up a full guarded child.
#   2) tools/resource_guard.sh sits at the lint LOC cap (492). Adding the
#      PSI + RSS-panic + log-rotation features inline would push it past
#      the cap and require either a budget bump or a cleanup pass. The
#      tools/lib/ directory is the established pattern for this kind of
#      decomposition (gate_common.sh, cc_resolver.sh).
#
# This file is sourced, never executed. It MUST be safe under `set -euo
# pipefail` in the caller — every function that can fail returns the
# expected non-zero status without short-circuiting via `exit`.

# Read PSI memory pressure from a file (typically /proc/pressure/memory)
# and echo the integer "some avg10" percentage.
#
# Returns 0 on success (output is valid percentage in 0..100).
# Returns 1 when the source is empty / missing / unparseable. Callers MUST
# treat any non-zero return as "PSI unavailable, skip this sample".
#
# Sample input lines (real /proc/pressure/memory format):
#   some avg10=12.34 avg60=5.67 avg300=1.11 total=2345678
#   full avg10=0.00 avg60=0.00 avg300=0.00 total=0
#
# We use the `some` line because that is what trips when ANY task is
# memory-stalled — the early-warning signal. `full` only fires when EVERY
# runnable task is stalled, by which point WSL2 has already swap-thrashed.
configflux_guard_read_psi() {
  local source_path="$1"
  if [[ -z "${source_path}" ]] || [[ ! -r "${source_path}" ]]; then
    return 1
  fi

  # awk reads the 'some' line, extracts avg10's value, and prints it as
  # a float. We then round to int via printf to avoid bash's lack of
  # native float comparison.
  local avg10_float
  avg10_float="$(
    awk '
      $1 == "some" {
        for (i = 2; i <= NF; i++) {
          if (index($i, "avg10=") == 1) {
            sub(/^avg10=/, "", $i)
            print $i
            exit
          }
        }
      }
    ' "${source_path}" 2>/dev/null
  )"

  if [[ -z "${avg10_float}" ]]; then
    return 1
  fi

  # Reject obviously bad values; round half-up to integer percent.
  if ! [[ "${avg10_float}" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
    return 1
  fi

  printf '%d\n' "$(awk -v v="${avg10_float}" 'BEGIN { printf "%d", v + 0.5 }')"
}

# Predicate: does observed RSS breach the 1-sample panic threshold?
#
# Args:
#   $1 — observed aggregate RSS in MB (integer)
#   $2 — configured MAX_RSS_MB (integer, must be >0)
#   $3 — RSS_PANIC_MULTIPLIER (integer, must be >=1)
#
# Echoes the computed panic threshold for logging, exits 0 (true) when
# observed > threshold, exit 1 (false) otherwise. Callers may discard the
# stdout if they only need the boolean.
configflux_guard_rss_panic_check() {
  local observed="$1" max_rss="$2" multiplier="$3"

  if [[ "${max_rss}" =~ ^[0-9]+$ ]] && [[ "${multiplier}" =~ ^[0-9]+$ ]]; then
    local threshold=$((max_rss * multiplier))
    printf '%d\n' "${threshold}"
    if [[ "${observed}" =~ ^[0-9]+$ ]] && (( observed > threshold )); then
      return 0
    fi
  fi
  return 1
}

# Rotate an event log file when it exceeds a byte threshold. Keeps one
# rotation level: the previous .1 is overwritten on each rotation. This
# is intentional — the log is for "what just panicked" review, not
# long-term forensics. CRA evidence bundles capture the longer-term
# record separately.
#
# Args:
#   $1 — path to the event log
#   $2 — max bytes (integer; 0 disables rotation)
#
# Always returns 0. Best-effort — failures are silent.
configflux_guard_rotate_event_log() {
  local path="$1" max_bytes="$2"
  if [[ -z "${path}" ]] || [[ ! -f "${path}" ]]; then
    return 0
  fi
  if [[ -z "${max_bytes}" ]] || [[ "${max_bytes}" == "0" ]]; then
    return 0
  fi
  if ! [[ "${max_bytes}" =~ ^[0-9]+$ ]]; then
    return 0
  fi

  local size
  size="$(stat -c%s "${path}" 2>/dev/null || echo 0)"
  if (( size > max_bytes )); then
    mv -f "${path}" "${path}.1" 2>/dev/null || true
    : > "${path}" 2>/dev/null || true
  fi
  return 0
}
