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
#   runtime set-parameter      -> write to encoder_mode REFUSED: it declares
#                                 `facet: encoder_type`, so the write is a
#                                 facet selection the model's constraints
#                                 govern (05hm, ADR-0064)
#   runtime set-parameter      -> control: a path that is no parameter at all
#                                 is refused earlier, by path validation
#
# Assertions:
#   * every positive step exits 0 and reports status "ok"
#   * solver options surfaces the modeled facet option
#   * resolve emits the expected resolved config + resolve_hash, byte-equal
#     to the committed golden (examples/03-motor-controller/expected/)
#   * runtime get-parameter returns exactly what set-parameter wrote
#   * a write that violates a DECLARED constraint exits 2 with status "error",
#     code E_SELECTION_CONFLICT, entity_path constraints/<id>, an unsat_core
#     naming that constraint and quoting its condition, and NO snapshot
#   * the negative control keeps its own verdict: a write to a path that is
#     not a parameter is still E_RUNTIME_UNKNOWN_PATH
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
# `model_hash` covers the CONTENT of the sources and nothing else (ADR-0056),
# so the golden holds wherever the runfiles tree lives and the sources can be
# compiled in place under their real paths. This step used to stage copies and
# compile with filename-only source ids, because the hash once covered the
# `--source` argument strings too; that scaffolding is gone. Compiling straight
# from the runfiles paths now also witnesses the invariant — these are absolute
# and sandbox-specific, and the golden below pins the hashes they produce.
step "compile"
"${COMPILER}" compile \
  --source "${DEFS}" \
  --source "${COMPONENTS}" \
  --out "${OUT}" \
  > "${OUT}/compile.result.json"
assert_status_ok "${OUT}/compile.result.json" "compile"
[[ -f "${OUT}/cmp.manifest.json" ]] || fail "compile: cmp.manifest.json missing"
[[ -f "${OUT}/ccm/ccm.symbols.json" ]] || fail "compile: sibling .ccm not emitted"
echo "  -> CMP + .ccm produced"

# ---------------------------------------------------------------------------
# 2) interpreter open
# ---------------------------------------------------------------------------
step "open"
python3 -c 'import json,sys
json.dump({"schema_version":5,"cmp_manifest_ref":sys.argv[1]},open(sys.argv[2],"w"))' \
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
# options validates the selection_state's canonical hash, so derive a canonical
# empty state first (this probe carries no context and no prior choices).
python3 -c 'import json,sys
o=json.load(open(sys.argv[1]))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "context_tags":{}},open(sys.argv[2],"w"))' \
  "${OUT}/open.res.json" "${OUT}/options_init.req.json" "${SCOPE}"
"${INTERPRETER}" init-selection-state \
  --request-file "${OUT}/options_init.req.json" \
  --response-file "${OUT}/options_init.res.json"
assert_status_ok "${OUT}/options_init.res.json" "options init"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "facet":"motor_class"},open(sys.argv[3],"w"))' \
  "${OUT}/open.res.json" "${OUT}/options_init.res.json" "${OUT}/options_probe.req.json" "${SCOPE}"
# motor_class is a declared closed facet (ADR-0047, Amendment 1). Both declared
# arms are enumerated in valid_options: brushed_dc is named by a condition, and
# brushless_dc — the declared default arm no condition names — is now a
# first-class selectable option too (symbol-introduction only, no BDD mutex, so
# the forced condition arm no longer prunes the unforced default). The default
# arm is additionally surfaced through the response's `default` annotation.
"${INTERPRETER}" options \
  --request-file "${OUT}/options_probe.req.json" \
  --response-file "${OUT}/options_probe.res.json"
assert_status_ok "${OUT}/options_probe.res.json" "options"
OPTS="$(jget "${OUT}/options_probe.res.json" 'sorted(d["valid_options"])')"
[[ "${OPTS}" == "['brushed_dc', 'brushless_dc']" ]] || fail "options: expected ['brushed_dc', 'brushless_dc'] for motor_class, got ${OPTS}"
DEF="$(jget "${OUT}/options_probe.res.json" 'd.get("default")')"
[[ "${DEF}" == "brushless_dc" ]] || fail "options: expected declared default brushless_dc for motor_class, got ${DEF}"
echo "  -> solver valid_options(motor_class) = ${OPTS}; declared default = ${DEF}"

# ---------------------------------------------------------------------------
# 4) interpreter init-selection-state  (encoder_type pinned as context tag)
# ---------------------------------------------------------------------------
# encoder_type gates the encoder_driver override; pinning it as a context tag
# lets the full-model `all` resolve succeed with exactly TWO facet applies.
step "init-selection-state"
python3 -c 'import json,sys
o=json.load(open(sys.argv[1]))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[3],
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
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
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
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
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
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
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
req={"schema_version":5,"model_hash":d["model_hash"],
     "ccm_ref":o["model_handle"]["ccm_ref"],"resolve_hash":d["resolve_hash"],
     "scope":d["scope"],"resolved_output":d["resolved_output"],
     "resolved_component_dependencies":d.get("resolved_component_dependencies",{}),
     "resolved_artifacts":d.get("resolved_artifacts",{}),
     "context_tags":d.get("context_tags",{}),"choices":d.get("choices",{}),
     "defaulted_choices":d.get("defaulted_choices",{}),
     "implied_choices":d.get("implied_choices",{}),
     "closed_facet_domains":d.get("closed_facet_domains",{})}
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
json.dump({"schema_version":5,"runtime_snapshot":ro["runtime_snapshot"],
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
json.dump({"schema_version":5,"runtime_snapshot":ro["runtime_snapshot"],
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
json.dump({"schema_version":5,"runtime_snapshot":sp["runtime_snapshot"],
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
# 10) runtime set-parameter  — a write the MODEL forbids is REJECTED
# ---------------------------------------------------------------------------
# `encoder_interface.encoder_mode` declares `facet: encoder_type`, which is what
# makes it that facet's runtime handle (ADR-0064): writing it is a selection of
# encoder_type, not a free-form scalar poke, so the model's declared
# constraints govern it. This session already holds `power_rating=high`, and
# the model declares
#
#   high_power_requires_absolute_encoder:
#     power_rating != 'high' || encoder_type != 'incremental'
#
# so writing `incremental` makes the session's total assignment unsatisfiable.
# The write is issued on the snapshot returned by step 9, which carries the
# accepted pid_gain_trim write — a later write is checked against what earlier
# writes established, not against the state at open.
#
# The assertions mirror the in-process contract (runtime/src/tests.rs,
# `assert_constraint_rejected` and the named-core cases): exit 2, status error,
# E_SELECTION_CONFLICT, entity_path `constraints/<id>`, an unsat_core naming
# the violated constraint and quoting its condition, and NO snapshot returned.
step "set-parameter (violates a declared constraint, expect rejection)"
FACET_PATH="component.encoder_interface.param.encoder_mode"
RULE_ID="high_power_requires_absolute_encoder"
RULE_TEXT="power_rating != 'high' || encoder_type != 'incremental'"
python3 -c 'import json,sys
sp=json.load(open(sys.argv[1]))
json.dump({"schema_version":5,"runtime_snapshot":sp["runtime_snapshot"],
          "path":sys.argv[3],"value":"incremental"},open(sys.argv[2],"w"))' \
  "${OUT}/set.res.json" "${OUT}/conflict.req.json" "${FACET_PATH}"
set +e
"${RUNTIME}" set-parameter \
  --request-file "${OUT}/conflict.req.json" \
  --response-file "${OUT}/conflict.res.json"
CONFLICT_RC=$?
set -e
[[ ${CONFLICT_RC} -eq 2 ]] || fail "constraint set: expected exit 2 (command error), got ${CONFLICT_RC}"
CONFLICT_STATUS="$(jget "${OUT}/conflict.res.json" 'd["status"]')"
[[ "${CONFLICT_STATUS}" == "error" ]] || fail "constraint set: expected status error, got ${CONFLICT_STATUS}"
CONFLICT_CODE="$(jget "${OUT}/conflict.res.json" 'd["diagnostics"]["diagnostics"][0]["code"]')"
if [[ "${CONFLICT_CODE}" != "E_SELECTION_CONFLICT" ]]; then
  echo "--- rejection response ---" >&2
  cat "${OUT}/conflict.res.json" >&2 || true
  fail "constraint set: expected E_SELECTION_CONFLICT, got ${CONFLICT_CODE}"
fi
CONFLICT_ENTITY="$(jget "${OUT}/conflict.res.json" 'd["diagnostics"]["diagnostics"][0]["entity_path"]')"
[[ "${CONFLICT_ENTITY}" == "constraints/${RULE_ID}" ]] \
  || fail "constraint set: expected entity_path constraints/${RULE_ID}, got ${CONFLICT_ENTITY}"
# A rejected write changes nothing, so no snapshot comes back.
HAS_SNAPSHOT="$(jget "${OUT}/conflict.res.json" 'd.get("runtime_snapshot") is not None')"
[[ "${HAS_SNAPSHOT}" == "False" ]] || fail "constraint set: a rejected write must return no snapshot"
# The core names the rejected selection and the violated rule, with its
# condition text quoted verbatim from the model.
CORE_REJECTED="$(jget "${OUT}/conflict.res.json" 'json.dumps(d["unsat_core"]["rejected"],sort_keys=True)')"
[[ "${CORE_REJECTED}" == '{"facet": "encoder_type", "option": "incremental"}' ]] \
  || fail "constraint set: unexpected unsat_core.rejected ${CORE_REJECTED}"
python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
named=[(c.get("constraint_id"), c["summary"])
       for c in d["unsat_core"]["conflicting_constraints"] if c["kind"] == "model_rule"]
if named != [(sys.argv[2], sys.argv[3])]:
    sys.exit("model_rule clauses %r do not name %s and quote its condition"
             % (named, sys.argv[2]))' \
  "${OUT}/conflict.res.json" "${RULE_ID}" "${RULE_TEXT}" \
  || { cat "${OUT}/conflict.res.json" >&2 || true; fail "constraint set: unsat_core core mismatch"; }
echo "  -> write refused by the model: ${CONFLICT_CODE} at ${CONFLICT_ENTITY}"
echo "  -> unsat_core names ${RULE_ID}: ${RULE_TEXT}"

# ---------------------------------------------------------------------------
# 11) NEGATIVE CONTROL — a path that is not a parameter is refused EARLIER
# ---------------------------------------------------------------------------
# 'power_rating' is declared as a FACET (00_definitions.json) and appears in
# this model only inside `condition` strings — motor_drive's actual params are
# control_mode / current_limit / motor_driver / pwm_frequency. There is no such
# parameter to write, so the compiler's own path validation refuses it before
# the solver is consulted at all (the D1 ordering configflux-jraj established)
# and the code is E_RUNTIME_UNKNOWN_PATH, whatever value is supplied.
#
# Keeping it beside step 10 is the point: the two refusals must stay
# distinguishable. A regression that made every refused write look like a
# policy violation — or every policy violation look like a bad path — would
# pass either assertion alone.
step "set-parameter (path is not a parameter, expect unknown-path rejection)"
BAD_PATH="component.motor_drive.param.power_rating"
python3 -c 'import json,sys
ro=json.load(open(sys.argv[1]))
json.dump({"schema_version":5,"runtime_snapshot":ro["runtime_snapshot"],
          "path":sys.argv[3],"value":"nonexistent_option"},open(sys.argv[2],"w"))' \
  "${OUT}/ropen.res.json" "${OUT}/badset.req.json" "${BAD_PATH}"
set +e
"${RUNTIME}" set-parameter \
  --request-file "${OUT}/badset.req.json" \
  --response-file "${OUT}/badset.res.json"
BAD_RC=$?
set -e
[[ ${BAD_RC} -eq 2 ]] || fail "bad-path set: expected exit 2 (command error), got ${BAD_RC}"
BAD_STATUS="$(jget "${OUT}/badset.res.json" 'd["status"]')"
[[ "${BAD_STATUS}" == "error" ]] || fail "bad-path set: expected status error, got ${BAD_STATUS}"
BAD_CODE="$(jget "${OUT}/badset.res.json" 'd["diagnostics"]["diagnostics"][0]["code"]')"
if [[ "${BAD_CODE}" != "E_RUNTIME_UNKNOWN_PATH" ]]; then
  echo "--- rejection response ---" >&2
  cat "${OUT}/badset.res.json" >&2 || true
  fail "bad-path set: expected E_RUNTIME_UNKNOWN_PATH, got ${BAD_CODE}"
fi
echo "  -> write to a non-parameter path rejected: ${BAD_CODE}"

step "DONE — full solver-path E2E green"
echo "compile -> open -> options -> init -> select x2 -> resolve(golden) -> runtime-open -> get/set/get -> constraint-reject (with an unknown-path control)"
