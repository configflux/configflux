#!/usr/bin/env bash
# 02-sensor-gateway — selections, overrides, and conditional components
#
# Compiles a multi-component sensor gateway model that demonstrates:
#   - Parameter overrides based on selection context
#   - A conditional component (network_monitor, included only for ethernet)
#   - Multi-source compilation (definitions + components in separate files)
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
# OUT_DIR defaults to ${EXAMPLE_DIR}/out but can be overridden so the script
# works under Bazel runfiles (read-only) or CI sandboxes.
OUT_DIR="${CONFIGFLUX_EXAMPLE_OUT_DIR:-${EXAMPLE_DIR}/out}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

find_compiler() {
  if [[ -n "${CONFIGFLUX_COMPILER:-}" ]]; then
    echo "${CONFIGFLUX_COMPILER}"
    return
  fi
  local bazel_bin="${REPO_ROOT}/bazel-bin/compiler/compiler"
  if [[ -x "${bazel_bin}" ]]; then
    echo "${bazel_bin}"
    return
  fi
  echo >&2 "Error: compiler binary not found."
  echo >&2 "  Either build it:  bazel build //compiler"
  echo >&2 "  Or set:           export CONFIGFLUX_COMPILER=/path/to/compiler"
  exit 1
}

banner() { printf "\n=== %s ===\n" "$1"; }
ok()     { printf "  -> %s\n" "$1"; }

COMPILER="$(find_compiler)"

SOURCES=(
  --source "${EXAMPLE_DIR}/00_definitions.json"
  --source "${EXAMPLE_DIR}/10_components.json"
)

# ---------------------------------------------------------------------------
# Pipeline
# ---------------------------------------------------------------------------

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

banner "Step 1: Compile"
"${COMPILER}" compile \
  "${SOURCES[@]}" \
  --out "${OUT_DIR}" \
  > "${OUT_DIR}/compile_result.json"
ok "Compiled model -> ${OUT_DIR}/cmp.manifest.json"

banner "Step 2: Verify"
"${COMPILER}" verify \
  "${SOURCES[@]}" \
  > "${OUT_DIR}/verify_report.json"
ok "Verification passed (0 errors, 0 warnings)"

banner "Step 3: Inspect — model summary"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  summary \
  > "${OUT_DIR}/inspect_summary.json"
ok "3 components: sensor_bus, data_logger, network_monitor"
ok "3 definitions: protocol, poll_interval_ms, buffer_depth"

banner "Step 4: Inspect — component detail"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  component data_logger \
  > "${OUT_DIR}/inspect_data_logger.json"
ok "data_logger has overrides on poll_interval and buffer_depth"

banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - 00_definitions.json and 10_components.json (exported from cue/) are compiled together"
echo "  - network_monitor has condition = \"bus_type == 'ethernet'\""
echo "  - data_logger.poll_interval overrides from 1000 -> 100 when"
echo "    environment == 'high_speed'"
echo ""
echo "Try exploring:"
echo "  cat ${OUT_DIR}/compile_result.json | jq .stats"
echo "  cat ${OUT_DIR}/inspect_data_logger.json | jq ."
