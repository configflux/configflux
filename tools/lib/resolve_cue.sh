#!/usr/bin/env bash
# Shared resolver for the pinned `cue` evaluator (ADR-0021 Decision 5).
#
# Hermetic-first, PATH demoted. Sourced by compiler/cue/export_fixtures.sh and
# compiler/cue/validate_fixtures.sh so both consume the Bazel-pinned cue by
# default instead of whatever `cue` happens to be on $PATH.
#
# Resolution order:
#   1. $CUE set explicitly  -> honored, but version-asserted. This is the ONLY
#      way PATH (or any ad-hoc binary) is used: the caller opts in. CI sets
#      CUE=/tmp/cue-bin/cue from its own sha256-verified download; developers
#      can point at a local build. Every resolution (override or hermetic) is
#      asserted against the CUE_VERSION pin in tools/cue_toolchain.bzl — the
#      single source of truth — so an override picks WHICH binary supplies the
#      pinned evaluator, never a different evaluator version.
#   2. Bazel-pinned binary  -> the extracted `cue` from @cue_linux_<arch>,
#      located via `bazel cquery` against //tools:cue_binary. No download: the
#      archive was fetched at build time; cquery only reports its on-disk path.
#   3. Hard failure         -> if $CUE is unset AND Bazel cannot produce the
#      pinned binary, exit non-zero. We deliberately do NOT silently fall back
#      to PATH cue, because an unpinned evaluator can change model_hash
#      (ADR-0027). The remedy is printed: set $CUE or run via Bazel.
#
# Usage:
#   source "$REPO/tools/lib/resolve_cue.sh"
#   CUE="$(configflux_resolve_cue)" || exit $?
#
# Honors $CONFIGFLUX_BAZEL for the bazel entrypoint (e.g. a resource_guard-
# wrapped invocation); defaults to `bazel` on PATH for cquery, which is a
# read-only metadata query and does not run a build under load.

# Assert that a resolved cue binary matches the repository pin (configflux
# gate hardening: a wrong-version evaluator is rejected, not warned about).
# $1 = cue binary path, $2 = repo root. Returns 0 on match, 2 otherwise.
configflux_assert_cue_version() {
  local cue_bin="$1" repo_root="$2" expected actual
  expected="$(sed -n 's/^CUE_VERSION = "\([0-9][0-9.]*\)".*/\1/p' \
    "${repo_root}/tools/cue_toolchain.bzl" | head -n1)"
  if [[ -z "${expected}" ]]; then
    echo "ERROR: could not read CUE_VERSION from tools/cue_toolchain.bzl (the pin's source of truth)" >&2
    return 2
  fi
  actual="$("${cue_bin}" version 2>/dev/null \
    | sed -n 's/^cue version v\([0-9][0-9.]*\).*/\1/p' | head -n1)"
  if [[ -z "${actual}" ]]; then
    echo "ERROR: could not determine the version of '${cue_bin}' (\`cue version\` produced no parsable output)" >&2
    return 2
  fi
  if [[ "${actual}" != "${expected}" ]]; then
    cat >&2 <<EOF
ERROR: cue version mismatch: '${cue_bin}' reports v${actual}, but the
  repository pin (CUE_VERSION in tools/cue_toolchain.bzl) is v${expected}.
  An unpinned evaluator can change model_hash (ADR-0027), so this is rejected.
  Remedy: unset \$CUE to use the hermetic Bazel-pinned binary, or point \$CUE
  at a cue v${expected} build.
EOF
    return 2
  fi
  return 0
}

configflux_resolve_cue() {
  local repo_root bazel_bin cue_path
  repo_root="${BUILD_WORKSPACE_DIRECTORY:-}"
  if [[ -z "${repo_root}" ]]; then
    # Derive the workspace root from this file's location: tools/lib/ -> ../..
    repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  fi

  # (1) Explicit override wins — the demoted, opt-in path, version-asserted.
  if [[ -n "${CUE:-}" ]]; then
    if command -v "${CUE}" >/dev/null 2>&1 || [[ -x "${CUE}" ]]; then
      configflux_assert_cue_version "${CUE}" "${repo_root}" || return $?
      printf '%s\n' "${CUE}"
      return 0
    fi
    echo "ERROR: \$CUE is set to '${CUE}' but it is not an executable cue binary" >&2
    return 2
  fi

  # (2) Hermetic Bazel-pinned binary.

  bazel_bin="${CONFIGFLUX_BAZEL:-bazel}"
  if command -v "${bazel_bin%% *}" >/dev/null 2>&1; then
    # Materialize the pinned binary, then resolve its absolute path. If the
    # @cue_linux_* repo is not yet fetched, Bazel fetches it here (the only
    # sanctioned download path — the pinned http_archive); a sandbox denial of
    # that fetch surfaces as a build/cquery failure, which we treat as a hard
    # error rather than working around.
    local exec_root rel_path
    if ( cd "${repo_root}" && ${bazel_bin} build //tools:cue_binary >/dev/null 2>&1 ); then
      # `cquery --output=files` prints the File path relative to the execution
      # root (e.g. external/+cue_repos+cue_linux_amd64/cue), so join it with
      # `bazel info execution_root` to get an absolute path.
      exec_root="$( cd "${repo_root}" && ${bazel_bin} info execution_root 2>/dev/null )"
      rel_path="$( cd "${repo_root}" && \
        ${bazel_bin} cquery --output=files //tools:cue_binary 2>/dev/null \
        | head -n1 )"
      if [[ -n "${exec_root}" && -n "${rel_path}" ]]; then
        [[ "${rel_path}" = /* ]] && cue_path="${rel_path}" || cue_path="${exec_root}/${rel_path}"
        if [[ -x "${cue_path}" ]]; then
          # Belt-and-braces: the hermetic binary is the pin by construction,
          # but asserting uniformly catches a stale/poisoned bazel cache.
          configflux_assert_cue_version "${cue_path}" "${repo_root}" || return $?
          printf '%s\n' "${cue_path}"
          return 0
        fi
      fi
    fi
  fi

  # (3) No explicit override and no hermetic binary -> fail closed.
  cat >&2 <<'EOF'
ERROR: could not resolve the pinned cue binary.
  The CUE authoring scripts use the hermetic, Bazel-pinned cue by default
  (ADR-0021 Decision 5) and do NOT fall back to a $PATH cue, because an
  unpinned evaluator can change model_hash (ADR-0027).
  Remedy (pick one):
    - run the script through Bazel, or ensure `bazel` is on PATH so it can
      materialize //tools:cue_binary; or
    - set $CUE to a specific, trusted cue binary, e.g.
        CUE=/path/to/cue compiler/cue/validate_fixtures.sh
EOF
  return 2
}
