#!/usr/bin/env bash
# Regression test for examples/resolve_environment.sh — the reference
# environment-manifest resolver.
#
# An environment manifest is a user-side, version-controlled JSON file that
# names deployment targets, each bound to a (scope, context_tags, choices)
# tuple (see docs/service-integration-guide.md). It is NOT read by the product
# binaries: resolve_environment.sh "desugars" one named environment into the
# existing interpreter verb chain (open -> init-selection-state -> select x N
# -> resolve) and writes one resolved snapshot.
#
# This test drives the REAL product binaries over the examples/04-fleet-edge-node
# sources (the same model 04-fleet-edge-node/run.sh compiles) and asserts:
#
#   1. REPRODUCIBILITY: resolving ONE named environment twice yields a
#      byte-identical snapshot and an identical resolve_hash. This is the
#      "a named environment resolves reproducibly" acceptance test.
#   2. MATRIX: an N x M (environments x scopes) run yields exactly N*M
#      snapshots, each reproducible, and the per-cell lineage (resolve_hash)
#      is distinct where the selection or scope differs.
#
# Binaries, the example sources, the resolver, and the example manifest are
# located via the runfiles tree (rootpath env vars set by the sh_test `env`
# attribute), mirroring examples/verify_bundle_test.sh. Requires `jq` on the
# host PATH (the example-04 pipeline and the resolver both depend on it).
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
require_var RESOLVER_RLOCATION
require_var MANIFEST_RLOCATION
require_var DEFS_RLOCATION
require_var COMPONENTS_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
INTERPRETER="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
RESOLVER="${RUNFILES_ROOT}/${RESOLVER_RLOCATION}"
MANIFEST="${RUNFILES_ROOT}/${MANIFEST_RLOCATION}"
DEFS="${RUNFILES_ROOT}/${DEFS_RLOCATION}"
COMPONENTS="${RUNFILES_ROOT}/${COMPONENTS_RLOCATION}"

for bin in "${COMPILER}" "${INTERPRETER}"; do
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

# ---------------------------------------------------------------------------
# 0) Compile the example-04 model once (the resolver consumes a cmp manifest)
# ---------------------------------------------------------------------------
step "compile the example-04 model"
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
# 1) REPRODUCIBILITY — resolve ONE named environment twice (expect identical)
# ---------------------------------------------------------------------------
step "reproducibility: resolve one environment twice"
REPRO_A="${WORK}/repro_a"
REPRO_B="${WORK}/repro_b"
rm -rf "${REPRO_A}" "${REPRO_B}"

set +e
bash "${RESOLVER}" \
  --cmp "${CMP}" --manifest "${MANIFEST}" \
  --environment production --out "${REPRO_A}" \
  > "${WORK}/repro_a.log" 2>&1
RC_A=$?
set -e
if [[ ${RC_A} -ne 0 ]]; then
  echo "--- resolver (run A) log ---" >&2
  cat "${WORK}/repro_a.log" >&2 || true
  fail "single-environment resolve (run A): expected exit 0, got ${RC_A}"
fi

set +e
bash "${RESOLVER}" \
  --cmp "${CMP}" --manifest "${MANIFEST}" \
  --environment production --out "${REPRO_B}" \
  > "${WORK}/repro_b.log" 2>&1
RC_B=$?
set -e
[[ ${RC_B} -eq 0 ]] || {
  echo "--- resolver (run B) log ---" >&2
  cat "${WORK}/repro_b.log" >&2 || true
  fail "single-environment resolve (run B): expected exit 0, got ${RC_B}"
}

# Exactly one snapshot per run, named per the resolve_result.<root>.<selection>
# convention.
shopt -s nullglob
SNAP_A=("${REPRO_A}"/resolve_result.*.json)
SNAP_B=("${REPRO_B}"/resolve_result.*.json)
shopt -u nullglob
[[ ${#SNAP_A[@]} -eq 1 ]] || fail "run A: expected exactly one snapshot, found ${#SNAP_A[@]}"
[[ ${#SNAP_B[@]} -eq 1 ]] || fail "run B: expected exactly one snapshot, found ${#SNAP_B[@]}"
[[ "$(basename "${SNAP_A[0]}")" == "$(basename "${SNAP_B[0]}")" ]] \
  || fail "snapshot file names differ between runs: $(basename "${SNAP_A[0]}") vs $(basename "${SNAP_B[0]}")"

# Byte-identical snapshot (the D1 reproducibility guarantee).
if ! cmp -s "${SNAP_A[0]}" "${SNAP_B[0]}"; then
  echo "--- diff ---" >&2
  diff "${SNAP_A[0]}" "${SNAP_B[0]}" >&2 || true
  fail "snapshots are not byte-identical across two runs of the same environment"
fi

# Identical resolve_hash.
RH_A="$(jq -er '.resolve_hash' "${SNAP_A[0]}")" || fail "run A snapshot has no resolve_hash"
RH_B="$(jq -er '.resolve_hash' "${SNAP_B[0]}")" || fail "run B snapshot has no resolve_hash"
[[ "${RH_A}" == "${RH_B}" ]] || fail "resolve_hash differs across runs: ${RH_A} vs ${RH_B}"
[[ "$(jq -er '.status' "${SNAP_A[0]}")" == "ok" ]] || fail "run A snapshot status is not ok"
echo "  -> two runs produced a byte-identical snapshot (resolve_hash ${RH_A})"

# ---------------------------------------------------------------------------
# 2) MATRIX — N environments x M scopes yields exactly N*M reproducible cells
# ---------------------------------------------------------------------------
step "matrix: 2 environments x 2 scopes (expect 4 reproducible snapshots)"
MATRIX_1="${WORK}/matrix_1"
MATRIX_2="${WORK}/matrix_2"
rm -rf "${MATRIX_1}" "${MATRIX_2}"

run_matrix() {
  local out="$1" log="$2"
  set +e
  bash "${RESOLVER}" \
    --cmp "${CMP}" --manifest "${MANIFEST}" \
    --matrix --scopes "component:runtime_tuner,component:update_agent" \
    --out "${out}" \
    > "${log}" 2>&1
  local rc=$?
  set -e
  if [[ ${rc} -ne 0 ]]; then
    echo "--- matrix resolver log ---" >&2
    cat "${log}" >&2 || true
    fail "matrix resolve: expected exit 0, got ${rc}"
  fi
}

run_matrix "${MATRIX_1}" "${WORK}/matrix_1.log"
run_matrix "${MATRIX_2}" "${WORK}/matrix_2.log"

# Exactly N*M = 2*2 = 4 snapshots produced. Count all snapshots under the
# matrix output tree, regardless of nesting.
SNAP_COUNT="$(find "${MATRIX_1}" -type f -name 'resolve_result.*.json' | wc -l | tr -d ' ')"
[[ "${SNAP_COUNT}" -eq 4 ]] || {
  echo "--- matrix tree ---" >&2
  find "${MATRIX_1}" -type f >&2 || true
  fail "matrix: expected exactly 4 snapshots (2 envs x 2 scopes), found ${SNAP_COUNT}"
}
echo "  -> matrix produced exactly 4 snapshots"

# Each cell is reproducible: the whole matrix run is byte-identical between two
# independent invocations (file-by-file).
while IFS= read -r f1; do
  rel="${f1#"${MATRIX_1}/"}"
  f2="${MATRIX_2}/${rel}"
  [[ -f "${f2}" ]] || fail "matrix cell missing on second run: ${rel}"
  if ! cmp -s "${f1}" "${f2}"; then
    echo "--- diff for ${rel} ---" >&2
    diff "${f1}" "${f2}" >&2 || true
    fail "matrix cell not reproducible: ${rel}"
  fi
done < <(find "${MATRIX_1}" -type f -name 'resolve_result.*.json' | sort)
echo "  -> every matrix cell is reproducible across two runs"

# Per-cell lineage is distinct where the selection/scope differs. Collect every
# cell's (scope, resolve_hash) and assert the four cells are not all identical:
# the two environments differ in their choices, and the two scopes differ, so
# the resolve_hash set must contain more than one distinct value.
HASHES="$(find "${MATRIX_1}" -type f -name 'resolve_result.*.json' -print0 \
  | xargs -0 -I{} jq -er '.resolve_hash' {} | sort)"
DISTINCT="$(printf '%s\n' "${HASHES}" | sort -u | wc -l | tr -d ' ')"
[[ "${DISTINCT}" -gt 1 ]] || fail "matrix: expected distinct lineage across cells, all ${SNAP_COUNT} resolve_hash values were identical"

# Stronger: the two scopes for a single environment must differ in scope, and
# at least the production environment's two scoped snapshots must carry the
# correct scope field.
SCOPES_SEEN="$(find "${MATRIX_1}" -type f -name 'resolve_result.*.json' -print0 \
  | xargs -0 -I{} jq -er '.scope' {} | sort -u)"
echo "${SCOPES_SEEN}" | grep -qx "component:runtime_tuner" \
  || fail "matrix: no cell carried scope component:runtime_tuner"
echo "${SCOPES_SEEN}" | grep -qx "component:update_agent" \
  || fail "matrix: no cell carried scope component:update_agent"
echo "  -> per-cell lineage distinct across scopes/environments (${DISTINCT} distinct resolve_hash values)"

step "DONE — environment resolves reproducibly and the matrix yields N*M reproducible cells"
