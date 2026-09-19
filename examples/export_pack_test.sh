#!/usr/bin/env bash
# Bazel sh_test for examples/export_pack.sh, the reference N-chunk CUE
# exporter, and the DRIFT GUARD for examples/06-catalogue-polyrepo's committed
# JSON (configflux-dkmm.6).
#
# Two guards live in this repository and they cover different layouts. Do not
# confuse them:
#
#   compiler/cue/export_fixtures.sh --check   the scenario corpus plus the
#       two-file and single-file example packs under examples/*/cue
#   THIS TEST                                 example 06, whose chunks live in
#       three separate repository directories under
#       examples/06-catalogue-polyrepo/repos/*/cue — a layout export_fixtures.sh
#       deliberately does not sweep
#
# What it asserts:
#   1. Re-exporting example 06's three chunks reproduces the three committed
#      JSON files BYTE-FOR-BYTE. The committed JSON is produced by
#      export_pack.sh, so byte equality is the right comparison and it is what
#      pins determinism.
#   2. Re-exporting example 00 (the two-file pack) reproduces its committed
#      JSON. That JSON was emitted by a different pipeline (2-space indent,
#      serde struct key order), so — exactly as export_fixtures.sh does for
#      examples/* — both sides are canonicalized with `jq -S .` first. This is
#      what proves export_pack.sh really does replace the documented two-file
#      recipe rather than merely resembling it.
#   3. The failure modes: a missing --definitions and an unreadable chunk are
#      usage errors (exit 2); a chunk that violates the schema is an export
#      failure (exit 1). Every one must say something on stderr.
#
# The pinned cue evaluator is supplied through runfiles (//tools:cue); `jq` must
# be on PATH, the same requirement export_fixtures.sh's examples pass carries.
set -euo pipefail

if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi
command -v jq >/dev/null 2>&1 \
  || { echo "ERROR: jq not found (required to canonicalize examples/* JSON)" >&2; exit 2; }

RUNFILES_ROOT="${TEST_SRCDIR}/_main"
abs() {
  local var="$1" path="${RUNFILES_ROOT}/${2}"
  [[ -e "${path}" ]] || { echo "ERROR: ${var} not found at ${path}" >&2; exit 2; }
  printf '%s' "${path}"
}

EXPORTER="$(abs EXPORTER_RLOCATION "${EXPORTER_RLOCATION:?}")"
SCHEMA="$(abs SCHEMA_RLOCATION "${SCHEMA_RLOCATION:?}")"
CUE_BIN="$(abs CUE_RLOCATION "${CUE_RLOCATION:?}")"
export CUE="${CUE_BIN}"

# The examples are part of THIS test's own bazel package, so resolve them from
# the script's own runfiles location — never from the workspace root. The
# examples/ tree is redistributed by copying the package wholesale into another
# directory, where it becomes a package of its own: its runfiles hold the data
# under that directory and there is no _main/examples to read. A workspace-root
# path passes in this repository and fails in every copy (configflux-k5gt).
# //examples:export_pack_relocated_test pins both layouts.
EXAMPLES_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
EX06="${EXAMPLES_DIR}/06-catalogue-polyrepo"
EX00="${EXAMPLES_DIR}/00-service-multi-env"
for root in "${EX06}" "${EX00}"; do
  [[ -d "${root}" ]] \
    || { echo "ERROR: example data not found at ${root}" >&2; exit 2; }
done
WORK="${TEST_TMPDIR:-$(mktemp -d)}"

# `cue` wants a writable system cache directory and refuses to run when neither
# $XDG_CACHE_HOME nor $HOME is set — which is exactly the bazel test sandbox. A
# real user always has one, so point cue at the per-test scratch directory
# rather than making the sandbox a special case inside the script under test.
export XDG_CACHE_HOME="${WORK}/cue-cache"
mkdir -p "${XDG_CACHE_HOME}"

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
pass() { echo "ok: $*"; }

# ---------------------------------------------------------------------------
# 1. Example 06 — the N-chunk, multi-repository pack. Byte-exact.
# ---------------------------------------------------------------------------
assert_committed_json_matches_fresh_export() {
  local out="${WORK}/ex06"
  rm -rf "${out}"
  if ! "${EXPORTER}" \
      --schema "${SCHEMA}" \
      --definitions "${EX06}/repos/catalogue/cue/00_catalogue.cue" \
      --components "${EX06}/repos/vision/cue/10_vision.cue" \
      --components "${EX06}/repos/compute/cue/10_compute.cue" \
      --components "${EX06}/repos/sorter/cue/20_sorter.cue" \
      --out "${out}" >"${WORK}/ex06.log" 2>&1; then
    cat "${WORK}/ex06.log" >&2
    fail "export_pack.sh could not export example 06"
    return
  fi
  local pairs=(
    "repos/catalogue/00_catalogue.json:00_catalogue.json"
    "repos/vision/10_vision.json:10_vision.json"
    "repos/compute/10_compute.json:10_compute.json"
    "repos/sorter/20_sorter.json:20_sorter.json"
  )
  local entry committed fresh
  for entry in "${pairs[@]}"; do
    committed="${EX06}/${entry%%:*}"
    fresh="${out}/${entry##*:}"
    if [[ ! -f "${committed}" ]]; then
      fail "committed fixture missing: ${entry%%:*}"
    elif ! cmp -s "${committed}" "${fresh}"; then
      diff -u "${committed}" "${fresh}" | head -40 >&2 || true
      fail "example 06 drift: ${entry%%:*} differs from a fresh export_pack.sh run"
    else
      pass "example 06 ${entry%%:*} is byte-identical to a fresh export"
    fi
  done
}

# Determinism: the same inputs a second time must produce the same bytes.
assert_export_is_deterministic() {
  local a="${WORK}/det_a" b="${WORK}/det_b" i
  for i in a b; do
    local out="${WORK}/det_${i}"
    rm -rf "${out}"
    "${EXPORTER}" \
      --schema "${SCHEMA}" \
      --definitions "${EX06}/repos/catalogue/cue/00_catalogue.cue" \
      --components "${EX06}/repos/vision/cue/10_vision.cue" \
      --components "${EX06}/repos/compute/cue/10_compute.cue" \
      --components "${EX06}/repos/sorter/cue/20_sorter.cue" \
      --out "${out}" >/dev/null 2>&1 || { fail "deterministic re-export run ${i} failed"; return; }
  done
  if diff -r "${a}" "${b}" >/dev/null 2>&1; then
    pass "two runs over the same inputs are byte-identical"
  else
    fail "export_pack.sh is not deterministic across two runs"
  fi
}

# ---------------------------------------------------------------------------
# 2. Example 00 — the two-file pack the documented recipe covers. Canonicalized.
# ---------------------------------------------------------------------------
assert_two_file_pack_matches_committed_json() {
  local out="${WORK}/ex00"
  rm -rf "${out}"
  if ! "${EXPORTER}" \
      --schema "${SCHEMA}" \
      --definitions "${EX00}/cue/00_definitions.cue" \
      --components "${EX00}/cue/10_components.cue" \
      --out "${out}" >"${WORK}/ex00.log" 2>&1; then
    cat "${WORK}/ex00.log" >&2
    fail "export_pack.sh could not export the two-file example 00 pack"
    return
  fi
  local name
  for name in 00_definitions 10_components; do
    if diff -u <(jq -S . "${EX00}/${name}.json") <(jq -S . "${out}/${name}.json") >/dev/null 2>&1; then
      pass "example 00 ${name}.json matches a fresh export (canonicalized)"
    else
      diff -u <(jq -S . "${EX00}/${name}.json") <(jq -S . "${out}/${name}.json") | head -40 >&2 || true
      fail "example 00 ${name}.json differs from a fresh export_pack.sh run"
    fi
  done
}

# ---------------------------------------------------------------------------
# 3. Failure modes.
# ---------------------------------------------------------------------------
# Run the exporter expecting a specific exit code and a non-empty stderr.
expect_failure() {
  local what="$1" want_rc="$2"; shift 2
  local err="${WORK}/err.$$" rc=0
  "${EXPORTER}" "$@" >/dev/null 2>"${err}" || rc=$?
  if [[ "${rc}" -ne "${want_rc}" ]]; then
    fail "${what}: expected exit ${want_rc}, got ${rc}"
  elif [[ ! -s "${err}" ]]; then
    fail "${what}: exited ${rc} but said nothing on stderr"
  else
    pass "${what}: exit ${rc} with a diagnostic on stderr"
  fi
  rm -f "${err}"
}

assert_failure_modes() {
  expect_failure "missing --definitions" 2 \
    --schema "${SCHEMA}" --out "${WORK}/n1"
  expect_failure "unreadable chunk" 2 \
    --schema "${SCHEMA}" --definitions "${WORK}/does-not-exist.cue" --out "${WORK}/n2"

  # A chunk the schema rejects: #Config is closed, so a misspelled top-level
  # key is a hard "field not allowed" error rather than a silently ignored one.
  local bad="${WORK}/bad_chunk.cue"
  cat >"${bad}" <<'BADCUE'
package configflux

chunk: #Config & {
	package: "broken_pack"
	version: "1.0.0"
	defintions: {}
}
BADCUE
  expect_failure "chunk that violates the schema" 1 \
    --schema "${SCHEMA}" --definitions "${bad}" --out "${WORK}/n3"

  # Two chunks sharing a basename would overwrite each other's output.
  expect_failure "colliding chunk basenames" 2 \
    --schema "${SCHEMA}" \
    --definitions "${EX00}/cue/00_definitions.cue" \
    --components "${EX00}/cue/10_components.cue" \
    --components "${EX06}/repos/vision/cue/10_vision.cue" \
    --components "${EX00}/cue/10_components.cue" \
    --out "${WORK}/n4"
}

assert_committed_json_matches_fresh_export
assert_export_is_deterministic
assert_two_file_pack_matches_committed_json
assert_failure_modes

if [[ "${failures}" -ne 0 ]]; then
  echo "${failures} assertion(s) failed" >&2
  exit 1
fi
echo "all export_pack.sh assertions passed"
