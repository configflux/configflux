#!/usr/bin/env bash
# Stdin-safety regression test for examples/resolve_environment.sh
# (configflux-f3uf).
#
# resolve_environment.sh drives the interpreter verb chain from inside two
# loops whose input arrives on the script's stdin via `<<<` here-strings: the
# inner `select x N` loop (one delta per choice) and the matrix outer loop (one
# cell per environment). With the NATIVE interpreter binary this is benign —
# the --request-file verbs never read stdin. But a CONFIGFLUX_INTERPRETER
# wrapper that ATTACHES stdin — e.g. a `docker run -i` wrapper around the
# official toolchain image (ADR-0033), or an ssh wrapper — drains the remaining
# here-string, so only the FIRST loop item is processed. The single-environment
# case then resolves with only the first choice and fails closed with
# E_RESOLVE_CONTEXT_UNSATISFIED; the matrix case silently resolves only the
# first environment.
#
# This test reproduces that wrapper precisely with a stdin-DRAINING interpreter
# stub (it reads its stdin to EOF and forwards it to the real interpreter,
# exactly as `docker run -i` would) and asserts:
#
#   1. SELECT LOOP: resolving one named environment with >1 choice under the
#      draining wrapper completes — all choices are applied (resolve returns
#      status ok) and the snapshot is BYTE-IDENTICAL to a native-binary run.
#   2. MATRIX LOOP: a matrix run over two environments under the draining
#      wrapper yields one snapshot PER environment (not just the first).
#
# On the pre-fix script both assertions FAIL (the wrapper drains the loop
# input). After the fix (the in-loop interpreter calls read from /dev/null)
# both PASS. Binaries, the resolver, the example sources, and the example
# manifest are located via the runfiles tree, mirroring
# examples/resolve_environment_test.sh. Requires `jq` on the host PATH.
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

# Sanity: the environment under test must have more than one choice, otherwise
# a drain after the first choice would lose nothing and the test would be vacuous.
N_CHOICES="$(jq -r '.environments.production.choices | length' "${MANIFEST}")"
[[ "${N_CHOICES}" -ge 2 ]] \
  || fail "test fixture invalid: 'production' must have >=2 choices, has ${N_CHOICES}"
echo "  -> 'production' has ${N_CHOICES} choices (drain would drop $((N_CHOICES - 1)))"

# ---------------------------------------------------------------------------
# 1) Build a stdin-DRAINING interpreter wrapper (mimics `docker run -i`)
# ---------------------------------------------------------------------------
# The wrapper reads its stdin to EOF and forwards it to the real interpreter,
# exactly as an `-i` container or ssh wrapper attaches and streams host stdin.
# On a resolver that leaks its loop input on stdin, this drains the loop.
STUB="${WORK}/docker_i_interpreter.sh"
cat > "${STUB}" <<'STUB_EOF'
#!/usr/bin/env bash
set -euo pipefail
# Attach + drain stdin (the `-i` behaviour), then hand it to the real binary.
buf="$(cat)"
exec "${CONFIGFLUX_REAL_INTERPRETER}" "$@" <<< "${buf}"
STUB_EOF
chmod +x "${STUB}"
export CONFIGFLUX_REAL_INTERPRETER="${INTERPRETER}"

# ---------------------------------------------------------------------------
# 2) NATIVE baseline — resolve one environment with the real binary directly
# ---------------------------------------------------------------------------
step "native baseline: resolve 'production' with the real interpreter"
NATIVE_OUT="${WORK}/native"
rm -rf "${NATIVE_OUT}"
set +e
CONFIGFLUX_INTERPRETER="${INTERPRETER}" \
  bash "${RESOLVER}" \
    --cmp "${CMP}" --manifest "${MANIFEST}" \
    --environment production --out "${NATIVE_OUT}" \
    > "${WORK}/native.log" 2>&1 </dev/null
RC_NATIVE=$?
set -e
[[ ${RC_NATIVE} -eq 0 ]] || {
  echo "--- native resolver log ---" >&2
  cat "${WORK}/native.log" >&2 || true
  fail "native resolve: expected exit 0, got ${RC_NATIVE}"
}
shopt -s nullglob
NATIVE_SNAP=("${NATIVE_OUT}"/resolve_result.*.json)
shopt -u nullglob
[[ ${#NATIVE_SNAP[@]} -eq 1 ]] || fail "native: expected one snapshot, found ${#NATIVE_SNAP[@]}"
echo "  -> native snapshot $(basename "${NATIVE_SNAP[0]}")"

# ---------------------------------------------------------------------------
# 3) SELECT LOOP — resolve the same environment under the draining wrapper
# ---------------------------------------------------------------------------
# On the pre-fix script the wrapper drains the select loop after the first
# choice, so resolve fails closed (E_RESOLVE_CONTEXT_UNSATISFIED) -> exit 1.
# After the fix all choices are applied and the snapshot matches native.
step "select loop: resolve 'production' under the stdin-draining wrapper"
WRAP_OUT="${WORK}/wrapped"
rm -rf "${WRAP_OUT}"
set +e
CONFIGFLUX_INTERPRETER="${STUB}" \
  bash "${RESOLVER}" \
    --cmp "${CMP}" --manifest "${MANIFEST}" \
    --environment production --out "${WRAP_OUT}" \
    > "${WORK}/wrapped.log" 2>&1 </dev/null
RC_WRAP=$?
set -e
if [[ ${RC_WRAP} -ne 0 ]]; then
  echo "--- wrapped resolver log ---" >&2
  cat "${WORK}/wrapped.log" >&2 || true
  fail "select loop: a stdin-attaching interpreter drained the select loop (exit ${RC_WRAP}); only the first choice was applied"
fi

shopt -s nullglob
WRAP_SNAP=("${WRAP_OUT}"/resolve_result.*.json)
shopt -u nullglob
[[ ${#WRAP_SNAP[@]} -eq 1 ]] || fail "wrapped: expected one snapshot, found ${#WRAP_SNAP[@]}"
[[ "$(jq -er '.status' "${WRAP_SNAP[0]}")" == "ok" ]] \
  || fail "wrapped: resolve status is not ok (the wrapper dropped a choice)"

# The wrapper must change NOTHING: same snapshot the native binary produced.
[[ "$(basename "${WRAP_SNAP[0]}")" == "$(basename "${NATIVE_SNAP[0]}")" ]] \
  || fail "wrapped vs native: snapshot file names differ"
if ! cmp -s "${WRAP_SNAP[0]}" "${NATIVE_SNAP[0]}"; then
  echo "--- diff (wrapped vs native) ---" >&2
  diff "${NATIVE_SNAP[0]}" "${WRAP_SNAP[0]}" >&2 || true
  fail "wrapped snapshot is not byte-identical to the native snapshot"
fi
echo "  -> wrapper resolved all ${N_CHOICES} choices; snapshot identical to native"

# ---------------------------------------------------------------------------
# 4) MATRIX LOOP — two environments under the draining wrapper
# ---------------------------------------------------------------------------
# The matrix outer loop reads environment names on stdin too, so the same
# wrapper would drain it and resolve only the first environment. Use a synthetic
# two-environment manifest derived from the proven-resolvable 'production' entry
# so both cells resolve cleanly; assert one snapshot PER environment.
step "matrix loop: two environments under the stdin-draining wrapper"
SYN_MANIFEST="${WORK}/synthetic.environments.json"
jq '{schema_version: 1,
     environments: {prod_a: .environments.production,
                    prod_b: .environments.production}}' \
  "${MANIFEST}" > "${SYN_MANIFEST}"

MATRIX_OUT="${WORK}/matrix"
rm -rf "${MATRIX_OUT}"
set +e
CONFIGFLUX_INTERPRETER="${STUB}" \
  bash "${RESOLVER}" \
    --cmp "${CMP}" --manifest "${SYN_MANIFEST}" \
    --matrix --out "${MATRIX_OUT}" \
    > "${WORK}/matrix.log" 2>&1 </dev/null
RC_MATRIX=$?
set -e
if [[ ${RC_MATRIX} -ne 0 ]]; then
  echo "--- matrix resolver log ---" >&2
  cat "${WORK}/matrix.log" >&2 || true
  fail "matrix loop: a stdin-attaching interpreter drained the environment loop (exit ${RC_MATRIX})"
fi

MATRIX_COUNT="$(find "${MATRIX_OUT}" -type f -name 'resolve_result.*.json' | wc -l | tr -d ' ')"
[[ "${MATRIX_COUNT}" -eq 2 ]] || {
  echo "--- matrix tree ---" >&2
  find "${MATRIX_OUT}" -type f >&2 || true
  fail "matrix loop: expected one snapshot per environment (2), found ${MATRIX_COUNT}"
}
find "${MATRIX_OUT}/prod_a" -name 'resolve_result.*.json' | grep -q . \
  || fail "matrix loop: environment 'prod_a' produced no snapshot"
find "${MATRIX_OUT}/prod_b" -name 'resolve_result.*.json' | grep -q . \
  || fail "matrix loop: environment 'prod_b' produced no snapshot (the loop was drained after the first)"
echo "  -> matrix resolved both environments under the wrapper"

step "DONE — resolve_environment.sh is stdin-safe for stdin-attaching wrapper interpreters"
