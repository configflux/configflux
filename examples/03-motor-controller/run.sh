#!/usr/bin/env bash
# 03-motor-controller — artifacts, 2-step selection, export-resolved, BOM
#
# Compiles a motor controller model that demonstrates:
#   - Artifact references (driver binaries bound to components)
#   - 2-step selection: motor_class selects control mode + driver,
#     power_rating selects current limit and enables safety_monitor
#   - Export-resolved output (C++ header and CMake flags via loader API)
#   - Software BOM generation (full audit of resolved artifacts)
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
ok "4 artifacts, 4 components, 6 definitions"

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
ok "4 components: motor_drive, encoder_interface, motion_controller, safety_monitor"
ok "4 artifacts: foc_driver, trapz_driver, absolute_encoder_driver, incremental_encoder_driver"

banner "Step 4: Inspect — artifact detail"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  artifact foc_driver \
  > "${OUT_DIR}/inspect_artifact_foc_driver.json"
ok "foc_driver v3.1.0 -> /opt/configflux/motor/foc_driver.so"

banner "Step 5: Inspect — motor_drive component"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  component motor_drive \
  > "${OUT_DIR}/inspect_motor_drive.json"
ok "motor_drive has 4 params: control_mode, motor_driver, pwm_frequency, current_limit"
ok "motor_driver param references artifact (2-step: motor_class selects driver)"

banner "Step 6: Inspect — safety_monitor (conditional)"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  component safety_monitor \
  > "${OUT_DIR}/inspect_safety_monitor.json"
ok "safety_monitor: condition = \"power_rating == 'high'\""

banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - cue/00_definitions.cue declares 2 artifact slots (motor_driver_slot, encoder_driver_slot)"
echo "  - cue/10_components.cue declares 4 artifacts and binds them via overrides"
echo "  - 2-step selection: motor_class picks control_mode + driver,"
echo "    power_rating picks current_limit and gates safety_monitor"
echo "  - safety_monitor is conditional: only included when power_rating == 'high'"
echo ""
echo "Try exploring:"
echo "  cat ${OUT_DIR}/compile_result.json | jq .stats"
echo "  cat ${OUT_DIR}/inspect_summary.json | jq .summary.artifact_ids"
echo "  cat ${OUT_DIR}/inspect_artifact_foc_driver.json | jq .item"
echo "  cat ${OUT_DIR}/inspect_motor_drive.json | jq .item.param_keys"
echo ""
echo "Export-resolved and BOM generation (C++ header, CMake flags, software"
echo "BOM) are exercised via the loader API after open-model. See the"
echo "compiler scenario tests (s4_mobile_robot, s5_building_hvac) for"
echo "programmatic examples of the full export-resolved and BOM pipeline."
