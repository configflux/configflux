#!/usr/bin/env bash
# Bazel sh_test wrapper for examples/*/run.sh.
#
# Each example's run.sh compiles (and optionally interprets / runs) a sample
# model using the ConfigFlux binaries. This wrapper is invoked by sh_test
# targets in examples/BUILD.bazel and is responsible for:
#
#   1. Locating the required binaries inside the runfiles tree
#   2. Locating the example's run.sh inside the runfiles tree
#   3. Redirecting the example's output directory to $TEST_TMPDIR (writable)
#   4. Running run.sh and propagating its exit status
#
# Required env vars (passed via the sh_test `env` attribute):
#
#   COMPILER_RLOCATION    Rootpath (runfiles-relative) of //compiler:compiler
#   EXAMPLE_RUN_RLOCATION Rootpath of the example's run.sh file
#
# Optional env vars (used by full-pipeline examples such as 04-fleet-edge-node):
#
#   INTERPRETER_RLOCATION Rootpath of //interpreter:interpreter
#   RUNTIME_RLOCATION     Rootpath of //runtime:runtime
#
# All rlocations are resolved to absolute paths under ${TEST_SRCDIR}/_main
# before being exported to the example script.
set -euo pipefail

if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi

if [[ -z "${COMPILER_RLOCATION:-}" ]]; then
  echo "ERROR: COMPILER_RLOCATION env var is not set" >&2
  exit 2
fi

if [[ -z "${EXAMPLE_RUN_RLOCATION:-}" ]]; then
  echo "ERROR: EXAMPLE_RUN_RLOCATION env var is not set" >&2
  exit 2
fi

RUNFILES_ROOT="${TEST_SRCDIR}/_main"
COMPILER_ABS="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
EXAMPLE_RUN_ABS="${RUNFILES_ROOT}/${EXAMPLE_RUN_RLOCATION}"

if [[ ! -x "${COMPILER_ABS}" ]]; then
  echo "ERROR: compiler binary not executable at ${COMPILER_ABS}" >&2
  exit 2
fi

if [[ ! -f "${EXAMPLE_RUN_ABS}" ]]; then
  echo "ERROR: example run.sh not found at ${EXAMPLE_RUN_ABS}" >&2
  exit 2
fi

# Redirect example output to a writable location. Runfiles trees are
# typically read-only, so the default ${EXAMPLE_DIR}/out would fail.
OUT_DIR="${TEST_TMPDIR:-$(mktemp -d)}/out"
mkdir -p "${OUT_DIR}"

export CONFIGFLUX_COMPILER="${COMPILER_ABS}"
export CONFIGFLUX_EXAMPLE_OUT_DIR="${OUT_DIR}"

if [[ -n "${INTERPRETER_RLOCATION:-}" ]]; then
  INTERPRETER_ABS="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
  if [[ ! -x "${INTERPRETER_ABS}" ]]; then
    echo "ERROR: interpreter binary not executable at ${INTERPRETER_ABS}" >&2
    exit 2
  fi
  export CONFIGFLUX_INTERPRETER="${INTERPRETER_ABS}"
fi

if [[ -n "${CFX_RLOCATION:-}" ]]; then
  CFX_ABS="${RUNFILES_ROOT}/${CFX_RLOCATION}"
  if [[ ! -x "${CFX_ABS}" ]]; then
    echo "ERROR: cfx binary not executable at ${CFX_ABS}" >&2
    exit 2
  fi
  export CONFIGFLUX_CFX="${CFX_ABS}"
fi

if [[ -n "${RUNTIME_RLOCATION:-}" ]]; then
  RUNTIME_ABS="${RUNFILES_ROOT}/${RUNTIME_RLOCATION}"
  if [[ ! -x "${RUNTIME_ABS}" ]]; then
    echo "ERROR: runtime binary not executable at ${RUNTIME_ABS}" >&2
    exit 2
  fi
  export CONFIGFLUX_RUNTIME="${RUNTIME_ABS}"
fi

# Examples 01-03 use `jq` only in their trailing echo suggestions, not in
# the pipeline itself. Example 04 produces its config with `cfx resolve` (no
# jq on the produce path) and uses jq only for the runtime-handoff envelope
# seam and metadata reads, so jq must be on PATH for that test to pass.
bash "${EXAMPLE_RUN_ABS}"
