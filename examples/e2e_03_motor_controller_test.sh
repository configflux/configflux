#!/usr/bin/env bash
# Canonical end-to-end test for examples/03-motor-controller on the
# solver-wired product stack (configflux-g3f.4).
#
# Drives the REAL product binaries through the full pipeline:
#
#   compiler compile           -> CMP package + sibling .ccm (9hi2)
#   interpreter open           -> ModelHandle (carries ccm_ref)
#   interpreter options        -> solver valid_options for a facet (g3f.2)
#   interpreter init-state     -> initial SelectionState (encoder_type pinned)
#   interpreter select  (x2)   -> two facet applies via solver apply (g3f.2)
#   interpreter resolve        -> solver sat-gate + compiler-composed result
#   runtime runtime-open       -> RuntimeSnapshot (handoff via lineage)
#   runtime get-parameter      -> read pid_gain_trim (runtime-writable)
#   runtime set-parameter      -> write pid_gain_trim (round-trip)
#   runtime get-parameter      -> read back the value just written
#   runtime set-parameter      -> facet write rejected by solver validation
#                                 (g3f.3, CONSTRAINT_VIOLATED family)
#
# Assertions:
#   * every positive step exits 0 and reports status "ok"
#   * solver options surfaces the modeled facet option
#   * resolve emits the expected resolved config + resolve_hash, byte-equal
#     to the committed golden (examples/03-motor-controller/expected/)
#   * runtime get-parameter returns exactly what set-parameter wrote
#   * a constraint-violating runtime write (an unknown option for a real
#     model facet) is rejected with the selection/constraint error family
#
# Binaries, example sources and the golden are located via the runfiles tree
# (rootpath env vars set by the sh_test `env` attribute), mirroring the
# convention in examples/run_example_test.sh.
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
require_var RUNTIME_RLOCATION
require_var DEFS_RLOCATION
require_var COMPONENTS_RLOCATION
require_var GOLDEN_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
INTERPRETER="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
RUNTIME="${RUNFILES_ROOT}/${RUNTIME_RLOCATION}"
DEFS="${RUNFILES_ROOT}/${DEFS_RLOCATION}"
COMPONENTS="${RUNFILES_ROOT}/${COMPONENTS_RLOCATION}"
GOLDEN="${RUNFILES_ROOT}/${GOLDEN_RLOCATION}"

for bin in "${COMPILER}" "${INTERPRETER}" "${RUNTIME}"; do
  if [[ ! -x "${bin}" ]]; then
    echo "ERROR: binary not executable at ${bin}" >&2
    exit 2
  fi
done
for f in "${DEFS}" "${COMPONENTS}" "${GOLDEN}"; do
  if [[ ! -f "${f}" ]]; then
    echo "ERROR: data file not found at ${f}" >&2
    exit 2
  fi
done

OUT="${TEST_TMPDIR:-$(mktemp -d)}/out"
rm -rf "${OUT}"
mkdir -p "${OUT}"

SCOPE="all"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# jget FILE PY_EXPR  — evaluate a python expression against the parsed JSON in
# FILE (bound as `d`) and print the result. Keeps the harness dependency-free
# (no jq required, unlike example 04) and gives precise assertions.
jget() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
print(eval(sys.argv[2]))' "$1" "$2"
}

fail() {
  echo "ASSERT FAILED: $*" >&2
  exit 1
}

assert_status_ok() {
  local file="$1" label="$2"
  local status
  status="$(jget "${file}" 'd["status"]')"
  if [[ "${status}" != "ok" ]]; then
    echo "--- ${label} response ---" >&2
    cat "${file}" >&2 || true
    fail "${label}: expected status ok, got '${status}'"
  fi
}

step() { printf '\n=== %s ===\n' "$1"; }

# ---------------------------------------------------------------------------
# 1) compiler compile  (CMP package + sibling .ccm)
# ---------------------------------------------------------------------------
# The compiler derives `model_hash` from the `--source` argument STRINGS
# (source_id = the literal path) plus file content. To make `model_hash` —
# and therefore the cascading selection_state_hash / resolve_hash and the
# golden — independent of where the runfiles tree lives, stage the sources
# into the writable out dir and compile with FILENAME-ONLY source ids from
# inside it. Verified stable across distinct working directories.
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
    --out "${OUT}" \
    > "${OUT}/compile.result.json"
)
assert_status_ok "${OUT}/compile.result.json" "compile"
[[ -f "${OUT}/cmp.manifest.json" ]] || fail "compile: cmp.manifest.json missing"
[[ -f "${OUT}/ccm/ccm.symbols.json" ]] || fail "compile: sibling .ccm not emitted"
echo "  -> CMP + .ccm produced"

# ---------------------------------------------------------------------------
# 2) interpreter open
# ---------------------------------------------------------------------------
step "open"
python3 -c 'import json,sys
json.dump({"schema_version":2,"cmp_manifest_ref":sys.argv[1]},open(sys.argv[2],"w"))' \
  "${OUT}/cmp.manifest.json" "${OUT}/open.req.json"
"${INTERPRETER}" open \
  --request-file "${OUT}/open.req.json" \
  --response-file "${OUT}/open.res.json"
assert_status_ok "${OUT}/open.res.json" "open"
CCM_REF="$(jget "${OUT}/open.res.json" 'd["model_handle"]["ccm_ref"]')"
[[ -n "${CCM_REF}" ]] || fail "open: model_handle.ccm_ref empty (expected sibling .ccm)"
echo "  -> model handle obtained, ccm_ref present"

# ---------------------------------------------------------------------------
# 3) interpreter options  (solver-sourced valid_options for motor_class)
# ---------------------------------------------------------------------------
step "options (motor_class)"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1]))
json.dump({"schema_version":2,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "selection_state":{"schema_version":2,"model_hash":o["model_handle"]["model_hash"],
                             "scope":sys.argv[3],"context_tags":{},"choices":{},
                             "selection_state_hash":""},
          "facet":"motor_class"},open(sys.argv[2],"w"))' \
  "${OUT}/open.res.json" "${OUT}/options_probe.req.json" "${SCOPE}"
# Note: options is a stateless facet query; an empty selection_state_hash is
# accepted because options does not re-derive prior choices. We assert only
# that the solver enumerates the one modeled option for the facet.
"${INTERPRETER}" options \
  --request-file "${OUT}/options_probe.req.json" \
  --response-file "${OUT}/options_probe.res.json"
assert_status_ok "${OUT}/options_probe.res.json" "options"
OPTS="$(jget "${OUT}/options_probe.res.json" 'sorted(d["valid_options"])')"
[[ "${OPTS}" == "['brushed_dc']" ]] || fail "options: expected ['brushed_dc'] for motor_class, got ${OPTS}"
echo "  -> solver valid_options(motor_class) = ${OPTS}"

# ---------------------------------------------------------------------------
# 4) interpreter init-selection-state  (encoder_type pinned as context tag)
# ---------------------------------------------------------------------------
# encoder_type gates the encoder_driver override; pinning it as a context tag
# lets the full-model `all` resolve succeed with exactly TWO facet applies.
step "init-selection-state"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1]))
json.dump({"schema_version":2,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "context_tags":{"encoder_type":"absolute"}},open(sys.argv[2],"w"))' \
  "${OUT}/open.res.json" "${OUT}/init.req.json" "${SCOPE}"
"${INTERPRETER}" init-selection-state \
  --request-file "${OUT}/init.req.json" \
  --response-file "${OUT}/init.res.json"
assert_status_ok "${OUT}/init.res.json" "init-selection-state"
echo "  -> initial selection state (encoder_type=absolute pinned)"

# ---------------------------------------------------------------------------
# 5a) interpreter select  #1: motor_class=brushed_dc  (solver apply)
# ---------------------------------------------------------------------------
step "select #1 motor_class=brushed_dc"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":2,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":"motor_class","option":"brushed_dc"}},
          open(sys.argv[3],"w"))' \
  "${OUT}/open.res.json" "${OUT}/init.res.json" "${OUT}/sel1.req.json" "${SCOPE}"
"${INTERPRETER}" select \
  --request-file "${OUT}/sel1.req.json" \
  --response-file "${OUT}/sel1.res.json"
assert_status_ok "${OUT}/sel1.res.json" "select #1"
echo "  -> applied motor_class=brushed_dc"

# ---------------------------------------------------------------------------
# 5b) interpreter select  #2: power_rating=high  (solver apply)
# ---------------------------------------------------------------------------
step "select #2 power_rating=high"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":2,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":"power_rating","option":"high"}},
          open(sys.argv[3],"w"))' \
  "${OUT}/open.res.json" "${OUT}/sel1.res.json" "${OUT}/sel2.req.json" "${SCOPE}"
"${INTERPRETER}" select \
  --request-file "${OUT}/sel2.req.json" \
  --response-file "${OUT}/sel2.res.json"
assert_status_ok "${OUT}/sel2.res.json" "select #2"
CHOICES="$(jget "${OUT}/sel2.res.json" 'json.dumps(d["selection_state"]["choices"],sort_keys=True)')"
[[ "${CHOICES}" == '{"motor_class": "brushed_dc", "power_rating": "high"}' ]] \
  || fail "select #2: unexpected choices ${CHOICES}"
echo "  -> applied power_rating=high; choices=${CHOICES}"

# ---------------------------------------------------------------------------
# 6) interpreter resolve  (solver sat-gate + compiler-composed result)
# ---------------------------------------------------------------------------
step "resolve"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":2,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"]},open(sys.argv[3],"w"))' \
  "${OUT}/open.res.json" "${OUT}/sel2.res.json" "${OUT}/resolve.req.json" "${SCOPE}"
"${INTERPRETER}" resolve \
  --request-file "${OUT}/resolve.req.json" \
  --response-file "${OUT}/resolve.res.json"
assert_status_ok "${OUT}/resolve.res.json" "resolve"

# Byte-stable golden comparison. The resolve result carries no absolute paths,
# timestamps, or unordered maps (verified during authoring across independent
# output directories), so a raw byte compare is a valid determinism guard.
if ! cmp -s "${GOLDEN}" "${OUT}/resolve.res.json"; then
  echo "--- golden vs actual diff ---" >&2
  diff <(python3 -m json.tool "${GOLDEN}") \
       <(python3 -m json.tool "${OUT}/resolve.res.json") >&2 || true
  fail "resolve: output does not match committed golden (examples/03-motor-controller/expected/resolve.golden.json)"
fi
RHASH="$(jget "${OUT}/resolve.res.json" 'd["resolve_hash"]')"
echo "  -> resolve matched golden byte-for-byte; resolve_hash=${RHASH}"

# Sanity-assert the two-step overrides landed in the resolved config.
CM="$(jget "${OUT}/resolve.res.json" 'd["resolved_output"]["all"]["components"]["motor_drive"]["params"]["control_mode"]["value"]')"
CL="$(jget "${OUT}/resolve.res.json" 'd["resolved_output"]["all"]["components"]["motor_drive"]["params"]["current_limit"]["value"]')"
[[ "${CM}" == "trapezoidal" ]] || fail "resolve: control_mode override not applied (got ${CM})"
[[ "${CL}" == "30.0" ]] || fail "resolve: current_limit override not applied (got ${CL})"
echo "  -> overrides verified: control_mode=${CM}, current_limit=${CL}"

# ---------------------------------------------------------------------------
# 7) runtime runtime-open  (handoff from interpreter resolve)
# ---------------------------------------------------------------------------
step "runtime-open"
python3 -c 'import json,sys
d=json.load(open(sys.argv[1])); o=json.load(open(sys.argv[2]))
req={"schema_version":2,"model_hash":d["model_hash"],
     "ccm_ref":o["model_handle"]["ccm_ref"],"resolve_hash":d["resolve_hash"],
     "scope":d["scope"],"resolved_output":d["resolved_output"],
     "resolved_component_dependencies":d.get("resolved_component_dependencies",{}),
     "resolved_artifacts":d.get("resolved_artifacts",{}),
     "context_tags":d.get("context_tags",{}),"choices":d.get("choices",{})}
json.dump(req,open(sys.argv[3],"w"))' \
  "${OUT}/resolve.res.json" "${OUT}/open.res.json" "${OUT}/ropen.req.json"
"${RUNTIME}" runtime-open \
  --request-file "${OUT}/ropen.req.json" \
  --response-file "${OUT}/ropen.res.json"
assert_status_ok "${OUT}/ropen.res.json" "runtime-open"
echo "  -> runtime snapshot acquired"

PARAM_PATH="component.motion_controller.param.pid_gain_trim"

# ---------------------------------------------------------------------------
# 8) runtime get-parameter  (initial value of pid_gain_trim)
# ---------------------------------------------------------------------------
step "get-parameter (initial)"
python3 -c 'import json,sys
ro=json.load(open(sys.argv[1]))
json.dump({"schema_version":2,"runtime_snapshot":ro["runtime_snapshot"],
          "path":sys.argv[3]},open(sys.argv[2],"w"))' \
  "${OUT}/ropen.res.json" "${OUT}/get1.req.json" "${PARAM_PATH}"
"${RUNTIME}" get-parameter \
  --request-file "${OUT}/get1.req.json" \
  --response-file "${OUT}/get1.res.json"
assert_status_ok "${OUT}/get1.res.json" "get-parameter (initial)"
INIT_VAL="$(jget "${OUT}/get1.res.json" 'd["parameter"]["value"]')"
[[ "${INIT_VAL}" == "0.1" ]] || fail "get-parameter: expected initial 0.1, got ${INIT_VAL}"
echo "  -> initial pid_gain_trim = ${INIT_VAL}"

# ---------------------------------------------------------------------------
# 9) runtime set-parameter  (write pid_gain_trim)  + read-back round-trip
# ---------------------------------------------------------------------------
step "set-parameter (pid_gain_trim=0.42)"
python3 -c 'import json,sys
ro=json.load(open(sys.argv[1]))
json.dump({"schema_version":2,"runtime_snapshot":ro["runtime_snapshot"],
          "path":sys.argv[3],"value":0.42},open(sys.argv[2],"w"))' \
  "${OUT}/ropen.res.json" "${OUT}/set.req.json" "${PARAM_PATH}"
"${RUNTIME}" set-parameter \
  --request-file "${OUT}/set.req.json" \
  --response-file "${OUT}/set.res.json"
assert_status_ok "${OUT}/set.res.json" "set-parameter"
SET_VAL="$(jget "${OUT}/set.res.json" 'd["parameter"]["value"]')"
[[ "${SET_VAL}" == "0.42" ]] || fail "set-parameter: echoed value ${SET_VAL} != 0.42"

step "get-parameter (after set)"
# Read back from the snapshot returned by set-parameter (carries the dirty write).
python3 -c 'import json,sys
sp=json.load(open(sys.argv[1]))
json.dump({"schema_version":2,"runtime_snapshot":sp["runtime_snapshot"],
          "path":sys.argv[3]},open(sys.argv[2],"w"))' \
  "${OUT}/set.res.json" "${OUT}/get2.req.json" "${PARAM_PATH}"
"${RUNTIME}" get-parameter \
  --request-file "${OUT}/get2.req.json" \
  --response-file "${OUT}/get2.res.json"
assert_status_ok "${OUT}/get2.res.json" "get-parameter (after set)"
RT_VAL="$(jget "${OUT}/get2.res.json" 'd["parameter"]["value"]')"
[[ "${RT_VAL}" == "0.42" ]] || fail "round-trip: get returned ${RT_VAL}, expected 0.42"
echo "  -> set/get round-trip confirmed: pid_gain_trim = ${RT_VAL}"

# ---------------------------------------------------------------------------
# 10) runtime set-parameter  — constraint-violating facet write is REJECTED
# ---------------------------------------------------------------------------
# The runtime set-parameter handler runs the solver option-validity pre-check
# (g3f.3): a write whose param_key names a real model facet is adjudicated by
# solver::Session::valid_options before any compiler write. Writing an option
# the facet does not admit ('nonexistent_option' for the 'power_rating' facet)
# must be rejected with the selection/constraint error family, snapshot
# unchanged. This is the runtime CONSTRAINT_VIOLATED path the example supports.
step "set-parameter (constraint violation, expect rejection)"
BAD_PATH="component.motor_drive.param.power_rating"
python3 -c 'import json,sys
ro=json.load(open(sys.argv[1]))
json.dump({"schema_version":2,"runtime_snapshot":ro["runtime_snapshot"],
          "path":sys.argv[3],"value":"nonexistent_option"},open(sys.argv[2],"w"))' \
  "${OUT}/ropen.res.json" "${OUT}/badset.req.json" "${BAD_PATH}"
set +e
"${RUNTIME}" set-parameter \
  --request-file "${OUT}/badset.req.json" \
  --response-file "${OUT}/badset.res.json"
BAD_RC=$?
set -e
[[ ${BAD_RC} -eq 2 ]] || fail "constraint set: expected exit 2 (command error), got ${BAD_RC}"
BAD_STATUS="$(jget "${OUT}/badset.res.json" 'd["status"]')"
[[ "${BAD_STATUS}" == "error" ]] || fail "constraint set: expected status error, got ${BAD_STATUS}"
BAD_CODE="$(jget "${OUT}/badset.res.json" 'd["diagnostics"]["diagnostics"][0]["code"]')"
case "${BAD_CODE}" in
  E_SELECTION_INVALID_OPTION|E_SELECTION_CONFLICT|E_SELECTION_UNSATISFIABLE|E_SELECTION_UNKNOWN_FACET)
    ;;
  *)
    echo "--- rejection response ---" >&2
    cat "${OUT}/badset.res.json" >&2 || true
    fail "constraint set: expected a selection/constraint error code, got ${BAD_CODE}"
    ;;
esac
echo "  -> constraint-violating write rejected: ${BAD_CODE}"

step "DONE — full solver-path E2E green"
echo "compile -> open -> options -> init -> select x2 -> resolve(golden) -> runtime-open -> get/set/get -> constraint-reject"
