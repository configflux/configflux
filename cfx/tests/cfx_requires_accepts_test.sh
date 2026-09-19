#!/usr/bin/env bash
# SPDX-License-Identifier: BUSL-1.1
#
# sh_test wrapper for the `requires` / `accepts` / `derive` end-to-end check
# (configflux-secb.5 / ADR-0057 §D4). Resolves the real compiler and cfx
# binaries plus the committed three-unit `s_requires_accepts` pack from
# runfiles, then hands them to the Python driver. Mirrors the
# runfiles-resolution pattern of the cfx options catalogue-binding sh_test.
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
  PACK_CATALOGUE_RLOCATION PACK_BINDINGS_RLOCATION PACK_COMPONENTS_RLOCATION \
  GOLDEN_OPTIONS_SITE_RLOCATION GOLDEN_EXPLAIN_DERIVE_RLOCATION \
  GOLDEN_EXPLAIN_ACCEPTS_RLOCATION; do
  require_var "${v}"
done

export COMPILER="$(resolve "${COMPILER_RLOCATION}")"
export CFX="$(resolve "${CFX_RLOCATION}")"
export PACK_CATALOGUE="$(resolve "${PACK_CATALOGUE_RLOCATION}")"
export PACK_BINDINGS="$(resolve "${PACK_BINDINGS_RLOCATION}")"
export PACK_COMPONENTS="$(resolve "${PACK_COMPONENTS_RLOCATION}")"
export GOLDEN_OPTIONS_SITE="$(resolve "${GOLDEN_OPTIONS_SITE_RLOCATION}")"
export GOLDEN_EXPLAIN_DERIVE="$(resolve "${GOLDEN_EXPLAIN_DERIVE_RLOCATION}")"
export GOLDEN_EXPLAIN_ACCEPTS="$(resolve "${GOLDEN_EXPLAIN_ACCEPTS_RLOCATION}")"

HELPER="$(resolve "cfx/tests/cfx_requires_accepts.py")"
exec python3 -u "${HELPER}"
