#!/usr/bin/env bash
# Regression test for examples/verify_bundle.sh — the reference delivery-bundle
# verification script.
#
# Produces a REAL delivery bundle by driving the product binaries over the
# examples/04-fleet-edge-node sources (the same pipeline as
# 04-fleet-edge-node/run.sh: compile -> cfx resolve, which also emits the
# sibling ccm/), assembles a minimal standard-layout bundle from the result,
# and asserts that verify_bundle.sh:
#
#   1. EXITS 0 on a well-formed, matched bundle (snapshot + ccm/, equal hashes)
#   2. REJECTS (non-zero) a bundle whose ccm/ directory has been removed
#   3. REJECTS (non-zero) a bundle whose ccm/ccm.manifest.json bound_model_hash
#      has been tampered to a different value (a mis-assembled pair)
#
# The standard bundle layout being verified (ratified in
# docs/service-integration-guide.md):
#
#   <bundle>/
#     resolve_result.<root>.<selection>.json   # per-scope resolved snapshot
#     ccm/                                      # solver model
#       ccm.manifest.json
#       ccm.symbols.json
#       partition-*/ccm.bdd.bin
#
# Binaries, the example sources, run.sh, and the verify script are located via
# the runfiles tree (rootpath env vars set by the sh_test `env` attribute),
# mirroring the convention in examples/run_example_test.sh and
# examples/e2e_03_motor_controller_test.sh. Requires `jq` on the host PATH (the
# example-04 pipeline and the verify script both depend on it).
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
require_var CFX_RLOCATION
require_var RUNTIME_RLOCATION
require_var EXAMPLE_RUN_RLOCATION
require_var VERIFY_SCRIPT_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
CFX="${RUNFILES_ROOT}/${CFX_RLOCATION}"
RUNTIME="${RUNFILES_ROOT}/${RUNTIME_RLOCATION}"
EXAMPLE_RUN="${RUNFILES_ROOT}/${EXAMPLE_RUN_RLOCATION}"
VERIFY_SCRIPT="${RUNFILES_ROOT}/${VERIFY_SCRIPT_RLOCATION}"

for bin in "${COMPILER}" "${CFX}" "${RUNTIME}"; do
  if [[ ! -x "${bin}" ]]; then
    echo "ERROR: binary not executable at ${bin}" >&2
    exit 2
  fi
done
for f in "${EXAMPLE_RUN}" "${VERIFY_SCRIPT}"; do
  if [[ ! -f "${f}" ]]; then
    echo "ERROR: script not found at ${f}" >&2
    exit 2
  fi
done

fail() {
  echo "ASSERT FAILED: $*" >&2
  exit 1
}

step() { printf '\n=== %s ===\n' "$1"; }

# ---------------------------------------------------------------------------
# 1) Produce a real resolve result + ccm/ via the example-04 pipeline
# ---------------------------------------------------------------------------
step "produce resolve result via example-04 pipeline"
PIPE_OUT="${TEST_TMPDIR:-$(mktemp -d)}/pipeline"
rm -rf "${PIPE_OUT}"
mkdir -p "${PIPE_OUT}"

# run.sh locates its binaries from these env vars and writes every output into
# CONFIGFLUX_EXAMPLE_OUT_DIR. It runs the full compile -> resolve chain
# (device_class=gateway, update_channel=canary, region=eu) and emits the
# sibling ccm/ during compile.
export CONFIGFLUX_COMPILER="${COMPILER}"
export CONFIGFLUX_CFX="${CFX}"
export CONFIGFLUX_RUNTIME="${RUNTIME}"
export CONFIGFLUX_EXAMPLE_OUT_DIR="${PIPE_OUT}"
bash "${EXAMPLE_RUN}" > "${PIPE_OUT}/run.log" 2>&1 || {
  echo "--- example run.sh log ---" >&2
  cat "${PIPE_OUT}/run.log" >&2 || true
  fail "example-04 pipeline did not complete cleanly"
}

[[ -f "${PIPE_OUT}/resolve.result.json" ]] || fail "pipeline: resolve.result.json missing"
[[ -f "${PIPE_OUT}/ccm/ccm.manifest.json" ]] || fail "pipeline: ccm/ccm.manifest.json missing"
echo "  -> resolve.result.json + ccm/ produced"

# ---------------------------------------------------------------------------
# 2) Assemble a minimal standard-layout bundle
# ---------------------------------------------------------------------------
# The snapshot's scope is component:runtime_tuner and the canary update channel
# was selected, so the standard per-scope snapshot name is
# resolve_result.runtime_tuner.canary.json (matching the guide's worked names).
step "assemble standard-layout bundle"
BUNDLE="${TEST_TMPDIR:-$(mktemp -d)}/bundle"
rm -rf "${BUNDLE}"
mkdir -p "${BUNDLE}"
cp "${PIPE_OUT}/resolve.result.json" "${BUNDLE}/resolve_result.runtime_tuner.canary.json"
cp -R "${PIPE_OUT}/ccm" "${BUNDLE}/ccm"
echo "  -> bundle assembled at ${BUNDLE}"

# ---------------------------------------------------------------------------
# 3) Well-formed bundle must PASS (exit 0)
# ---------------------------------------------------------------------------
step "verify_bundle.sh on a well-formed bundle (expect exit 0)"
set +e
bash "${VERIFY_SCRIPT}" "${BUNDLE}" > "${PIPE_OUT}/verify_ok.log" 2>&1
OK_RC=$?
set -e
if [[ ${OK_RC} -ne 0 ]]; then
  echo "--- verify (well-formed) output ---" >&2
  cat "${PIPE_OUT}/verify_ok.log" >&2 || true
  fail "well-formed bundle: expected exit 0, got ${OK_RC}"
fi
echo "  -> well-formed bundle accepted (exit 0)"

# The script must also accept a correct expected content hash and reject a
# wrong one, when one is supplied.
EXPECTED_HASH="$(sha256sum "${BUNDLE}/resolve_result.runtime_tuner.canary.json" | awk '{print $1}')"
set +e
bash "${VERIFY_SCRIPT}" "${BUNDLE}" "${EXPECTED_HASH}" > "${PIPE_OUT}/verify_hash_ok.log" 2>&1
HASH_OK_RC=$?
set -e
[[ ${HASH_OK_RC} -eq 0 ]] || {
  echo "--- verify (matching content hash) output ---" >&2
  cat "${PIPE_OUT}/verify_hash_ok.log" >&2 || true
  fail "matching content hash: expected exit 0, got ${HASH_OK_RC}"
}
echo "  -> matching expected content hash accepted (exit 0)"

set +e
bash "${VERIFY_SCRIPT}" "${BUNDLE}" "0000000000000000000000000000000000000000000000000000000000000000" \
  > "${PIPE_OUT}/verify_hash_bad.log" 2>&1
HASH_BAD_RC=$?
set -e
[[ ${HASH_BAD_RC} -ne 0 ]] || {
  echo "--- verify (wrong content hash) output ---" >&2
  cat "${PIPE_OUT}/verify_hash_bad.log" >&2 || true
  fail "wrong content hash: expected non-zero exit, got 0"
}
echo "  -> wrong expected content hash rejected (exit ${HASH_BAD_RC})"

# ---------------------------------------------------------------------------
# 4) Missing ccm/ must be REJECTED (non-zero)
# ---------------------------------------------------------------------------
step "verify_bundle.sh on a bundle with ccm/ removed (expect non-zero)"
NO_CCM="${TEST_TMPDIR:-$(mktemp -d)}/bundle_no_ccm"
rm -rf "${NO_CCM}"
mkdir -p "${NO_CCM}"
cp "${BUNDLE}/resolve_result.runtime_tuner.canary.json" "${NO_CCM}/"
set +e
bash "${VERIFY_SCRIPT}" "${NO_CCM}" > "${PIPE_OUT}/verify_no_ccm.log" 2>&1
NO_CCM_RC=$?
set -e
[[ ${NO_CCM_RC} -ne 0 ]] || {
  echo "--- verify (missing ccm/) output ---" >&2
  cat "${PIPE_OUT}/verify_no_ccm.log" >&2 || true
  fail "missing ccm/: expected non-zero exit, got 0"
}
echo "  -> missing ccm/ rejected (exit ${NO_CCM_RC})"

# ---------------------------------------------------------------------------
# 5) Tampered bound_model_hash must be REJECTED (non-zero)
# ---------------------------------------------------------------------------
step "verify_bundle.sh on a mis-assembled bundle (tampered bound_model_hash)"
BAD_PAIR="${TEST_TMPDIR:-$(mktemp -d)}/bundle_bad_pair"
rm -rf "${BAD_PAIR}"
mkdir -p "${BAD_PAIR}"
cp "${BUNDLE}/resolve_result.runtime_tuner.canary.json" "${BAD_PAIR}/"
cp -R "${BUNDLE}/ccm" "${BAD_PAIR}/ccm"
# Rewrite bound_model_hash to a different, well-formed sha256 hex string so the
# snapshot and ccm/ no longer belong together.
TAMPERED="deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
jq --arg h "${TAMPERED}" '.bound_model_hash = $h' \
  "${BAD_PAIR}/ccm/ccm.manifest.json" > "${BAD_PAIR}/ccm/ccm.manifest.json.tmp"
mv "${BAD_PAIR}/ccm/ccm.manifest.json.tmp" "${BAD_PAIR}/ccm/ccm.manifest.json"
set +e
bash "${VERIFY_SCRIPT}" "${BAD_PAIR}" > "${PIPE_OUT}/verify_bad_pair.log" 2>&1
BAD_PAIR_RC=$?
set -e
[[ ${BAD_PAIR_RC} -ne 0 ]] || {
  echo "--- verify (mismatched pair) output ---" >&2
  cat "${PIPE_OUT}/verify_bad_pair.log" >&2 || true
  fail "mismatched pair: expected non-zero exit, got 0"
}
echo "  -> mismatched model_hash/bound_model_hash rejected (exit ${BAD_PAIR_RC})"

step "DONE — verify_bundle.sh accepts a matched bundle and rejects mis-assembled ones"
