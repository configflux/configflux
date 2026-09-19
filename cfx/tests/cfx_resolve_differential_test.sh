#!/usr/bin/env bash
# SPDX-License-Identifier: BUSL-1.1
#
# sh_test wrapper for the `cfx resolve` differential/determinism/exit-code/golden
# validation (configflux-2awb.2 / CFX-1, ADR-0042 §5). Resolves the real product
# binaries and scenario fixtures from runfiles, then hands them to the Python
# driver, which does the byte-identical comparison against the interpreter
# envelope path. Mirrors the runfiles-resolution pattern of the examples/ e2e
# sh_tests.
set -euo pipefail

if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; run under bazel test" >&2
  exit 2
fi
RUNFILES_ROOT="${TEST_SRCDIR}/_main"

resolve() {
  local rloc="$1"
  local path="${RUNFILES_ROOT}/${rloc}"
  if [[ ! -e "${path}" ]]; then
    echo "ERROR: runfile not found: ${rloc}" >&2
    exit 2
  fi
  echo "${path}"
}

require_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "ERROR: ${name} env var not set" >&2
    exit 2
  fi
}

for v in \
  COMPILER_RLOCATION INTERPRETER_RLOCATION CFX_RLOCATION \
  S1_DEFS_RLOCATION S1_COMPONENTS_RLOCATION \
  S5_DEFS_RLOCATION S5_COMPONENTS_RLOCATION \
  REQUIRES_CATALOGUE_RLOCATION REQUIRES_BINDINGS_RLOCATION \
  REQUIRES_SERVICES_RLOCATION \
  GOLDEN_HAPPY_RLOCATION; do
  require_var "${v}"
done

export COMPILER="$(resolve "${COMPILER_RLOCATION}")"
export INTERPRETER="$(resolve "${INTERPRETER_RLOCATION}")"
export CFX="$(resolve "${CFX_RLOCATION}")"
export S1_DEFS="$(resolve "${S1_DEFS_RLOCATION}")"
export S1_COMPONENTS="$(resolve "${S1_COMPONENTS_RLOCATION}")"
export S5_DEFS="$(resolve "${S5_DEFS_RLOCATION}")"
export S5_COMPONENTS="$(resolve "${S5_COMPONENTS_RLOCATION}")"
export REQUIRES_CATALOGUE="$(resolve "${REQUIRES_CATALOGUE_RLOCATION}")"
export REQUIRES_BINDINGS="$(resolve "${REQUIRES_BINDINGS_RLOCATION}")"
export REQUIRES_SERVICES="$(resolve "${REQUIRES_SERVICES_RLOCATION}")"
GOLDEN_HAPPY="$(resolve "${GOLDEN_HAPPY_RLOCATION}")"
export GOLDEN_DIR="$(dirname "${GOLDEN_HAPPY}")"

HELPER="$(resolve "cfx/tests/cfx_resolve_differential.py")"
exec python3 -u "${HELPER}"
