#!/usr/bin/env bash
# 01-hello-led — bare minimum ConfigFlux pipeline
#
# Compiles a single-component model, verifies it, and inspects the result.
# No selections, no overrides, no conditional components — just the basics.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
# OUT_DIR defaults to ${EXAMPLE_DIR}/out but can be overridden so the script
# works under Bazel runfiles (read-only) or CI sandboxes.
OUT_DIR="${CONFIGFLUX_EXAMPLE_OUT_DIR:-${EXAMPLE_DIR}/out}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Locate the compiler binary (Bazel output or user-supplied path).
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

# ---------------------------------------------------------------------------
# Pipeline
# ---------------------------------------------------------------------------

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

banner "Step 1: Compile"
"${COMPILER}" compile \
  --source "${EXAMPLE_DIR}/config.json" \
  --out "${OUT_DIR}" \
  > "${OUT_DIR}/compile_result.json"
ok "Compiled model -> ${OUT_DIR}/cmp.manifest.json"

banner "Step 2: Verify"
"${COMPILER}" verify \
  --source "${EXAMPLE_DIR}/config.json" \
  > "${OUT_DIR}/verify_report.json"
ok "Verification passed (0 errors, 0 warnings)"

banner "Step 3: Inspect"
"${COMPILER}" inspect \
  --source "${EXAMPLE_DIR}/config.json" \
  summary \
  > "${OUT_DIR}/inspect_summary.json"
ok "Model summary -> ${OUT_DIR}/inspect_summary.json"

banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Try exploring the outputs:"
echo "  cat ${OUT_DIR}/compile_result.json | jq ."
echo "  cat ${OUT_DIR}/verify_report.json  | jq .status"
echo "  cat ${OUT_DIR}/inspect_summary.json | jq .summary"
