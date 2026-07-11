#!/usr/bin/env bash
# End-to-end test for examples/05-compose-fleet — the docker-compose fleet
# example (the demonstration that wires together the convention-only deployment
# surfaces from ADR-0032: named environments + matrix resolve + delivery
# bundle, consumed by a Pattern 1 service).
#
# This test exercises the NON-DOCKER chain (docker is not available under bazel
# test, and the example's run.sh skips compose in that case). It drives the
# REAL product binaries and the example's own service / override generator:
#
#   compiler compile                 -> CMP package + sibling ccm/ (model_hash)
#   interpreter open/init/select x2/resolve, twice per environment, once per
#     service scope                  -> 4 resolved snapshots (2 envs x 2 scopes)
#   assemble delivery bundle (snapshot + ccm/) per cell, verify the binding
#   gen_compose_override.py          -> a compose override from a snapshot
#   service/app.py (standalone)      -> Pattern 1 load + lineage pin + values
#
# All JSON shaping and assertions use python3 (no jq dependency), mirroring
# examples/e2e_03_motor_controller_test.sh. The example's OWN reference scripts
# (examples/resolve_environment.sh, examples/verify_bundle.sh) require jq; this
# test ADDITIONALLY runs them when jq is on PATH, so the shipped chain is
# covered where possible without making jq a hard test dependency.
#
# Determinism: model_hash is derived from the --source argument STRINGS plus
# content, so sources are staged into the writable out dir and compiled with
# FILENAME-ONLY source ids (verified stable across working directories), which
# makes the cascading resolve_hash stable too.
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
require_var DEFS_RLOCATION
require_var COMPONENTS_RLOCATION
require_var MANIFEST_RLOCATION
require_var APP_RLOCATION
require_var OVERRIDE_GEN_RLOCATION
require_var RENDER_GEN_RLOCATION
require_var RESOLVER_RLOCATION
require_var VERIFIER_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
INTERPRETER="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
DEFS="${RUNFILES_ROOT}/${DEFS_RLOCATION}"
COMPONENTS="${RUNFILES_ROOT}/${COMPONENTS_RLOCATION}"
MANIFEST="${RUNFILES_ROOT}/${MANIFEST_RLOCATION}"
APP="${RUNFILES_ROOT}/${APP_RLOCATION}"
OVERRIDE_GEN="${RUNFILES_ROOT}/${OVERRIDE_GEN_RLOCATION}"
RENDER_GEN="${RUNFILES_ROOT}/${RENDER_GEN_RLOCATION}"
RESOLVER="${RUNFILES_ROOT}/${RESOLVER_RLOCATION}"
VERIFIER="${RUNFILES_ROOT}/${VERIFIER_RLOCATION}"

for bin in "${COMPILER}" "${INTERPRETER}"; do
  [[ -x "${bin}" ]] || { echo "ERROR: binary not executable at ${bin}" >&2; exit 2; }
done
for f in "${DEFS}" "${COMPONENTS}" "${MANIFEST}" "${APP}" "${OVERRIDE_GEN}" "${RENDER_GEN}"; do
  [[ -f "${f}" ]] || { echo "ERROR: data file not found at ${f}" >&2; exit 2; }
done

OUT="${TEST_TMPDIR:-$(mktemp -d)}/out"
rm -rf "${OUT}"
mkdir -p "${OUT}"

# Service scopes and the two named environments (must match environments.json).
SCOPES=("vision_service" "telemetry_service")
ENVS=("robot-alpha" "local")

fail() { echo "ASSERT FAILED: $*" >&2; exit 1; }
step() { printf '\n=== %s ===\n' "$1"; }

# jget FILE PY_EXPR — evaluate a python expression against parsed JSON in FILE
# (bound as `d`) and print the result. Keeps the harness jq-free.
jget() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
print(eval(sys.argv[2]))' "$1" "$2"
}

assert_status_ok() {
  local file="$1" label="$2" status
  status="$(jget "${file}" 'd["status"]')"
  if [[ "${status}" != "ok" ]]; then
    echo "--- ${label} response ---" >&2; cat "${file}" >&2 || true
    fail "${label}: expected status ok, got '${status}'"
  fi
}

# ---------------------------------------------------------------------------
# 1) compile — CMP + sibling ccm/, with filename-only source ids
# ---------------------------------------------------------------------------
step "compile"
SRC_DIR="${OUT}/src"
mkdir -p "${SRC_DIR}"
cp "${DEFS}" "${SRC_DIR}/00_definitions.json"
cp "${COMPONENTS}" "${SRC_DIR}/10_components.json"
(
  cd "${SRC_DIR}"
  "${COMPILER}" compile \
    --source 00_definitions.json \
    --source 10_components.json \
    --out "${OUT}/cmp" \
    > "${OUT}/compile.result.json"
)
assert_status_ok "${OUT}/compile.result.json" "compile"
[[ -f "${OUT}/cmp/cmp.manifest.json" ]] || fail "compile: cmp.manifest.json missing"
[[ -f "${OUT}/cmp/ccm/ccm.manifest.json" ]] || fail "compile: sibling ccm/ not emitted"
MODEL_HASH="$(jget "${OUT}/cmp/cmp.manifest.json" 'd["model_hash"]')"
BOUND_HASH="$(jget "${OUT}/cmp/ccm/ccm.manifest.json" 'd["bound_model_hash"]')"
[[ "${MODEL_HASH}" == "${BOUND_HASH}" ]] || fail "compile: model_hash != ccm bound_model_hash"
echo "  -> CMP + ccm/ produced; model_hash == bound_model_hash"

# ---------------------------------------------------------------------------
# Helper: resolve ONE (environment, scope) cell into a snapshot file via the
# interpreter verb chain, reading the env's choices/context_tags from the
# manifest. Mirrors what examples/resolve_environment.sh does, in python3.
# Writes the snapshot path to stdout.
# ---------------------------------------------------------------------------
resolve_cell() {
  local env_name="$1" root="$2" scope="component:${root}"
  local work="${OUT}/work/${env_name}/${root}"
  mkdir -p "${work}"

  # open
  python3 -c 'import json,sys
json.dump({"schema_version":3,"cmp_manifest_ref":sys.argv[1]},open(sys.argv[2],"w"))' \
    "${OUT}/cmp/cmp.manifest.json" "${work}/open.req.json"
  "${INTERPRETER}" open --request-file "${work}/open.req.json" --response-file "${work}/open.res.json"
  assert_status_ok "${work}/open.res.json" "open(${env_name},${root})"

  # init-selection-state with the env's context_tags
  python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); m=json.load(open(sys.argv[2]))
env=m["environments"][sys.argv[4]]
json.dump({"schema_version":3,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "context_tags":env.get("context_tags",{})},open(sys.argv[5],"w"))' \
    "${work}/open.res.json" "${MANIFEST}" "${scope}" "${env_name}" "${work}/init.req.json"
  "${INTERPRETER}" init-selection-state \
    --request-file "${work}/init.req.json" --response-file "${work}/init.res.json"
  assert_status_ok "${work}/init.res.json" "init(${env_name},${root})"

  # select x N — one delta per choice, sorted by facet for determinism
  local prev="${work}/init.res.json"
  local facets
  facets="$(python3 -c 'import json,sys
m=json.load(open(sys.argv[1]))
print(" ".join(sorted(m["environments"][sys.argv[2]].get("choices",{}).keys())))' \
    "${MANIFEST}" "${env_name}")"
  local idx=0
  for facet in ${facets}; do
    idx=$((idx + 1))
    python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2])); m=json.load(open(sys.argv[3]))
opt=m["environments"][sys.argv[5]]["choices"][sys.argv[6]]
json.dump({"schema_version":3,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":sys.argv[6],"option":opt}},open(sys.argv[7],"w"))' \
      "${work}/open.res.json" "${prev}" "${MANIFEST}" "${scope}" "${env_name}" "${facet}" \
      "${work}/select_${idx}.req.json"
    "${INTERPRETER}" select \
      --request-file "${work}/select_${idx}.req.json" \
      --response-file "${work}/select_${idx}.res.json"
    assert_status_ok "${work}/select_${idx}.res.json" "select ${facet} (${env_name},${root})"
    prev="${work}/select_${idx}.res.json"
  done

  # resolve
  python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":3,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "selection_state":s["selection_state"]},open(sys.argv[4],"w"))' \
    "${work}/open.res.json" "${prev}" "${scope}" "${work}/resolve.req.json"
  local snap="${OUT}/resolved/${env_name}/${root}/resolve_result.${root}.json"
  mkdir -p "$(dirname "${snap}")"
  "${INTERPRETER}" resolve --request-file "${work}/resolve.req.json" --response-file "${snap}"
  assert_status_ok "${snap}" "resolve(${env_name},${root})"
  echo "${snap}"
}

# ---------------------------------------------------------------------------
# 2) matrix resolve — 2 environments x 2 service scopes = 4 snapshots
# ---------------------------------------------------------------------------
step "matrix resolve (2 environments x 2 scopes)"
declare -A SNAP
for env_name in "${ENVS[@]}"; do
  for root in "${SCOPES[@]}"; do
    SNAP["${env_name}/${root}"]="$(resolve_cell "${env_name}" "${root}")"
    echo "  -> ${env_name}/${root}: $(basename "${SNAP["${env_name}/${root}"]}")"
  done
done

# Assert exactly 4 snapshots, each with a DISTINCT resolve_hash.
mapfile -t ALL_HASHES < <(for k in "${!SNAP[@]}"; do jget "${SNAP[$k]}" 'd["resolve_hash"]'; done)
[[ "${#ALL_HASHES[@]}" -eq 4 ]] || fail "expected 4 snapshots, got ${#ALL_HASHES[@]}"
DISTINCT="$(printf '%s\n' "${ALL_HASHES[@]}" | sort -u | wc -l)"
[[ "${DISTINCT}" -eq 4 ]] || fail "expected 4 distinct resolve_hashes, got ${DISTINCT}"
echo "  -> 4 snapshots, 4 distinct resolve_hashes"

# Assert the two environments produce DIFFERENT resolved values for the same
# scope (robot vs local broker_endpoint on vision_service).
RA_BROKER="$(jget "${SNAP["robot-alpha/vision_service"]}" \
  'd["resolved_output"]["vision_service"]["components"]["vision_service"]["params"]["broker_endpoint"]["value"]')"
LOCAL_BROKER="$(jget "${SNAP["local/vision_service"]}" \
  'd["resolved_output"]["vision_service"]["components"]["vision_service"]["params"]["broker_endpoint"]["value"]')"
[[ "${RA_BROKER}" != "${LOCAL_BROKER}" ]] \
  || fail "robot-alpha and local resolved the SAME broker_endpoint (${RA_BROKER}); environments not differentiated"
echo "  -> environments differ: robot-alpha broker=${RA_BROKER}, local broker=${LOCAL_BROKER}"

# ---------------------------------------------------------------------------
# 3) assemble + verify a delivery bundle per cell (snapshot + ccm/)
# ---------------------------------------------------------------------------
step "assemble + verify delivery bundles"
verify_binding() {
  # Python verifier: snapshot.model_hash == ccm/ bound_model_hash. Mirrors the
  # core check of examples/verify_bundle.sh without the jq dependency.
  python3 -c 'import json,sys
snap=json.load(open(sys.argv[1])); ccm=json.load(open(sys.argv[2]))
mh=snap["model_hash"]; bmh=ccm["bound_model_hash"]
assert mh==bmh, "bundle mismatch: %s != %s" % (mh, bmh)
print("  binding OK:", mh[:16])' "$1" "$2"
}
for k in "${!SNAP[@]}"; do
  bdir="${OUT}/bundles/${k//\//--}"
  mkdir -p "${bdir}"
  cp "${SNAP[$k]}" "${bdir}/"
  cp -R "${OUT}/cmp/ccm" "${bdir}/ccm"
  # exactly one snapshot at the bundle root
  n="$(find "${bdir}" -maxdepth 1 -name 'resolve_result.*.json' | wc -l)"
  [[ "${n}" -eq 1 ]] || fail "bundle ${k}: expected exactly 1 snapshot at root, found ${n}"
  verify_binding "${SNAP[$k]}" "${bdir}/ccm/ccm.manifest.json" >/dev/null \
    || fail "bundle ${k}: binding verification failed"
done
echo "  -> 4 bundles assembled; each has one snapshot + a matched ccm/"

# ---------------------------------------------------------------------------
# 4) compose override generator (user-side transform)
# ---------------------------------------------------------------------------
step "compose override generator (user-side)"
OVR="${OUT}/vision.override.yml"
python3 "${OVERRIDE_GEN}" \
  --snapshot "${SNAP["robot-alpha/vision_service"]}" \
  --service vision-service \
  --out "${OVR}"
[[ -f "${OVR}" ]] || fail "override generator did not write ${OVR}"
# Structural assertions on the emitted override (no PyYAML dependency): the
# service block, an environment block, and sorted UPPERCASE COMPONENT_PARAM keys.
python3 -c '
import sys
lines=[l.rstrip("\n") for l in open(sys.argv[1])]
assert "services:" in lines, "missing services:"
assert any(l.strip()=="vision-service:" for l in lines), "missing service entry"
assert any(l.strip()=="environment:" for l in lines), "missing environment:"
keys=[l.strip().split(":")[0] for l in lines if l.startswith("      ") and ":" in l]
assert keys, "no environment keys emitted"
assert all(k.isupper() for k in keys), f"keys not uppercase: {keys}"
assert keys==sorted(keys), f"keys not sorted: {keys}"
assert any(k.startswith("VISION_SERVICE_") for k in keys), "no VISION_SERVICE_* key"
print("  -> override OK; sorted uppercase keys:", keys)
' "${OVR}"

# ---------------------------------------------------------------------------
# 5) standalone service (Pattern 1) against the LOCAL bundle — happy path
# ---------------------------------------------------------------------------
step "standalone service — Pattern 1 load + lineage pin"
LOCAL_SNAP="${SNAP["local/vision_service"]}"
EXPECT_RH="$(jget "${LOCAL_SNAP}" 'd["resolve_hash"]')"
SVC_OUT="${OUT}/service.stdout.txt"
CONFIGFLUX_SNAPSHOT="${LOCAL_SNAP}" \
CONFIGFLUX_ENV_LABEL="local (test)" \
CONFIGFLUX_EXPECT_RESOLVE_HASH="${EXPECT_RH}" \
  python3 "${APP}" > "${SVC_OUT}"
grep -q "startup OK" "${SVC_OUT}" || { cat "${SVC_OUT}" >&2; fail "service did not report startup OK"; }
grep -q "scope       : component:vision_service" "${SVC_OUT}" || { cat "${SVC_OUT}" >&2; fail "service did not print scope"; }
grep -q "tcp://127.0.0.1:1883" "${SVC_OUT}" || { cat "${SVC_OUT}" >&2; fail "service did not load the local broker value"; }
echo "  -> standalone service loaded local bundle, pinned resolve_hash, printed config"

# ---------------------------------------------------------------------------
# 6) standalone service — FAIL CLOSED on a wrong resolve_hash pin
# ---------------------------------------------------------------------------
step "standalone service — fail closed on hash mismatch"
set +e
CONFIGFLUX_SNAPSHOT="${LOCAL_SNAP}" \
CONFIGFLUX_EXPECT_RESOLVE_HASH="0000000000000000000000000000000000000000000000000000000000000000" \
  python3 "${APP}" > "${OUT}/service.bad.txt" 2>&1
BAD_RC=$?
set -e
[[ "${BAD_RC}" -ne 0 ]] || { cat "${OUT}/service.bad.txt" >&2; fail "service did NOT fail closed on hash mismatch"; }
grep -q "resolve_hash mismatch" "${OUT}/service.bad.txt" || { cat "${OUT}/service.bad.txt" >&2; fail "missing mismatch diagnostic"; }
echo "  -> fail-closed confirmed (exit ${BAD_RC})"

# ---------------------------------------------------------------------------
# 7) (optional) exercise the shipped reference scripts when jq is available
# ---------------------------------------------------------------------------
if command -v jq >/dev/null 2>&1; then
  step "reference scripts (jq present) — resolve_environment.sh + verify_bundle.sh"
  export CONFIGFLUX_INTERPRETER="${INTERPRETER}"
  REF_OUT="${OUT}/ref"
  "${RESOLVER}" \
    --cmp "${OUT}/cmp/cmp.manifest.json" \
    --manifest "${MANIFEST}" \
    --matrix \
    --scopes "component:vision_service,component:telemetry_service" \
    --out "${REF_OUT}" >/dev/null
  REF_N="$(find "${REF_OUT}" -name 'resolve_result.*.json' | wc -l)"
  [[ "${REF_N}" -eq 4 ]] || fail "reference resolver produced ${REF_N} snapshots, expected 4"
  # Verify one assembled bundle through the reference verifier.
  REF_SNAP="$(find "${REF_OUT}/robot-alpha/vision_service" -name 'resolve_result.*.json' | head -n1)"
  REF_BUNDLE="${OUT}/ref-bundle"
  mkdir -p "${REF_BUNDLE}"
  cp "${REF_SNAP}" "${REF_BUNDLE}/"
  cp -R "${OUT}/cmp/ccm" "${REF_BUNDLE}/ccm"
  "${VERIFIER}" "${REF_BUNDLE}" >/dev/null || fail "reference verifier rejected a matched bundle"
  echo "  -> reference scripts produced 4 snapshots and verified a matched bundle"
else
  step "reference scripts skipped (jq not on PATH)"
  echo "  -> python3 chain above already covered the same surfaces"
fi

# ---------------------------------------------------------------------------
# 8) render-final-files transform (user-side) — NESTED app-config document
# ---------------------------------------------------------------------------
# render_app_config.py is the SECOND user-side transform (ADR-0035 C3 / ADR-0032
# D3): it shapes the same resolved snapshot into a NESTED config-file document —
# the layered shape app-config files take in common service frameworks — as
# opposed to gen_compose_override.py's FLAT uppercased env vars. Asserted with
# python3 (no PyYAML/jq): the rendered doc is valid JSON, nested
# component -> param -> value, carries a top-level lineage block, and is
# deterministic (two renders are byte-identical).
step "render-final-files transform (user-side, nested app-config document)"
RA_VISION_SNAP="${SNAP["robot-alpha/vision_service"]}"
APPCFG="${OUT}/vision-service.app-config.json"
python3 "${RENDER_GEN}" --snapshot "${RA_VISION_SNAP}" --out "${APPCFG}"
[[ -f "${APPCFG}" ]] || fail "render transform did not write ${APPCFG}"

# Structural assertions (python3, no PyYAML/jq): valid JSON, a lineage block
# whose model_hash/resolve_hash/scope match the snapshot, and a NESTED config
# section (scope root -> component -> param -> value) reproducing the snapshot
# leaves exactly — NOT flat KEY=value.
python3 -c '
import json, sys
doc = json.load(open(sys.argv[1])); snap = json.load(open(sys.argv[2]))
assert isinstance(doc, dict), "rendered document is not a JSON object"
lin = doc.get("lineage")
assert isinstance(lin, dict), "missing top-level lineage block"
for k in ("model_hash", "resolve_hash", "scope"):
    assert lin.get(k) == snap.get(k), f"lineage.{k} mismatch: {lin.get(k)!r} != {snap.get(k)!r}"
root = snap["scope"].split(":", 1)[1] if ":" in snap["scope"] else snap["scope"]
want = {c: {p: leaf.get("value") for p, leaf in cdef.get("params", {}).items()}
        for c, cdef in snap["resolved_output"][root]["components"].items()}
assert doc.get(root) == want, "nested scope-root -> component -> param -> value does not reproduce the snapshot leaves"
print("  -> nested app-config doc OK; lineage consistent; component->param nesting verified")
' "${APPCFG}" "${RA_VISION_SNAP}"

# Determinism: a second render is byte-identical (sorted keys).
APPCFG2="${OUT}/vision-service.app-config.2.json"
python3 "${RENDER_GEN}" --snapshot "${RA_VISION_SNAP}" --out "${APPCFG2}"
cmp -s "${APPCFG}" "${APPCFG2}" || { diff "${APPCFG}" "${APPCFG2}" >&2 || true; fail "render transform is not deterministic across two runs"; }
echo "  -> render is deterministic (two runs byte-identical)"

# Lineage-first: the transform refuses a non-ok snapshot (status mutated to a
# non-"ok" value in a COPY) and exits non-zero.
BAD_SNAP="${OUT}/render.bad.json"
python3 -c 'import json,sys
d=json.load(open(sys.argv[1])); d["status"]="error"
json.dump(d,open(sys.argv[2],"w"))' "${RA_VISION_SNAP}" "${BAD_SNAP}"
set +e
python3 "${RENDER_GEN}" --snapshot "${BAD_SNAP}" --out "${OUT}/render.shouldnotexist.json" > "${OUT}/render.bad.log" 2>&1
RENDER_BAD_RC=$?
set -e
[[ "${RENDER_BAD_RC}" -ne 0 ]] || { cat "${OUT}/render.bad.log" >&2; fail "render transform did NOT refuse a non-ok snapshot"; }
echo "  -> render refuses a non-ok snapshot (exit ${RENDER_BAD_RC})"

# The deploy guard (examples/deploy_guard.sh, ADR-0035 C2) is exercised
# end-to-end by the example's own run.sh chain and covered hermetically and
# exhaustively by the dedicated //examples:deploy_guard_test target, so it is
# not re-driven here (keeping this end-to-end test focused on the resolve ->
# bundle -> consume chain plus the two user-side render transforms).

step "DONE — example 05 non-docker chain green"
echo "compile -> matrix resolve(4) -> bundle+verify(4) -> override gen -> render(nested) -> standalone(ok) -> fail-closed"
