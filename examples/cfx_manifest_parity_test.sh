#!/usr/bin/env bash
# Parity test for `cfx resolve --manifest ... --all` against the reference
# envelope path (configflux-dkmm.4, ADR-0059 D2/D6/M1).
#
# ADR-0059 makes the environment manifest a first-class `cfx` input, but it
# keeps examples/resolve_environment.sh — the script that unfolds a named
# environment through the raw interpreter `open -> init-selection-state ->
# select x N -> resolve` chain — as the ENVELOPE-PATH ORACLE (D6). That is what
# this test enforces: the new in-process matrix and the old envelope loop must
# produce the same cells, in the same places, with the same bytes. If they ever
# disagree, one of them is wrong about what a named environment means, and the
# product has two answers to the same question.
#
# Over examples/05-compose-fleet (two environments x two service scopes), it
# asserts:
#
#   1. LAYOUT: both paths write the same set of snapshot paths, relative to
#      their own output root — <environment>/<scope-root>/resolve_result.*.json.
#   2. BYTES: every snapshot pair is byte-identical (`cmp`). The interpreter
#      writes `serde_json::to_vec(response)` + '\n'; `cfx` writes the same bytes
#      through render::snapshot_bytes, so any drift in the resolution itself
#      shows up here rather than in a customer's delivery bundle.
#   3. TREE: `diff -r` over the two trees is clean once `generated/` is
#      excluded. The exclusion is the ONE deliberate difference: a `cfx` cell
#      writes what a single `cfx resolve` writes — the snapshot PLUS the export
#      profile's early-binding files — while the reference script drives only
#      the resolve verb and writes the snapshot alone.
#
# Binaries, the example sources, the resolver, and the example manifest are
# located via the runfiles tree (rootpath env vars set by the sh_test `env`
# attribute), mirroring examples/resolve_environment_test.sh. Requires `jq` on
# the host PATH (the reference resolver depends on it).
set -euo pipefail

# ---------------------------------------------------------------------------
# Runfiles resolution
# ---------------------------------------------------------------------------
if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi

RUNFILES_ROOT="${TEST_SRCDIR}/_main"

require_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "ERROR: ${name} env var is not set" >&2
    exit 2
  fi
}

require_var COMPILER_RLOCATION
require_var INTERPRETER_RLOCATION
require_var CFX_RLOCATION
require_var RESOLVER_RLOCATION
require_var MANIFEST_RLOCATION
require_var DEFS_RLOCATION
require_var COMPONENTS_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
INTERPRETER="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
CFX="${RUNFILES_ROOT}/${CFX_RLOCATION}"
RESOLVER="${RUNFILES_ROOT}/${RESOLVER_RLOCATION}"
MANIFEST="${RUNFILES_ROOT}/${MANIFEST_RLOCATION}"
DEFS="${RUNFILES_ROOT}/${DEFS_RLOCATION}"
COMPONENTS="${RUNFILES_ROOT}/${COMPONENTS_RLOCATION}"

for bin in "${COMPILER}" "${INTERPRETER}" "${CFX}"; do
  if [[ ! -x "${bin}" ]]; then
    echo "ERROR: binary not executable at ${bin}" >&2
    exit 2
  fi
done
for f in "${RESOLVER}" "${MANIFEST}" "${DEFS}" "${COMPONENTS}"; do
  if [[ ! -f "${f}" ]]; then
    echo "ERROR: file not found at ${f}" >&2
    exit 2
  fi
done

command -v jq >/dev/null 2>&1 || { echo "ERROR: jq not found on PATH" >&2; exit 2; }

fail() {
  echo "ASSERT FAILED: $*" >&2
  exit 1
}

step() { printf '\n=== %s ===\n' "$1"; }

WORK="${TEST_TMPDIR:-$(mktemp -d)}"
SCOPES="component:vision_service,component:telemetry_service"

# Every snapshot under a tree, as paths relative to that tree, sorted.
snapshot_paths() {
  local root="$1"
  (cd "${root}" && find . -name 'resolve_result.*.json' -printf '%P\n' | sort)
}

# ---------------------------------------------------------------------------
# 0) Compile the example-05 model once; both paths resolve the SAME package
# ---------------------------------------------------------------------------
step "compile the example-05 model"
MODEL_OUT="${WORK}/model"
rm -rf "${MODEL_OUT}"
mkdir -p "${MODEL_OUT}"
"${COMPILER}" compile \
  --source "${DEFS}" \
  --source "${COMPONENTS}" \
  --out "${MODEL_OUT}" \
  > "${MODEL_OUT}/compile_result.json" 2> "${MODEL_OUT}/compile.err" || {
    echo "--- compile stderr ---" >&2
    cat "${MODEL_OUT}/compile.err" >&2 || true
    fail "compile did not complete cleanly"
  }
CMP="${MODEL_OUT}/cmp.manifest.json"
[[ -f "${CMP}" ]] || fail "compile: cmp.manifest.json missing"
echo "  -> cmp.manifest.json produced"

export CONFIGFLUX_COMPILER="${COMPILER}"
export CONFIGFLUX_INTERPRETER="${INTERPRETER}"

# ---------------------------------------------------------------------------
# 1) The reference envelope path (the oracle)
# ---------------------------------------------------------------------------
step "reference resolver --matrix (the envelope path)"
SCRIPT_OUT="${WORK}/script-out"
rm -rf "${SCRIPT_OUT}"
bash "${RESOLVER}" \
  --cmp "${CMP}" \
  --manifest "${MANIFEST}" \
  --matrix \
  --scopes "${SCOPES}" \
  --out "${SCRIPT_OUT}" > "${WORK}/script.log" 2>&1 || {
    cat "${WORK}/script.log" >&2
    fail "reference resolver --matrix did not complete cleanly"
  }
mapfile -t SCRIPT_SNAPSHOTS < <(snapshot_paths "${SCRIPT_OUT}")
[[ "${#SCRIPT_SNAPSHOTS[@]}" -eq 4 ]] \
  || fail "expected 4 reference snapshots, got ${#SCRIPT_SNAPSHOTS[@]}"
echo "  -> ${#SCRIPT_SNAPSHOTS[@]} reference snapshots"

# ---------------------------------------------------------------------------
# 2) The product path
# ---------------------------------------------------------------------------
step "cfx resolve --manifest --all (the product path)"
CFX_OUT="${WORK}/cfx-out"
rm -rf "${CFX_OUT}"
"${CFX}" resolve \
  --model "${CMP}" \
  --manifest "${MANIFEST}" \
  --all \
  --scopes "${SCOPES}" \
  --out "${CFX_OUT}" > "${WORK}/cfx.log" 2>&1 || {
    cat "${WORK}/cfx.log" >&2
    fail "cfx resolve --all did not complete cleanly"
  }
mapfile -t CFX_SNAPSHOTS < <(snapshot_paths "${CFX_OUT}")
echo "  -> ${#CFX_SNAPSHOTS[@]} cfx snapshots"

# ---------------------------------------------------------------------------
# 3) LAYOUT — identical relative snapshot paths
# ---------------------------------------------------------------------------
step "layout: identical relative snapshot paths"
if [[ "${SCRIPT_SNAPSHOTS[*]}" != "${CFX_SNAPSHOTS[*]}" ]]; then
  echo "--- reference ---" >&2
  printf '%s\n' "${SCRIPT_SNAPSHOTS[@]}" >&2
  echo "--- cfx ---" >&2
  printf '%s\n' "${CFX_SNAPSHOTS[@]}" >&2
  fail "the two paths disagree about where a cell's snapshot lives"
fi
printf '  -> %s\n' "${CFX_SNAPSHOTS[@]}"

# ---------------------------------------------------------------------------
# 4) BYTES — every snapshot pair is byte-identical  [REQ-CFX-004]
# ---------------------------------------------------------------------------
# This is the requirement: `cfx` matrix resolve is byte-identical to the
# reference envelope path, cell for cell. Everything above sets it up.
assert_matrix_byte_identical_to_reference() {
  local rel
  for rel in "${CFX_SNAPSHOTS[@]}"; do
    cmp -s "${SCRIPT_OUT}/${rel}" "${CFX_OUT}/${rel}" \
      || fail "snapshot differs between the envelope path and cfx: ${rel}"
    [[ "$(jq -r '.status' "${CFX_OUT}/${rel}")" == "ok" ]] \
      || fail "cell did not resolve cleanly: ${rel}"
    echo "  -> ${rel}: identical"
  done
}

step "bytes: every snapshot pair is byte-identical"
assert_matrix_byte_identical_to_reference

# ---------------------------------------------------------------------------
# 5) TREE — `diff -r` clean apart from the exported generated/ files
# ---------------------------------------------------------------------------
step "tree: diff -r clean (generated/ excluded)"
if ! diff -r -x generated "${SCRIPT_OUT}" "${CFX_OUT}" > "${WORK}/tree.diff" 2>&1; then
  cat "${WORK}/tree.diff" >&2
  fail "the two cell trees differ beyond the exported generated/ files"
fi
echo "  -> cell trees are identical"

# Each cfx cell additionally carries the export profile's early-binding files,
# which is what makes a cell directory a complete delivery bundle.
for rel in "${CFX_SNAPSHOTS[@]}"; do
  cell_dir="${CFX_OUT}/$(dirname "${rel}")"
  [[ -f "${cell_dir}/generated/config.hpp" ]] \
    || fail "cfx cell is missing its exported generated/config.hpp: ${cell_dir}"
done
echo "  -> every cfx cell also carries generated/"

step "PASS"
echo "cfx --all is byte-identical to the reference envelope matrix"
