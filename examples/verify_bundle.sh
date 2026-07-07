#!/usr/bin/env bash
# verify_bundle.sh — reference verification for a ConfigFlux delivery bundle.
#
# A delivery bundle is the standard, self-contained delivery unit for one
# resolved configuration. Its layout (see docs/service-integration-guide.md):
#
#   <bundle>/
#     resolve_result.<root>.<selection>.json   # the per-scope resolved snapshot
#     ccm/                                      # the solver model
#       ccm.manifest.json
#       ccm.symbols.json
#       partition-*/ccm.bdd.bin
#
# This script cross-checks that a bundle is a matched, complete unit BEFORE it
# is shipped, baked into an image, or mounted. It verifies:
#
#   1. The snapshot's `model_hash` equals the solver model's `bound_model_hash`
#      (from ccm/ccm.manifest.json). A mismatch means the snapshot and the
#      ccm/ do not belong together — a mis-assembled bundle.
#   2. The snapshot's content hash (sha256 of the snapshot file). It is always
#      recomputed and printed; if an expected value is supplied as the second
#      argument it must match.
#
# This check is an OPTIONAL pre-assembly convenience. The on-target correctness
# guarantee is the fail-closed runtime open: a validated runtime session
# refuses to start without a usable ccm/ solver model alongside the snapshot,
# and that holds regardless of whether this script was ever run. This script
# only lets a pipeline catch a mis-assembled bundle earlier, before delivery.
#
# Usage:
#   verify_bundle.sh <bundle-dir> [expected-snapshot-content-hash]
#
# Exit status:
#   0  the bundle is a well-formed, matched unit (and, if an expected content
#      hash was supplied, it matched)
#   1  the bundle is rejected (missing/unparseable snapshot or ccm manifest,
#      mismatched model_hash/bound_model_hash, or content-hash mismatch)
#   2  usage error (wrong arguments, or a required tool is unavailable)
#
# Dependencies: bash, jq, sha256sum. No ConfigFlux binary is required — the
# script reads two already-produced JSON files and compares hash fields.
set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
usage() {
  cat >&2 <<'USAGE'
Usage: verify_bundle.sh <bundle-dir> [expected-snapshot-content-hash]

Cross-checks a ConfigFlux delivery bundle: the snapshot's model_hash against
the solver model's bound_model_hash, and the snapshot's content hash.
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
# Arguments and environment
# ---------------------------------------------------------------------------
if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 2
fi

BUNDLE_DIR="$1"
EXPECTED_CONTENT_HASH="${2:-}"

need_tool jq
need_tool sha256sum

if [[ ! -d "${BUNDLE_DIR}" ]]; then
  echo "ERROR: bundle directory not found: ${BUNDLE_DIR}" >&2
  exit 2
fi

# ---------------------------------------------------------------------------
# Locate the single resolved snapshot in the bundle root
# ---------------------------------------------------------------------------
# The standard layout places exactly one per-scope snapshot at the bundle root,
# named resolve_result.<root>.<selection>.json. Globbing must match exactly one.
shopt -s nullglob
SNAPSHOTS=("${BUNDLE_DIR}"/resolve_result.*.json)
shopt -u nullglob

if [[ ${#SNAPSHOTS[@]} -eq 0 ]]; then
  reject "no resolve_result.*.json snapshot found in bundle root '${BUNDLE_DIR}'"
fi
if [[ ${#SNAPSHOTS[@]} -gt 1 ]]; then
  reject "expected exactly one resolve_result.*.json in bundle root, found ${#SNAPSHOTS[@]}: ${SNAPSHOTS[*]}"
fi
SNAPSHOT="${SNAPSHOTS[0]}"

# ---------------------------------------------------------------------------
# Read model_hash from the snapshot
# ---------------------------------------------------------------------------
if ! SNAPSHOT_MODEL_HASH="$(jq -er '.model_hash' "${SNAPSHOT}" 2>/dev/null)"; then
  reject "snapshot is unparseable or has no top-level 'model_hash': ${SNAPSHOT}"
fi
if [[ -z "${SNAPSHOT_MODEL_HASH}" || "${SNAPSHOT_MODEL_HASH}" == "null" ]]; then
  reject "snapshot 'model_hash' is empty: ${SNAPSHOT}"
fi

# ---------------------------------------------------------------------------
# Read bound_model_hash from ccm/ccm.manifest.json
# ---------------------------------------------------------------------------
CCM_MANIFEST="${BUNDLE_DIR}/ccm/ccm.manifest.json"
if [[ ! -d "${BUNDLE_DIR}/ccm" ]]; then
  reject "ccm/ directory is missing from the bundle — a validated runtime session cannot open without it"
fi
if [[ ! -f "${CCM_MANIFEST}" ]]; then
  reject "ccm/ccm.manifest.json is missing from the bundle"
fi
if ! CCM_BOUND_MODEL_HASH="$(jq -er '.bound_model_hash' "${CCM_MANIFEST}" 2>/dev/null)"; then
  reject "ccm/ccm.manifest.json is unparseable or has no top-level 'bound_model_hash'"
fi
if [[ -z "${CCM_BOUND_MODEL_HASH}" || "${CCM_BOUND_MODEL_HASH}" == "null" ]]; then
  reject "ccm/ccm.manifest.json 'bound_model_hash' is empty"
fi

# ---------------------------------------------------------------------------
# Cross-check: snapshot.model_hash == ccm.bound_model_hash
# ---------------------------------------------------------------------------
if [[ "${SNAPSHOT_MODEL_HASH}" != "${CCM_BOUND_MODEL_HASH}" ]]; then
  echo "  snapshot model_hash:      ${SNAPSHOT_MODEL_HASH}" >&2
  echo "  ccm/   bound_model_hash:  ${CCM_BOUND_MODEL_HASH}" >&2
  reject "mis-assembled bundle: snapshot model_hash does not match the ccm/ bound_model_hash"
fi

# ---------------------------------------------------------------------------
# Compute (and optionally assert) the snapshot's content hash
# ---------------------------------------------------------------------------
CONTENT_HASH="$(sha256sum "${SNAPSHOT}" | awk '{print $1}')"

if [[ -n "${EXPECTED_CONTENT_HASH}" && "${CONTENT_HASH}" != "${EXPECTED_CONTENT_HASH}" ]]; then
  echo "  expected content hash: ${EXPECTED_CONTENT_HASH}" >&2
  echo "  actual   content hash: ${CONTENT_HASH}" >&2
  reject "snapshot content hash does not match the expected value"
fi

echo "  snapshot:          $(basename "${SNAPSHOT}")"
echo "  model_hash:        ${SNAPSHOT_MODEL_HASH}"
echo "  bound_model_hash:  ${CCM_BOUND_MODEL_HASH}"
echo "  content sha256:    ${CONTENT_HASH}"
pass "matched bundle (snapshot model_hash == ccm/ bound_model_hash)"
