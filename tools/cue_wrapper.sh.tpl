#!/usr/bin/env bash
# Runnable wrapper around the hermetic, Bazel-pinned `cue` binary
# (ADR-0021 Decision 5). `bazel run //tools:cue -- <args>` execs the pinned
# evaluator; other Bazel targets consume it as a tool/data dependency so the
# CUE authoring front-end never depends on whatever `cue` is on $PATH.
#
# This file is a TEMPLATE: //tools:cue_wrapper_gen substitutes @@CUE_RLOCATION@@
# with the runfiles path of the platform-selected cue binary (via the genrule's
# `$(rlocationpath :cue_binary)`), so the path is baked in at build time and the
# wrapper works identically under `bazel run`, as a `tools=`/`data=` dep, and
# from an unpacked runfiles tree — no dependence on an `env` attribute that only
# fires under `bazel run`.
set -euo pipefail

# --- rules_shell runfiles bootstrap (standard preamble) ----------------------
# shellcheck disable=SC1090,SC1091
if [[ -z "${RUNFILES_DIR:-}" && -z "${RUNFILES_MANIFEST_FILE:-}" ]]; then
  if [[ -f "$0.runfiles_manifest" ]]; then
    export RUNFILES_MANIFEST_FILE="$0.runfiles_manifest"
  elif [[ -d "$0.runfiles" ]]; then
    export RUNFILES_DIR="$0.runfiles"
  fi
fi
if [[ -f "${RUNFILES_DIR:-/dev/null}/bazel_tools/tools/bash/runfiles/runfiles.bash" ]]; then
  source "${RUNFILES_DIR}/bazel_tools/tools/bash/runfiles/runfiles.bash"
elif [[ -f "${RUNFILES_MANIFEST_FILE:-/dev/null}" ]]; then
  source "$(grep -m1 "^bazel_tools/tools/bash/runfiles/runfiles.bash " \
    "${RUNFILES_MANIFEST_FILE}" | cut -d ' ' -f 2-)"
else
  echo "ERROR: cannot locate runfiles bootstrap" >&2
  exit 1
fi
# -----------------------------------------------------------------------------

cue_bin="$(rlocation '@@CUE_RLOCATION@@')"
if [[ -z "${cue_bin}" || ! -x "${cue_bin}" ]]; then
  echo "ERROR: pinned cue binary not found in runfiles (@@CUE_RLOCATION@@)" >&2
  exit 1
fi

exec "${cue_bin}" "$@"
