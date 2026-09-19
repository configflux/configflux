#!/usr/bin/env bash
# Process-level test for the runtime v2 SESSION LIFECYCLE — the "Pattern 2"
# flow of docs/service-integration-guide.md — driven through the runtime
# BINARY (configflux-x5gb.6, ADR-0061 D3).
#
# The journey a technician performs in the field: resolve a configuration, open
# a session over it, trim a gain (the write goes DIRTY), inspect and attribute
# the deviation, roll it back, trim again for keeps, commit it, be refused a
# jointly invalid batch, and re-time the abandoned-experiment sweep.
#
# WHY THIS EXISTS: all 14 v2 commands were covered only IN PROCESS, from
# runtime/src/tests.rs. That misses the one property the CLI alone has to hold
# — the runtime binary is STATELESS per process, so the entire session lives in
# the `runtime_snapshot` the caller threads out of each response and into the
# next request. Lose that thread and the session silently reverts to the
# committed configuration; a REJECTED command returns no snapshot at all, so
# the caller must keep the previous one. Only a process-level test sees that.
#
# Example 03 is the vehicle because it is the only shipped example carrying
# both halves of the pair this needs: motion_controller.pid_gain_trim is
# lifecycle=runtime (writable in the field) and motor_drive.control_mode is
# lifecycle=construction (frozen at build time), so a batch naming both is
# individually plausible and jointly invalid.
#
# CONTRACT NOTE: field names below are shipped wire names, taken from
# compiler/src/runtime_api/contracts.rs and runtime/src/tests.rs. Section 6 of
# docs/runtime-v2-contract.md agrees with them, pinned by
# //compiler:runtime_v2_contract_doc_section6_test, which round-trips that
# section's fenced examples through those structs, not this script. Neither
# spelling below is a typo: `runtime-open` sends `RuntimeOpenRequest.scope`,
# `list-dirty-parameters` sends `ListDirtyParametersRequest.scope_root`.
#
# python3 does every JSON shaping and every assertion; no jq.
set -euo pipefail

# ---------------------------------------------------------------------------
# Runfiles resolution
# ---------------------------------------------------------------------------
if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi
RUNFILES_ROOT="${TEST_SRCDIR}/_main"

# One <NAME>=<runfiles path> per required <NAME>_RLOCATION.
for name in COMPILER INTERPRETER RUNTIME DEFS COMPONENTS; do
  rlocation="${name}_RLOCATION"
  if [[ -z "${!rlocation:-}" ]]; then
    echo "ERROR: ${rlocation} env var is not set" >&2
    exit 2
  fi
  declare "${name}=${RUNFILES_ROOT}/${!rlocation}"
done

for bin in "${COMPILER}" "${INTERPRETER}" "${RUNTIME}"; do
  [[ -x "${bin}" ]] || { echo "ERROR: binary not executable at ${bin}" >&2; exit 2; }
done
for f in "${DEFS}" "${COMPONENTS}"; do
  [[ -f "${f}" ]] || { echo "ERROR: data file not found at ${f}" >&2; exit 2; }
done

OUT="${TEST_TMPDIR:-$(mktemp -d)}/out"
rm -rf "${OUT}"
mkdir -p "${OUT}"

# ---------------------------------------------------------------------------
# Model constants (examples/03-motor-controller)
# ---------------------------------------------------------------------------
SCHEMA_VERSION=5
SCOPE="all"
# Example 03 resolves a single scope, so scope root and resolve scope coincide;
# two names because they are different concepts.
SCOPE_ROOT="all"
PID_PATH="component.motion_controller.param.pid_gain_trim"
# Dirty/rolled-back/changed paths come back SCOPE-QUALIFIED as
# "<scope_root>/<path>", while rejected_paths echoes the raw path the caller
# supplied. Both forms are pinned below, deliberately.
CANONICAL_PID_PATH="${SCOPE_ROOT}/${PID_PATH}"
IMMUTABLE_PATH="component.motor_drive.param.control_mode"
TRIM_VALUE=0.42
ACTOR="lifecycle-test"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
fail() { echo "ASSERT FAILED: $*" >&2; exit 1; }

step() { printf '\n=== %s ===\n' "$1"; }

# jget FILE PY_EXPR — evaluate PY_EXPR against the JSON in FILE (bound as `d`),
# mirroring examples/e2e_03_motor_controller_test.sh.
jget() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
print(eval(sys.argv[2]))' "$1" "$2"
}

# pycheck LABEL PY_CODE ARG... — run PY_CODE with the ARGs in sys.argv[1:]; the
# snippet fails via sys.exit("message"). List, float and hash comparisons live
# here rather than in bash, where they would be stringly typed and would
# silently pass on a formatting change.
pycheck() {
  local label="$1"
  shift
  python3 -c "$@" || fail "${label}"
}

# assert_status LABEL FILE EXPECTED — the envelope's own status field.
assert_status() {
  local label="$1" file="$2" expected="$3" status
  status="$(jget "${file}" 'd["status"]')"
  if [[ "${status}" != "${expected}" ]]; then
    echo "--- ${label} response ---" >&2
    cat "${file}" >&2 || true
    fail "${label}: expected status ${expected}, got '${status}'"
  fi
}

# rt COMMAND REQ RES EXPECTED_EXIT — run one runtime command as its own process
# and pin its exit code (docs/runtime-cli-contract.md section 4: 0 = status ok,
# 2 = status error, 1 = transport failure). The distinction matters: a refused
# write must be a 2 — a well-formed envelope carrying diagnostics — never a 1.
rt() {
  local command="$1" req="$2" res="$3" expected="$4" rc=0
  set +e
  "${RUNTIME}" "${command}" --request-file "${req}" --response-file "${res}"
  rc=$?
  set -e
  if [[ ${rc} -ne ${expected} ]]; then
    echo "--- ${command} response ---" >&2
    cat "${res}" >&2 || true
    fail "${command}: expected exit ${expected}, got ${rc}"
  fi
}

# req_from SNAP_RES OUT_REQ [PY_DICT_LITERAL] — build the next request from the
# `runtime_snapshot` the previous response carried, plus the given fields.
# Threading that snapshot IS the session: there is no server-side state.
req_from() {
  python3 -c 'import ast,json,sys
src=json.load(open(sys.argv[1]))
snapshot=src.get("runtime_snapshot")
if snapshot is None:
    sys.exit("no runtime_snapshot in " + sys.argv[1])
request={"schema_version":'"${SCHEMA_VERSION}"',"runtime_snapshot":snapshot}
if len(sys.argv) > 3:
    request.update(ast.literal_eval(sys.argv[3]))
json.dump(request,open(sys.argv[2],"w"))' "$1" "$2" ${3:+"$3"}
}

# read_parameter SNAP_RES TAG — get-parameter through the binary; every read
# re-opens the session from the threaded snapshot.
read_parameter() {
  local snap_res="$1" tag="$2"
  req_from "${snap_res}" "${OUT}/get_${tag}.req.json" "{'path': '${PID_PATH}'}"
  rt get-parameter "${OUT}/get_${tag}.req.json" "${OUT}/get_${tag}.res.json" 0
  assert_status "get-parameter (${tag})" "${OUT}/get_${tag}.res.json" ok
  jget "${OUT}/get_${tag}.res.json" 'd["parameter"]["value"]'
}

# identity SNAP_RES TAG — get-configuration-identity through the binary.
identity() {
  local snap_res="$1" tag="$2"
  req_from "${snap_res}" "${OUT}/id_${tag}.req.json"
  rt get-configuration-identity "${OUT}/id_${tag}.req.json" "${OUT}/id_${tag}.res.json" 0
  assert_status "get-configuration-identity (${tag})" "${OUT}/id_${tag}.res.json" ok
}

# list_dirty SNAP_RES TAG — list-dirty-parameters for the single scope root.
list_dirty() {
  local snap_res="$1" tag="$2"
  req_from "${snap_res}" "${OUT}/dirty_${tag}.req.json" "{'scope_root': '${SCOPE_ROOT}'}"
  rt list-dirty-parameters "${OUT}/dirty_${tag}.req.json" "${OUT}/dirty_${tag}.res.json" 0
  assert_status "list-dirty-parameters (${tag})" "${OUT}/dirty_${tag}.res.json" ok
}

# ---------------------------------------------------------------------------
# S1) compile + interpreter chain: open -> init-selection-state -> select x2 ->
#     resolve. Shaping copied from examples/e2e_03_motor_controller_test.sh.
# ---------------------------------------------------------------------------
step "S1 compile + resolve (interpreter chain)"
"${COMPILER}" compile --source "${DEFS}" --source "${COMPONENTS}" --out "${OUT}" \
  > "${OUT}/compile.result.json"
assert_status "compile" "${OUT}/compile.result.json" ok

python3 -c 'import json,sys
json.dump({"schema_version":int(sys.argv[3]),"cmp_manifest_ref":sys.argv[1]},open(sys.argv[2],"w"))' \
  "${OUT}/cmp.manifest.json" "${OUT}/open.req.json" "${SCHEMA_VERSION}"
"${INTERPRETER}" open --request-file "${OUT}/open.req.json" --response-file "${OUT}/open.res.json"
assert_status "open" "${OUT}/open.res.json" ok

# encoder_type gates the encoder_driver override; pinning it as a context tag
# lets the `all` resolve succeed with exactly TWO facet applies.
python3 -c 'import json,sys
o=json.load(open(sys.argv[1]))
json.dump({"schema_version":int(sys.argv[4]),"model_handle":o["model_handle"],"scope":sys.argv[3],
          "context_tags":{"encoder_type":"absolute"}},open(sys.argv[2],"w"))' \
  "${OUT}/open.res.json" "${OUT}/init.req.json" "${SCOPE}" "${SCHEMA_VERSION}"
"${INTERPRETER}" init-selection-state \
  --request-file "${OUT}/init.req.json" --response-file "${OUT}/init.res.json"
assert_status "init-selection-state" "${OUT}/init.res.json" ok

select_facet() {
  local prev="$1" out="$2" facet="$3" option="$4"
  python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":int(sys.argv[7]),"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":sys.argv[5],"option":sys.argv[6]}},
          open(sys.argv[3],"w"))' \
    "${OUT}/open.res.json" "${prev}" "${OUT}/${out}.req.json" "${SCOPE}" \
    "${facet}" "${option}" "${SCHEMA_VERSION}"
  "${INTERPRETER}" select \
    --request-file "${OUT}/${out}.req.json" --response-file "${OUT}/${out}.res.json"
  assert_status "select ${facet}=${option}" "${OUT}/${out}.res.json" ok
}
select_facet "${OUT}/init.res.json" sel1 motor_class brushed_dc
select_facet "${OUT}/sel1.res.json" sel2 power_rating high

python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":int(sys.argv[5]),"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"]},open(sys.argv[3],"w"))' \
  "${OUT}/open.res.json" "${OUT}/sel2.res.json" "${OUT}/resolve.req.json" "${SCOPE}" "${SCHEMA_VERSION}"
"${INTERPRETER}" resolve \
  --request-file "${OUT}/resolve.req.json" --response-file "${OUT}/resolve.res.json"
assert_status "resolve" "${OUT}/resolve.res.json" ok
echo "  -> resolved configuration ready for handoff"

# ---------------------------------------------------------------------------
# S2) runtime-open — the handoff from interpreter resolve into a session.
# ---------------------------------------------------------------------------
# runtime_open REQ RES — shaping copied from e2e_03 step 7: `ccm_ref` from the
# interpreter's model handle, everything else from the resolve result.
runtime_open() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1])); o=json.load(open(sys.argv[2]))
request={"schema_version":int(sys.argv[4]),"model_hash":d["model_hash"],
     "ccm_ref":o["model_handle"]["ccm_ref"],"resolve_hash":d["resolve_hash"],
     "scope":d["scope"],"resolved_output":d["resolved_output"],
     "resolved_component_dependencies":d.get("resolved_component_dependencies",{}),
     "resolved_artifacts":d.get("resolved_artifacts",{}),
     "context_tags":d.get("context_tags",{}),"choices":d.get("choices",{}),
     "defaulted_choices":d.get("defaulted_choices",{}),
     "implied_choices":d.get("implied_choices",{}),
     "closed_facet_domains":d.get("closed_facet_domains",{})}
json.dump(request,open(sys.argv[3],"w"))' \
    "${OUT}/resolve.res.json" "${OUT}/open.res.json" "$1" "${SCHEMA_VERSION}"
  rt runtime-open "$1" "$2" 0
  assert_status "runtime-open" "$2" ok
}

step "S2 runtime-open + initial read"
runtime_open "${OUT}/ropen.req.json" "${OUT}/ropen.res.json"
INITIAL="$(read_parameter "${OUT}/ropen.res.json" initial)"
# Guard against a vacuous rollback assertion in S5: if the committed value were
# already the value S4 writes, restoring it would prove nothing.
[[ "${INITIAL}" != "${TRIM_VALUE}" ]] \
  || fail "committed value is already ${TRIM_VALUE}; S5's rollback assertion would be vacuous"
echo "  -> session open; committed ${PID_PATH} = ${INITIAL}"

# ---------------------------------------------------------------------------
# S3) get-configuration-identity — the clean baseline.
# ---------------------------------------------------------------------------
step "S3 identity baseline"
identity "${OUT}/ropen.res.json" base
pycheck "identity: a session with no dirty state must have working == committed" '
import json,sys
i=json.load(open(sys.argv[1]))["identity"]
if i["working_configuration_id"] != i["committed_configuration_id"]:
    sys.exit("working %s != committed %s on a clean session"
             % (i["working_configuration_id"], i["committed_configuration_id"]))
' "${OUT}/id_base.res.json"
echo "  -> working == committed on a clean session"

# ---------------------------------------------------------------------------
# S4/S5) step_rollback — a dirty write is observable, attributed, and reversible.
# ---------------------------------------------------------------------------
step_rollback() {
  step "S4 dirty write"
  req_from "${OUT}/ropen.res.json" "${OUT}/set1.req.json" \
    "{'path': '${PID_PATH}', 'value': ${TRIM_VALUE}}"
  rt set-parameter "${OUT}/set1.req.json" "${OUT}/set1.res.json" 0
  assert_status "set-parameter (dirty write)" "${OUT}/set1.res.json" ok
  pycheck "set-parameter must echo the value it accepted" '
import json,sys
value=json.load(open(sys.argv[1]))["parameter"]["value"]
if float(value) != float(sys.argv[2]):
    sys.exit("echoed %r, expected %s" % (value, sys.argv[2]))
' "${OUT}/set1.res.json" "${TRIM_VALUE}"

  list_dirty "${OUT}/set1.res.json" after_write
  pycheck "list-dirty-parameters must name exactly the written path" '
import json,sys
d=json.load(open(sys.argv[1]))
if d["dirty_paths"] != [sys.argv[2]]:
    sys.exit("dirty_paths %r, expected [%r]" % (d["dirty_paths"], sys.argv[2]))
if d["dirty_count"] != 1:
    sys.exit("dirty_count %r, expected 1" % (d["dirty_count"],))
' "${OUT}/dirty_after_write.res.json" "${CANONICAL_PID_PATH}"

  req_from "${OUT}/set1.res.json" "${OUT}/meta.req.json" "{'path': '${PID_PATH}'}"
  rt get-dirty-metadata "${OUT}/meta.req.json" "${OUT}/meta.res.json" 0
  assert_status "get-dirty-metadata" "${OUT}/meta.res.json" ok
  pycheck "get-dirty-metadata must report the path dirty with a real generation" '
import json,sys
d=json.load(open(sys.argv[1]))
if d["dirty"] is not True:
    sys.exit("dirty %r, expected True" % (d["dirty"],))
generation=d["metadata"]["generation"]
if generation < 1:
    sys.exit("dirty generation %r, expected >= 1" % (generation,))
' "${OUT}/meta.res.json"

  identity "${OUT}/set1.res.json" dirty
  pycheck "a dirty session must diverge from its committed identity" '
import json,sys
base=json.load(open(sys.argv[1]))["identity"]
dirty=json.load(open(sys.argv[2]))["identity"]
if dirty["working_configuration_id"] == dirty["committed_configuration_id"]:
    sys.exit("working == committed while a path is dirty")
if dirty["dirty_diff_hash"] == base["dirty_diff_hash"]:
    sys.exit("dirty_diff_hash did not move for a dirty write")
' "${OUT}/id_base.res.json" "${OUT}/id_dirty.res.json"
  echo "  -> ${CANONICAL_PID_PATH} dirty; identity diverged"

  step "S5 rollback-dirty (mode all)"
  req_from "${OUT}/set1.res.json" "${OUT}/rollback.req.json" \
    "{'mode': 'all', 'actor': '${ACTOR}'}"
  rt rollback-dirty "${OUT}/rollback.req.json" "${OUT}/rollback.res.json" 0
  assert_status "rollback-dirty" "${OUT}/rollback.res.json" ok
  pycheck "rollback must name what it reverted and leave nothing behind" '
import json,sys
d=json.load(open(sys.argv[1]))
if d["rolled_back_paths"] != [sys.argv[2]]:
    sys.exit("rolled_back_paths %r, expected [%r]" % (d["rolled_back_paths"], sys.argv[2]))
if d["remaining_dirty_paths"] != []:
    sys.exit("remaining_dirty_paths %r, expected []" % (d["remaining_dirty_paths"],))
' "${OUT}/rollback.res.json" "${CANONICAL_PID_PATH}"

  local restored
  restored="$(read_parameter "${OUT}/rollback.res.json" restored)"
  [[ "${restored}" == "${INITIAL}" ]] \
    || fail "rollback: value is ${restored}, expected the committed ${INITIAL}"
  list_dirty "${OUT}/rollback.res.json" after_rollback
  pycheck "rollback must clear the dirty set" '
import json,sys
count=json.load(open(sys.argv[1]))["dirty_count"]
if count != 0:
    sys.exit("dirty_count %r after rollback, expected 0" % (count,))
' "${OUT}/dirty_after_rollback.res.json"
  echo "  -> rolled back to the committed ${restored}; dirty set empty"
}
step_rollback

# ---------------------------------------------------------------------------
# S6) step_commit — committing promotes the deviation and advances identity.
# ---------------------------------------------------------------------------
step_commit() {
  step "S6 commit-configuration"
  req_from "${OUT}/rollback.res.json" "${OUT}/set2.req.json" \
    "{'path': '${PID_PATH}', 'value': ${TRIM_VALUE}}"
  rt set-parameter "${OUT}/set2.req.json" "${OUT}/set2.res.json" 0
  assert_status "set-parameter (pre-commit)" "${OUT}/set2.res.json" ok

  req_from "${OUT}/set2.res.json" "${OUT}/commit.req.json" \
    "{'actor': '${ACTOR}', 'reason': 'persist trim'}"
  rt commit-configuration "${OUT}/commit.req.json" "${OUT}/commit.res.json" 0
  assert_status "commit-configuration" "${OUT}/commit.res.json" ok
  pycheck "commit must report an id, one changed path, and a moved target" '
import json,sys
d=json.load(open(sys.argv[1]))
if not d.get("commit_id"):
    sys.exit("commit_id is empty")
changed=d["changed_paths"]
if len(changed) != 1:
    sys.exit("changed_paths has %d entries, expected 1: %r" % (len(changed), changed))
entry=changed[0]
if entry["path"] != sys.argv[2]:
    sys.exit("changed path %r, expected %r" % (entry["path"], sys.argv[2]))
if entry["before_leaf_hash"] == entry["after_leaf_hash"]:
    sys.exit("before_leaf_hash == after_leaf_hash for a value that changed")
if d["target_configuration_id"] == d["base_configuration_id"]:
    sys.exit("target_configuration_id == base_configuration_id after a commit")
if not d.get("delta_manifest"):
    sys.exit("commit carried no delta_manifest")
' "${OUT}/commit.res.json" "${CANONICAL_PID_PATH}"

  identity "${OUT}/commit.res.json" committed
  pycheck "after commit the session is clean again at the new configuration" '
import json,sys
base=json.load(open(sys.argv[1]))["identity"]
after=json.load(open(sys.argv[2]))["identity"]
target=json.load(open(sys.argv[3]))["target_configuration_id"]
if after["committed_configuration_id"] != after["working_configuration_id"]:
    sys.exit("working %s != committed %s after commit"
             % (after["working_configuration_id"], after["committed_configuration_id"]))
if after["committed_configuration_id"] != target:
    sys.exit("committed %s is not the commit target %s"
             % (after["committed_configuration_id"], target))
if after["dirty_diff_hash"] != base["dirty_diff_hash"]:
    sys.exit("dirty_diff_hash %s did not return to the clean baseline %s"
             % (after["dirty_diff_hash"], base["dirty_diff_hash"]))
' "${OUT}/id_base.res.json" "${OUT}/id_committed.res.json" "${OUT}/commit.res.json"

  list_dirty "${OUT}/commit.res.json" after_commit
  pycheck "commit must clear the dirty set" '
import json,sys
count=json.load(open(sys.argv[1]))["dirty_count"]
if count != 0:
    sys.exit("dirty_count %r after commit, expected 0" % (count,))
' "${OUT}/dirty_after_commit.res.json"

  local committed_value
  committed_value="$(read_parameter "${OUT}/commit.res.json" committed)"
  [[ "${committed_value}" == "${TRIM_VALUE}" ]] \
    || fail "commit: value is ${committed_value}, expected ${TRIM_VALUE}"
  echo "  -> committed ${PID_PATH} = ${committed_value}; identity advanced, dirty set empty"
}
step_commit

# ---------------------------------------------------------------------------
# S7) step_atomic_reject — a jointly invalid batch is refused AS A WHOLE.
# ---------------------------------------------------------------------------
# The first write is legal on its own; the second names a lifecycle=construction
# parameter. All-or-nothing means the legal half must leave no trace either —
# the property this step pins, visible only by reading the parameter back.
step_atomic_reject() {
  step "S7 set-parameters-atomically (jointly invalid batch)"
  req_from "${OUT}/commit.res.json" "${OUT}/atomic.req.json" \
    "{'writes': [{'path': '${PID_PATH}', 'value': 0.5},
                 {'path': '${IMMUTABLE_PATH}', 'value': 'foc'}],
      'actor': '${ACTOR}'}"
  rt set-parameters-atomically "${OUT}/atomic.req.json" "${OUT}/atomic.res.json" 2
  assert_status "set-parameters-atomically" "${OUT}/atomic.res.json" error
  pycheck "a refused batch must apply nothing, return no snapshot, and name the offender" '
import json,sys
d=json.load(open(sys.argv[1]))
offender=sys.argv[2]
if d.get("applied_count", 0) != 0:
    sys.exit("applied_count %r on a rejected batch, expected 0" % (d.get("applied_count"),))
if d.get("runtime_snapshot") is not None:
    sys.exit("a rejected batch returned a runtime_snapshot")
diagnostics=d.get("diagnostics", {}).get("diagnostics", [])
named=(offender in d.get("rejected_paths", [])
       or any(offender in json.dumps(entry) for entry in diagnostics))
if not named:
    sys.exit("neither rejected_paths %r nor the diagnostics name %s"
             % (d.get("rejected_paths"), offender))
' "${OUT}/atomic.res.json" "${IMMUTABLE_PATH}"

  local after_reject
  after_reject="$(read_parameter "${OUT}/commit.res.json" after_reject)"
  [[ "${after_reject}" == "${TRIM_VALUE}" ]] \
    || fail "atomic reject: the legal half of the batch leaked (value is ${after_reject})"
  echo "  -> batch refused whole; ${PID_PATH} still ${after_reject}"
}
step_atomic_reject

# ---------------------------------------------------------------------------
# S8) auto-reset policy — read, re-time, read back.
# ---------------------------------------------------------------------------
step "S8 auto-reset policy"
req_from "${OUT}/commit.res.json" "${OUT}/policy_get0.req.json"
rt get-auto-reset-policy "${OUT}/policy_get0.req.json" "${OUT}/policy_get0.res.json" 0
assert_status "get-auto-reset-policy (before)" "${OUT}/policy_get0.res.json" ok
REV0="$(jget "${OUT}/policy_get0.res.json" 'd["auto_reset_policy"]["policy_revision"]')"

# `policy_revision` is assigned by the runtime (previous + 1), never by the
# caller, so what is sent here cannot make the assertion below pass on its own.
req_from "${OUT}/commit.res.json" "${OUT}/policy_set.req.json" \
  "{'auto_reset_policy': {'enabled': True, 'default_timeout_ms': 60000,
                          'per_path_overrides': {}}}"
rt set-auto-reset-policy "${OUT}/policy_set.req.json" "${OUT}/policy_set.res.json" 0
assert_status "set-auto-reset-policy" "${OUT}/policy_set.res.json" ok

req_from "${OUT}/policy_set.res.json" "${OUT}/policy_get1.req.json"
rt get-auto-reset-policy "${OUT}/policy_get1.req.json" "${OUT}/policy_get1.res.json" 0
assert_status "get-auto-reset-policy (after)" "${OUT}/policy_get1.res.json" ok
pycheck "the new policy must be readable back and its revision must advance" '
import json,sys
policy=json.load(open(sys.argv[1]))["auto_reset_policy"]
before=int(sys.argv[2])
if policy["default_timeout_ms"] != 60000:
    sys.exit("default_timeout_ms %r, expected 60000" % (policy["default_timeout_ms"],))
if policy["enabled"] is not True:
    sys.exit("enabled %r, expected True" % (policy["enabled"],))
if policy["policy_revision"] <= before:
    sys.exit("policy_revision %r did not advance past %d" % (policy["policy_revision"], before))
' "${OUT}/policy_get1.res.json" "${REV0}"
echo "  -> policy re-timed to 60000 ms; revision advanced from ${REV0}"

# ---------------------------------------------------------------------------
# S9) determinism — the same resolve yields the same identity, byte for byte.
# ---------------------------------------------------------------------------
# An identity that wobbled between two opens of the same resolved output would
# make every assertion above meaningless: nothing downstream could tell a real
# change from noise.
step "S9 determinism of a re-opened session"
runtime_open "${OUT}/ropen2.req.json" "${OUT}/ropen2.res.json"
identity "${OUT}/ropen2.res.json" base2
if ! cmp -s "${OUT}/id_base.res.json" "${OUT}/id_base2.res.json"; then
  diff "${OUT}/id_base.res.json" "${OUT}/id_base2.res.json" >&2 || true
  fail "get-configuration-identity is not byte-stable across two opens of the same resolve"
fi
echo "  -> identity byte-identical across two independent opens"

step "DONE — runtime v2 session lifecycle green through the binary"
echo "dirty -> rollback -> commit -> atomic-reject -> policy -> determinism"
