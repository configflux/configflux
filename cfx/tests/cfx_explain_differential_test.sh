#!/usr/bin/env bash
# SPDX-License-Identifier: BUSL-1.1
#
# sh_test wrapper for the `cfx explain` golden/JSON-passthrough/exit-code/
# determinism validation (configflux-2awb.3 / CFX-2, ADR-0042 §3 + ADR-0031).
# Resolves the real product binaries and the committed cross-facet
# `s_labeled_mus` scenario fixtures from runfiles, then hands them to the Python
# driver, which does the byte-identical comparison against the interpreter
# `explain` envelope path. Mirrors the runfiles-resolution pattern of the
# cfx options differential sh_test.
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
  MUS_DEFS_RLOCATION MUS_COMPONENTS_RLOCATION \
  GOLDEN_CPU_COOLING_RLOCATION; do
  require_var "${v}"
done

export COMPILER="$(resolve "${COMPILER_RLOCATION}")"
export INTERPRETER="$(resolve "${INTERPRETER_RLOCATION}")"
export CFX="$(resolve "${CFX_RLOCATION}")"
export MUS_DEFS="$(resolve "${MUS_DEFS_RLOCATION}")"
export MUS_COMPONENTS="$(resolve "${MUS_COMPONENTS_RLOCATION}")"
GOLDEN_CPU_COOLING="$(resolve "${GOLDEN_CPU_COOLING_RLOCATION}")"
export GOLDEN_DIR="$(dirname "${GOLDEN_CPU_COOLING}")"

HELPER="$(resolve "cfx/tests/cfx_explain_differential.py")"
exec python3 -u "${HELPER}"
