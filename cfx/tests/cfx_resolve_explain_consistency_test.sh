#!/usr/bin/env bash
# SPDX-License-Identifier: BUSL-1.1
#
# sh_test wrapper for the `cfx resolve` / `cfx explain` satisfiability-agreement
# check (configflux-sc69). Resolves the real compiler / cfx binaries and the
# committed S1 water_pump smoke fixtures from runfiles, then hands them to the
# Python driver, which asserts the two verbs never contradict each other on
# satisfiability for identical input. Mirrors the runfiles-resolution pattern of
# the cfx explain differential sh_test.
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
  COMPILER_RLOCATION CFX_RLOCATION \
  S1_DEFS_RLOCATION S1_COMPONENTS_RLOCATION; do
  require_var "${v}"
done

export COMPILER="$(resolve "${COMPILER_RLOCATION}")"
export CFX="$(resolve "${CFX_RLOCATION}")"
export S1_DEFS="$(resolve "${S1_DEFS_RLOCATION}")"
export S1_COMPONENTS="$(resolve "${S1_COMPONENTS_RLOCATION}")"

HELPER="$(resolve "cfx/tests/cfx_resolve_explain_consistency.py")"
exec python3 -u "${HELPER}"
