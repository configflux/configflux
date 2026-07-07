#!/usr/bin/env bash
# run_standalone.sh — run the SAME service binary standalone, no compose.
#
# This is the debugging path: a developer points the service at the `local`
# named-environment bundle and runs it directly on their machine. It is the
# identical service/app.py used in the compose stack; only CONFIGFLUX_SNAPSHOT
# changes (compose points it at the robot-alpha bundle, this points it at the
# local one). That symmetry is the whole point — the same consumer code runs in
# the fleet and on the bench.
#
# Prerequisite: run.sh has produced the bundles under OUT_DIR/bundles/. This
# script does not recompile or re-resolve; it consumes what run.sh built.
#
# Usage:
#   ./run_standalone.sh [scope-root]
# where scope-root is vision_service (default) or telemetry_service.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="${CONFIGFLUX_EXAMPLE_OUT_DIR:-${EXAMPLE_DIR}/out}"

SCOPE_ROOT="${1:-vision_service}"
ENV_NAME="local"

banner() { printf "\n=== %s ===\n" "$1"; }

BUNDLE_DIR="${OUT_DIR}/bundles/${ENV_NAME}--${SCOPE_ROOT}"
if [[ ! -d "${BUNDLE_DIR}" ]]; then
  echo "ERROR: bundle not found: ${BUNDLE_DIR}" >&2
  echo "  Run ./run.sh first to compile, resolve, and assemble the bundles." >&2
  exit 1
fi

# The bundle root holds exactly one resolve_result.*.json (bundle convention).
SNAPSHOT="$(find "${BUNDLE_DIR}" -maxdepth 1 -name 'resolve_result.*.json' | head -n1)"
if [[ -z "${SNAPSHOT}" ]]; then
  echo "ERROR: no resolve_result.*.json in ${BUNDLE_DIR}" >&2
  exit 1
fi

# Pin the lineage we expect: the resolve_hash baked into the snapshot itself.
# A real deploy pipeline would carry this value out-of-band; here we read it
# back so the standalone run exercises the same fail-closed pin as production.
EXPECT_RH="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["resolve_hash"])' "${SNAPSHOT}")"

banner "Standalone debug run: ${ENV_NAME} / ${SCOPE_ROOT}"
echo "  snapshot: ${SNAPSHOT}"
CONFIGFLUX_SNAPSHOT="${SNAPSHOT}" \
CONFIGFLUX_ENV_LABEL="${ENV_NAME} (standalone)" \
CONFIGFLUX_EXPECT_RESOLVE_HASH="${EXPECT_RH}" \
  python3 "${EXAMPLE_DIR}/service/app.py"
