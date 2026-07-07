#!/usr/bin/env bash
# tools/lib/affected_targets.sh — resolve the test targets affected by a change
# set, for the local gate's `affected` test strategy (ADR-0026).
#
# The auto inner loop runs only the tests reachable from the changed files via
#   kind(test, rdeps(//..., set(<changed files>)))
# instead of the full //... suite, keeping iteration fast while still catching
# breakage in the edited area (the configflux-dp1c class of failure).
#
# Contract — configflux_affected_test_targets prints, one per line:
#   * the affected `//...:..._test` labels (sorted, unique), or
#   * the single sentinel "__ALL__" when the affected set cannot be trusted
#     (no changed file resolves to a target, or the query itself failed) — the
#     caller MUST then fall back to `test //...` so we never silently under-test,
#   * nothing at all when the query succeeded but no test depends on the change
#     (the build step already validated compilation).
#
# Depends on configflux_gate_run_bazel_guarded (tools/lib/gate_common.sh), which
# the caller is expected to have sourced.

AFFECTED_TARGETS_ALL_SENTINEL="__ALL__"

# configflux_affected_test_targets <resource_guard> <output_user_root> \
#                                  <workspace_root> [changed_file ...]
configflux_affected_test_targets() {
  local resource_guard="$1"
  local output_user_root="$2"
  local workspace_root="$3"
  shift 3

  local -a existing=()
  local f
  for f in "$@"; do
    [[ -n "${f}" ]] || continue
    if [[ -e "${workspace_root}/${f}" ]]; then
      existing+=("${f}")
    fi
  done

  # No changed file maps to anything on disk (e.g. pure deletions) — cannot
  # compute a trustworthy affected set; fall back to the full suite.
  if [[ ${#existing[@]} -eq 0 ]]; then
    printf '%s\n' "${AFFECTED_TARGETS_ALL_SENTINEL}"
    return 0
  fi

  local query
  query="kind(test, rdeps(//..., set(${existing[*]})))"

  local out rc
  set +e
  out="$(
    cd "${workspace_root}" && \
      configflux_gate_run_bazel_guarded \
        "${resource_guard}" \
        "run_local_task_gate" \
        "affected_query" \
        "${output_user_root}" \
        query "${query}" --keep_going --noshow_progress --output=label 2>/dev/null
  )"
  rc=$?
  set -e

  # Exit 0 = clean, 3 = partial success under --keep_going (some inputs were not
  # targets — expected for files outside the build graph). Any other code means
  # the query itself broke; fall back to the full suite rather than trust a
  # possibly-empty result.
  if [[ "${rc}" -ne 0 && "${rc}" -ne 3 ]]; then
    printf '%s\n' "${AFFECTED_TARGETS_ALL_SENTINEL}"
    return 0
  fi

  # Emit only target labels (guards against any non-label noise on stdout).
  printf '%s\n' "${out}" | grep -E '^//' | sort -u
  return 0
}
