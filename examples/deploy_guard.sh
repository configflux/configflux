#!/usr/bin/env bash
# deploy_guard.sh — reference deploy guard for a ConfigFlux resolution.
#
# The guard gates the DEPLOY ACTION on PROVENANCE. A resolution may have been
# produced with a dev OVERLAY active: an optional, uncommitted, user-side
# composition layer that overloads a named environment's resolved values. When
# an overlay is active the resolution context records it as the well-known
# context_tags entry `overlay=<label>` (conventionally `dev`); that stamp
# travels with the snapshot and makes its divergent resolve_hash attributable
# (see docs/service-integration-guide.md and ADR 0035). This script reads that
# stamp and decides whether the snapshot may be deployed to a given target:
#
#   * Deploying an OVERLAY-ACTIVE resolution to a NON-LOCAL-CLASS target is
#     REFUSED unless an explicit --allow-overlay override is passed. Shipping a
#     local tweak to a shared, non-local-class target unnoticed is the mistake
#     this catches; the override makes the bypass an auditable operator choice.
#   * Deploying to a LOCAL-CLASS target is ALWAYS allowed (overlay or not) —
#     developer-local iteration is unobstructed.
#   * Deploying a NON-OVERLAY resolution is ALWAYS allowed (the provenance check
#     is a no-op).
#
# The target's CLASS is a user-owned, topology-neutral convention. The seam is
# the environment-manifest entry (the same manifest resolve_environment.sh
# reads), extended with an optional `class` field:
#
#   {
#     "schema_version": 3,
#     "environments": {
#       "<name>": { "scope": "...", "context_tags": {...}, "choices": {...},
#                   "class": "local" }
#     }
#   }
#
# A target is LOCAL-CLASS iff its manifest entry declares `class: "local"`. Any
# other value, or an ABSENT class, is NON-LOCAL-CLASS by default — a fail-safe:
# an unclassified target is treated as the protected case, so a forgotten
# classification refuses an overlay rather than waving it through.
#
# Like verify_bundle.sh (ADR-0032 D4) this is REFERENCE TOOLING, not product
# code. It reads ALREADY-PRODUCED JSON only (the snapshot's context_tags plus
# the manifest's class marker), invokes NO ConfigFlux binary, does not resolve,
# does not open a session, and adds no compiler->solver dependency. It is the
# overlay stamp that does the work; this script only acts on it. The guard is an
# OPTIONAL, EARLIER provenance check — it does NOT replace the on-target
# fail-closed runtime open (that correctness guarantee holds regardless of
# whether this script was ever run).
#
# Usage:
#   deploy_guard.sh --snapshot <resolve_result.json> \
#       --manifest <environments.json> --environment <name> [--allow-overlay]
#   deploy_guard.sh --snapshot <resolve_result.json> \
#       --target-class <class> [--allow-overlay]
#
# Options:
#   --snapshot <path>     Path to a resolved snapshot (resolve_result.*.json).
#                         Its context_tags.overlay (if present) is the stamp.
#   --manifest <path>     Path to the environment manifest JSON (the class seam).
#   --environment <name>  Target environment; its `class` is read from --manifest.
#   --target-class <cls>  Supply the target class directly (skips the manifest
#                         lookup). Mutually exclusive with --environment.
#   --allow-overlay       Explicit operator override: permit an overlay-active
#                         deploy to a non-local-class target.
#
# Exit status (mirrors verify_bundle.sh):
#   0  ALLOW  — the deploy is permitted
#   1  REFUSE — an overlay-active resolution targets a non-local-class
#               environment without --allow-overlay (or the snapshot/manifest is
#               malformed)
#   2  usage error (wrong arguments, or a required tool is unavailable)
#
# Dependencies: bash, jq. No ConfigFlux binary is required — the script reads two
# already-produced JSON files (the snapshot and the manifest) and compares
# fields that already exist.
set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
usage() {
  cat >&2 <<'USAGE'
Usage:
  deploy_guard.sh --snapshot <resolve_result.json> \
      --manifest <environments.json> --environment <name> [--allow-overlay]
  deploy_guard.sh --snapshot <resolve_result.json> \
      --target-class <class> [--allow-overlay]

Refuses to deploy an overlay-active resolution (context_tags.overlay set) to a
non-local-class target unless --allow-overlay is passed. A target is local-class
iff its manifest entry declares `class: "local"`; an absent/other class is
non-local-class by default (fail-safe).
USAGE
}

# Emit the terminal PASS/FAIL line and exit.
pass() { echo "PASS: $*"; exit 0; }
reject() { echo "FAIL: $*" >&2; exit 1; }

need_tool() {
  local tool="$1"
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "ERROR: required tool '${tool}' not found on PATH" >&2
    exit 2
  fi
}

# ---------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------
SNAPSHOT=""
MANIFEST=""
ENVIRONMENT=""
TARGET_CLASS=""
ALLOW_OVERLAY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --snapshot)      SNAPSHOT="${2:-}"; shift 2 ;;
    --manifest)      MANIFEST="${2:-}"; shift 2 ;;
    --environment)   ENVIRONMENT="${2:-}"; shift 2 ;;
    --target-class)  TARGET_CLASS="${2:-}"; shift 2 ;;
    --allow-overlay) ALLOW_OVERLAY=1; shift ;;
    -h|--help)       usage; exit 2 ;;
    *)               echo "ERROR: unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

# A snapshot is always required.
if [[ -z "${SNAPSHOT}" ]]; then
  usage
  exit 2
fi

# Exactly one class source: either (--manifest + --environment) or
# --target-class. They are mutually exclusive, and at least one is required.
if [[ -n "${TARGET_CLASS}" && ( -n "${MANIFEST}" || -n "${ENVIRONMENT}" ) ]]; then
  echo "ERROR: --target-class is mutually exclusive with --manifest/--environment" >&2
  usage
  exit 2
fi
if [[ -z "${TARGET_CLASS}" ]]; then
  if [[ -z "${MANIFEST}" || -z "${ENVIRONMENT}" ]]; then
    echo "ERROR: provide either --target-class <class>, or both --manifest <path> and --environment <name>" >&2
    usage
    exit 2
  fi
fi

need_tool jq

if [[ ! -f "${SNAPSHOT}" ]]; then
  echo "ERROR: snapshot not found: ${SNAPSHOT}" >&2
  exit 2
fi

# ---------------------------------------------------------------------------
# Read the overlay stamp from the snapshot's context_tags
# ---------------------------------------------------------------------------
# context_tags is a top-level {string:string} map; overlay is one entry. It is
# present iff an overlay was active. An empty/absent value means "no overlay".
if ! OVERLAY_LABEL="$(jq -er '.context_tags.overlay // ""' "${SNAPSHOT}" 2>/dev/null)"; then
  echo "ERROR: snapshot is unparseable or has no context_tags map: ${SNAPSHOT}" >&2
  exit 2
fi

OVERLAY_ACTIVE=0
if [[ -n "${OVERLAY_LABEL}" && "${OVERLAY_LABEL}" != "null" ]]; then
  OVERLAY_ACTIVE=1
fi

# ---------------------------------------------------------------------------
# Determine the target class (from --target-class, or the manifest entry)
# ---------------------------------------------------------------------------
TARGET_DESC=""
if [[ -n "${TARGET_CLASS}" ]]; then
  TARGET_DESC="class '${TARGET_CLASS}'"
else
  if [[ ! -f "${MANIFEST}" ]]; then
    echo "ERROR: environment manifest not found: ${MANIFEST}" >&2
    exit 2
  fi
  # The environment must exist in the manifest.
  if ! jq -e --arg e "${ENVIRONMENT}" '.environments | has($e)' \
    "${MANIFEST}" >/dev/null 2>&1; then
    echo "ERROR: environment '${ENVIRONMENT}' is not defined in the manifest: ${MANIFEST}" >&2
    exit 2
  fi
  # Read the optional class field; absent => empty string (treated non-local).
  if ! TARGET_CLASS="$(jq -er --arg e "${ENVIRONMENT}" \
    '.environments[$e].class // ""' "${MANIFEST}" 2>/dev/null)"; then
    echo "ERROR: manifest entry for '${ENVIRONMENT}' is unreadable: ${MANIFEST}" >&2
    exit 2
  fi
  TARGET_DESC="environment '${ENVIRONMENT}' (class '${TARGET_CLASS:-<unclassified>}')"
fi

# Local-class iff class == "local"; anything else (incl. empty) is non-local.
IS_LOCAL_CLASS=0
if [[ "${TARGET_CLASS}" == "local" ]]; then
  IS_LOCAL_CLASS=1
fi

# ---------------------------------------------------------------------------
# Decision: refuse iff overlay-active AND non-local-class AND not --allow-overlay
# ---------------------------------------------------------------------------
if [[ "${OVERLAY_ACTIVE}" -eq 1 ]]; then
  OVERLAY_NOTE="overlay '${OVERLAY_LABEL}' is active"
else
  OVERLAY_NOTE="no overlay is active"
fi

if [[ "${OVERLAY_ACTIVE}" -eq 1 && "${IS_LOCAL_CLASS}" -eq 0 && "${ALLOW_OVERLAY}" -eq 0 ]]; then
  echo "  snapshot       : ${SNAPSHOT}" >&2
  echo "  overlay label  : ${OVERLAY_LABEL}" >&2
  echo "  target         : ${TARGET_DESC}" >&2
  echo "  override       : --allow-overlay was NOT passed" >&2
  reject "refusing to deploy an overlay-active resolution to a non-local-class target (${TARGET_DESC}); pass --allow-overlay to override this as an explicit, auditable operator choice"
fi

# Otherwise: allow. Always surface whether an overlay was present and whether the
# target was local-class, so the stamp is never silent.
echo "  snapshot       : ${SNAPSHOT}"
echo "  provenance     : ${OVERLAY_NOTE}"
if [[ "${IS_LOCAL_CLASS}" -eq 1 ]]; then
  echo "  target         : ${TARGET_DESC} — local-class"
else
  echo "  target         : ${TARGET_DESC} — non-local-class"
fi
if [[ "${OVERLAY_ACTIVE}" -eq 1 && "${IS_LOCAL_CLASS}" -eq 0 && "${ALLOW_OVERLAY}" -eq 1 ]]; then
  echo "  override       : --allow-overlay passed (overlay-active deploy to a non-local-class target, by explicit choice)"
fi
pass "deploy permitted (${OVERLAY_NOTE}; ${TARGET_DESC})"
