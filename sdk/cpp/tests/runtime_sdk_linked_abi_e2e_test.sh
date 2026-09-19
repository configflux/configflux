#!/usr/bin/env bash
# The C++ SDK against the REAL runtime (ADR-0061 D3, configflux-x5gb.8).
#
# sdk/cpp/README.md's "Integration Example" — a RuntimeSession over
# MakeLinkedRuntimeCAbiApi(), opened on a real resolved snapshot, then read and
# written — had no live test: the only linked-ABI test covers the fail-closed
# open, and every happy-path SDK test runs against a test double. This closes
# that gap end to end:
#
#   compiler compile            -> CMP package + sibling .ccm
#   interpreter open            -> ModelHandle (carries ccm_ref)
#   interpreter init-state      -> SelectionState (encoder_type pinned)
#   interpreter select (x2)     -> motor_class=brushed_dc, power_rating=high
#   interpreter resolve         -> the resolved snapshot the SDK will open
#   runtime_session_e2e_driver  -> the linked C ABI, in-process, no subprocess:
#                                  open -> get -> set -> get -> immutable set
#                                  -> snapshot export
#
# Only the driver crosses the C ABI; the product binaries above are real
# subprocesses. The driver takes no decisions and asserts nothing — every
# assertion lives in assert_roundtrip below.
set -euo pipefail

# ---------------------------------------------------------------------------
# Runfiles resolution (the convention examples/e2e_03_motor_controller_test.sh
# established: rootpath env vars set by the sh_test `env` attribute).
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
require_var DRIVER_RLOCATION
require_var DEFS_RLOCATION
require_var COMPONENTS_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
INTERPRETER="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
DRIVER="${RUNFILES_ROOT}/${DRIVER_RLOCATION}"
DEFS="${RUNFILES_ROOT}/${DEFS_RLOCATION}"
COMPONENTS="${RUNFILES_ROOT}/${COMPONENTS_RLOCATION}"

for bin in "${COMPILER}" "${INTERPRETER}" "${DRIVER}"; do
  if [[ ! -x "${bin}" ]]; then
    echo "ERROR: binary not executable at ${bin}" >&2
    exit 2
  fi
done
for f in "${DEFS}" "${COMPONENTS}"; do
  if [[ ! -f "${f}" ]]; then
    echo "ERROR: data file not found at ${f}" >&2
    exit 2
  fi
done

OUT="${TEST_TMPDIR:-$(mktemp -d)}/out"
rm -rf "${OUT}"
mkdir -p "${OUT}"

SCOPE="all"
# lifecycle=runtime, type float — the one runtime-writable parameter in this model.
PARAM_PATH="component.motion_controller.param.pid_gain_trim"
NEW_VALUE="0.42"
# lifecycle=construction — a write to it must be refused by the runtime.
IMMUTABLE_PATH="component.motor_drive.param.control_mode"

fail() {
  echo "ASSERT FAILED: $*" >&2
  exit 1
}

jget() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
print(eval(sys.argv[2]))' "$1" "$2"
}

assert_status_ok() {
  local file="$1" label="$2" status
  status="$(jget "${file}" 'd["status"]')"
  if [[ "${status}" != "ok" ]]; then
    echo "--- ${label} response ---" >&2
    cat "${file}" >&2 || true
    fail "${label}: expected status ok, got '${status}'"
  fi
}

step() { printf '\n=== %s ===\n' "$1"; }

# ---------------------------------------------------------------------------
# 1) compile  (CMP package + sibling .ccm the runtime open requires)
# ---------------------------------------------------------------------------
step "compile"
"${COMPILER}" compile \
  --source "${DEFS}" \
  --source "${COMPONENTS}" \
  --out "${OUT}" \
  > "${OUT}/compile.result.json"
assert_status_ok "${OUT}/compile.result.json" "compile"
[[ -f "${OUT}/ccm/ccm.symbols.json" ]] || fail "compile: sibling .ccm not emitted"

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

# ---------------------------------------------------------------------------
# 3) init-selection-state  (encoder_type pinned as a context tag)
# ---------------------------------------------------------------------------
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

# ---------------------------------------------------------------------------
# 4) select x2  (motor_class=brushed_dc, then power_rating=high)
# ---------------------------------------------------------------------------
select_facet() {
  local prev="$1" out="$2" facet="$3" option="$4"
  python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":sys.argv[5],"option":sys.argv[6]}},
          open(sys.argv[3],"w"))' \
    "${OUT}/open.res.json" "${prev}" "${out}.req.json" "${SCOPE}" "${facet}" "${option}"
  "${INTERPRETER}" select \
    --request-file "${out}.req.json" \
    --response-file "${out}.res.json"
  assert_status_ok "${out}.res.json" "select ${facet}=${option}"
}

step "select motor_class=brushed_dc"
select_facet "${OUT}/init.res.json" "${OUT}/sel1" "motor_class" "brushed_dc"
step "select power_rating=high"
select_facet "${OUT}/sel1.res.json" "${OUT}/sel2" "power_rating" "high"

# ---------------------------------------------------------------------------
# 5) resolve  (the snapshot the SDK opens)
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

# ---------------------------------------------------------------------------
# 6) build the runtime-open request  (same projection as the e2e_03 handoff)
# ---------------------------------------------------------------------------
step "build runtime-open request"
python3 -c 'import json,sys
d=json.load(open(sys.argv[1])); o=json.load(open(sys.argv[2]))
req={"schema_version":5,"model_hash":d["model_hash"],
     "ccm_ref":o["model_handle"]["ccm_ref"],"resolve_hash":d["resolve_hash"],
     "scope":d["scope"],"resolved_output":d["resolved_output"],
     "resolved_component_dependencies":d.get("resolved_component_dependencies",{}),
     "resolved_artifacts":d.get("resolved_artifacts",{}),
     "context_tags":d.get("context_tags",{}),"choices":d.get("choices",{})}
json.dump(req,open(sys.argv[3],"w"))' \
  "${OUT}/resolve.res.json" "${OUT}/open.res.json" "${OUT}/ropen.req.json"

# ---------------------------------------------------------------------------
# 7) drive the linked C ABI
# ---------------------------------------------------------------------------
step "linked C ABI session (open -> get -> set -> get -> immutable set -> snapshot)"
set +e
"${DRIVER}" "${OUT}/ropen.req.json" "${PARAM_PATH}" "${NEW_VALUE}" "${IMMUTABLE_PATH}" \
  > "${OUT}/driver.out"
DRIVER_RC=$?
set -e
if [[ ${DRIVER_RC} -ne 0 ]]; then
  echo "--- driver output ---" >&2
  cat "${OUT}/driver.out" >&2 || true
  fail "driver exited ${DRIVER_RC} (expected 0)"
fi

# ---------------------------------------------------------------------------
# 8) assertions
# ---------------------------------------------------------------------------
# `set_immutable`'s printed status is the SDK's BOUNDARY status. A runtime-domain
# refusal is carried in the response envelope, not at the boundary, so the SDK
# maps it to RuntimeSdkStatus::kOk == 0 (see the enum in
# sdk/cpp/include/configflux/sdk/runtime_session.h and docs/runtime-c-abi.md
# section 6). Pinning the number here is what proves the refusal travelled as a
# domain rejection rather than as an ABI failure.
assert_roundtrip() {
  local driver_out="$1" resolve_res="$2"
  python3 - "${driver_out}" "${resolve_res}" "${PARAM_PATH}" "${NEW_VALUE}" "${SCOPE}" <<'PY'
import json
import sys

driver_out, resolve_res, param_path, new_value, scope = sys.argv[1:6]
expected_value = float(new_value)
steps = {}
with open(driver_out) as handle:
    for line in handle:
        line = line.strip()
        if line:
            record = json.loads(line)
            steps[record["step"]] = record

failures = []


def check(condition, message):
    if not condition:
        failures.append(message)


for name in ("open", "get_before", "set", "get_after", "set_immutable", "snapshot"):
    check(name in steps, f"driver emitted no '{name}' step")
if failures:
    print("\n".join(f"ASSERT FAILED: {f}" for f in failures), file=sys.stderr)
    sys.exit(1)

resolve = json.load(open(resolve_res))
baseline = resolve["resolved_output"][scope]["components"]["motion_controller"]["params"][
    "pid_gain_trim"
]["value"]

check(steps["open"]["ok"] is True, "open: driver reported a boundary failure")
check(
    steps["open"]["response"]["status"] == "ok",
    f"open: envelope status {steps['open']['response']['status']!r}, expected 'ok'",
)

before = steps["get_before"]["response"]
check(before["status"] == "ok", f"get_before: status {before['status']!r}")
check(
    before["parameter"]["value"] == baseline,
    f"get_before: {before['parameter']['value']!r} != resolve baseline {baseline!r}",
)

written = steps["set"]["response"]
check(written["status"] == "ok", f"set: status {written['status']!r}")
check(
    written["parameter"]["value"] == expected_value,
    f"set: echoed {written['parameter']['value']!r}, expected {expected_value!r}",
)

after = steps["get_after"]["response"]
check(after["status"] == "ok", f"get_after: status {after['status']!r}")
check(
    after["parameter"]["value"] == expected_value,
    f"get_after: read back {after['parameter']['value']!r}, expected {expected_value!r}",
)

rejected = steps["set_immutable"]
check(
    rejected["status"] == 0,
    f"set_immutable: boundary status {rejected['status']!r}, expected 0 (kOk)",
)
check(
    rejected["response"]["status"] == "error",
    f"set_immutable: envelope status {rejected['response']['status']!r}, expected 'error'",
)
code = rejected["response"]["diagnostics"]["diagnostics"][0]["code"]
check(
    code == "E_RUNTIME_LIFECYCLE_IMMUTABLE",
    f"set_immutable: first diagnostic {code!r}",
)

snapshot = steps["snapshot"]["response"]
check(
    "closed_facet_domains" in snapshot,
    "snapshot: missing the ABI v1.2 'closed_facet_domains' key",
)
# A runtime write lands in the dirty overlay; the resolved baseline is never
# rewritten, and the effective value is layered at read time
# (compiler/src/runtime_api/shared_ops.rs apply_dirty_write). Pinning both sides
# is what makes get_after above meaningful.
check(
    snapshot["dirty_overlay"].get(scope, {}).get(param_path) == expected_value,
    f"snapshot: dirty_overlay[{scope}][{param_path}] is "
    f"{snapshot['dirty_overlay'].get(scope, {}).get(param_path)!r}, expected {expected_value!r}",
)
check(
    snapshot["resolved_output"][scope]["components"]["motion_controller"]["params"][
        "pid_gain_trim"
    ]["value"]
    == baseline,
    "snapshot: resolved_output baseline was rewritten by a runtime write",
)

if failures:
    print("\n".join(f"ASSERT FAILED: {f}" for f in failures), file=sys.stderr)
    sys.exit(1)
print(f"  -> baseline {baseline} -> {expected_value} round-tripped; immutable write refused")
PY
}

step "assertions"
if ! assert_roundtrip "${OUT}/driver.out" "${OUT}/resolve.res.json"; then
  echo "--- driver output ---" >&2
  cat "${OUT}/driver.out" >&2 || true
  exit 1
fi

step "DONE — the C++ SDK round-tripped a real snapshot over the linked C ABI"
