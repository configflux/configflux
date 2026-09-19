#!/usr/bin/env bash
# SPDX-License-Identifier: BUSL-1.1
#
# sh_test wrapper for the `cfx options` catalogue-binding check
# (configflux-secb.4 / ADR-0057 §D3). Resolves the real compiler and cfx
# binaries plus the committed `s_catalogue_binding` pack from runfiles, then
# hands them to the Python driver. Mirrors the runfiles-resolution pattern of
# the cfx explain facet-equality sh_test.
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
  PACK_DEFS_RLOCATION PACK_COMPONENTS_RLOCATION \
  GOLDEN_OPTIONS_RLOCATION; do
  require_var "${v}"
done

export COMPILER="$(resolve "${COMPILER_RLOCATION}")"
export CFX="$(resolve "${CFX_RLOCATION}")"
export PACK_DEFS="$(resolve "${PACK_DEFS_RLOCATION}")"
export PACK_COMPONENTS="$(resolve "${PACK_COMPONENTS_RLOCATION}")"
export GOLDEN_OPTIONS="$(resolve "${GOLDEN_OPTIONS_RLOCATION}")"

HELPER="$(resolve "cfx/tests/cfx_options_catalogue_binding.py")"
exec python3 -u "${HELPER}"
