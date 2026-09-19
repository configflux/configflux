#!/usr/bin/env bash
# Bazel sh_test: examples/export_pack_test.sh must resolve the example data it
# guards relative to ITS OWN bazel package, never to the workspace root
# (configflux-k5gt).
#
# Why this guard exists
# ---------------------
# The examples/ tree is redistributed by copying the package wholesale into
# another directory, where it is built as a bazel package of its own. The copy's
# runfiles therefore hold the example data under THAT directory — there is no
# _main/examples for it to read. A drift guard that hardcodes the workspace-root
# path passes in this repository and fails in every relocated copy, which is
# precisely the regression this test pins: it went unnoticed until a copy was
# rebuilt, because nothing in the repository exercised the relocated layout.
#
# How it works
# ------------
# Build a synthetic runfiles tree under $TEST_TMPDIR that holds the package at a
# DIFFERENT path and deliberately has no _main/examples at all, then run the real
# export_pack_test.sh inside it with the rlocation environment a relocated copy
# receives. It must still pass, and it must still say so.
#
# Everything the guard reaches outside its own package — the pinned cue
# evaluator and the compiler schema — comes from absolute labels whose rootpaths
# are identical in both layouts, so the synthetic tree links those top-level
# directories back to the real runfiles rather than copying them. The cue wrapper
# finds its own binary through $RUNFILES_DIR, which keeps pointing at the real
# tree, so relocating $TEST_SRCDIR does not disturb it.
set -euo pipefail

if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi
command -v jq >/dev/null 2>&1 \
  || { echo "ERROR: jq not found (export_pack_test.sh requires it)" >&2; exit 2; }

RUNFILES_ROOT="${TEST_SRCDIR}/_main"
PKG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
INNER_NAME="export_pack_test.sh"
[[ -f "${PKG_DIR}/${INNER_NAME}" ]] \
  || { echo "ERROR: script under test not found at ${PKG_DIR}/${INNER_NAME}" >&2; exit 2; }

WORK="${TEST_TMPDIR:-$(mktemp -d)}"
FAKE="${WORK}/relocated.runfiles"
FAKE_MAIN="${FAKE}/_main"
# Any path other than examples/ proves the invariant; this one is nested so the
# copy is neither a prefix nor a suffix of the layout it must stop assuming.
RELOC_REL="redistributed/pack/examples"
RELOC_PKG="${FAKE_MAIN}/${RELOC_REL}"

rm -rf "${FAKE}"
mkdir -p "${RELOC_PKG}"
# -L: runfiles entries are symlinks into the source tree; the relocated package
# must be real files at a real, different path.
cp -RL "${PKG_DIR}/." "${RELOC_PKG}/"

for rl in "${SCHEMA_RLOCATION:?}" "${CUE_RLOCATION:?}"; do
  top="${rl%%/*}"
  [[ -e "${FAKE_MAIN}/${top}" ]] || ln -s "${RUNFILES_ROOT}/${top}" "${FAKE_MAIN}/${top}"
  [[ -e "${FAKE_MAIN}/${rl}" ]] \
    || { echo "ERROR: ${rl} did not resolve in the synthetic tree" >&2; exit 2; }
done

# Positive control. If the workspace-root layout were still reachable from the
# synthetic tree this test would pass without proving anything.
if [[ -e "${FAKE_MAIN}/examples" ]]; then
  echo "ERROR: harness bug — ${FAKE_MAIN}/examples must not exist" >&2
  exit 2
fi

INNER_TMP="${WORK}/inner-tmp"
mkdir -p "${INNER_TMP}"
LOG="${WORK}/inner.log"
rc=0
env \
  TEST_SRCDIR="${FAKE}" \
  TEST_TMPDIR="${INNER_TMP}" \
  RUNFILES_DIR="${TEST_SRCDIR}" \
  EXPORTER_RLOCATION="${RELOC_REL}/export_pack.sh" \
  SCHEMA_RLOCATION="${SCHEMA_RLOCATION}" \
  CUE_RLOCATION="${CUE_RLOCATION}" \
  bash "${RELOC_PKG}/${INNER_NAME}" >"${LOG}" 2>&1 || rc=$?

if [[ "${rc}" -ne 0 ]]; then
  cat "${LOG}" >&2
  echo "FAIL: ${INNER_NAME} exited ${rc} with its package relocated to ${RELOC_REL}" >&2
  echo "      it must resolve its example roots relative to its own package" >&2
  exit 1
fi
# Exit 0 alone is not enough: a guard that silently skipped its assertions would
# also exit 0. It has to have reached its own success line.
if ! grep -q 'all export_pack.sh assertions passed' "${LOG}"; then
  cat "${LOG}" >&2
  echo "FAIL: ${INNER_NAME} exited 0 but never reported its assertions" >&2
  exit 1
fi
echo "ok: ${INNER_NAME} passes with its package relocated to ${RELOC_REL}"
echo "all relocation assertions passed"
