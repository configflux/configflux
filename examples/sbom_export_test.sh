#!/usr/bin/env bash
# Process-level test for SOFTWARE BOM EXPORT through the interpreter BINARY
# (configflux-x5gb.7, ADR-0061 D3).
#
# `export-software-bom` is the interpreter's compliance surface: what software,
# at which versions, with which parameter values, is in the thing I am about to
# ship. Until now it was exercised only IN PROCESS, and only as a misuse case
# (interpreter/src/tests.rs int_006) — nothing pinned that the payload
# docs/software-bom-schema.md FREEZES is what the shipped binary emits.
#
# The chain is copied in shape from examples/e2e_03_motor_controller_test.sh
# steps 2-6; the export steps follow docs/interpreter-cli-contract.md §3.1
# steps 7 and 8.
#
# One distinction is worth naming up front, because it is the whole reason a
# bill of materials is not a directory listing: `resolved_artifacts` is the
# CATALOGUE of what the model knows about, while the BOM's `artifacts` are what
# this configuration actually BOUND through artifact-typed parameters — 2 of 4
# here. Both are derived below from the resolve response, never hard-coded.
#
# python3 does every JSON shaping and assertion; no jq, matching
# examples/e2e_03_motor_controller_test.sh.
set -euo pipefail

# --- Runfiles resolution ---------------------------------------------------
if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi
RUNFILES_ROOT="${TEST_SRCDIR}/_main"

for name in COMPILER INTERPRETER DEFS COMPONENTS; do
  rlocation="${name}_RLOCATION"
  if [[ -z "${!rlocation:-}" ]]; then
    echo "ERROR: ${rlocation} env var is not set" >&2
    exit 2
  fi
  declare "${name}=${RUNFILES_ROOT}/${!rlocation}"
done

for bin in "${COMPILER}" "${INTERPRETER}"; do
  [[ -x "${bin}" ]] || { echo "ERROR: binary not executable at ${bin}" >&2; exit 2; }
done
[[ -f "${DEFS}" && -f "${COMPONENTS}" ]] \
  || { echo "ERROR: example sources missing from runfiles" >&2; exit 2; }

OUT="${TEST_TMPDIR:-$(mktemp -d)}/sbom"
rm -rf "${OUT}"
mkdir -p "${OUT}"

SCOPE="all"
PID_PATH="component.motion_controller.param.pid_gain_trim"

fail() { echo "ASSERT FAILED: $*" >&2; exit 1; }
step() { printf '\n=== %s ===\n' "$1"; }

# jget FILE PY_EXPR — evaluate a python expression against FILE's JSON (as `d`).
jget() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1],encoding="utf-8"))
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

# run_export CMD REQ RES — run one export command, echo its exit code. A
# non-zero exit is EXPECTED at S5, so `set -e` is lifted around the call only.
run_export() {
  local rc=0
  set +e
  "${INTERPRETER}" "$1" --request-file "$2" --response-file "$3"
  rc=$?
  set -e
  echo "${rc}"
}

# export_request RESOLVE PROFILE OUT — wrap the WHOLE resolve response as
# `resolve_result`, the request shape both export commands take.
export_request() {
  python3 -c 'import json,sys
r=json.load(open(sys.argv[1],encoding="utf-8"))
json.dump({"schema_version":5,"resolve_result":r,"profile":sys.argv[3]},
          open(sys.argv[2],"w",encoding="utf-8"))' "$1" "$3" "$2"
}

# ---------------------------------------------------------------------------
# S1 — the chain that produces something to export
# ---------------------------------------------------------------------------
# encoder_type is pinned as a context tag and two facets applied, which is what
# `all` needs. Sources compile in place: model_hash covers CONTENT (ADR-0056).
step "S1 compile -> open -> init -> select x2 -> resolve"
"${COMPILER}" compile --source "${DEFS}" --source "${COMPONENTS}" --out "${OUT}" \
  > "${OUT}/compile.res.json"
assert_status_ok "${OUT}/compile.res.json" "compile"

python3 -c 'import json,sys
json.dump({"schema_version":5,"cmp_manifest_ref":sys.argv[1]},
          open(sys.argv[2],"w",encoding="utf-8"))' \
  "${OUT}/cmp.manifest.json" "${OUT}/open.req.json"
"${INTERPRETER}" open --request-file "${OUT}/open.req.json" \
  --response-file "${OUT}/open.res.json"
assert_status_ok "${OUT}/open.res.json" "open"

python3 -c 'import json,sys
o=json.load(open(sys.argv[1],encoding="utf-8"))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "context_tags":{"encoder_type":"absolute"}},
          open(sys.argv[2],"w",encoding="utf-8"))' \
  "${OUT}/open.res.json" "${OUT}/init.req.json" "${SCOPE}"
"${INTERPRETER}" init-selection-state --request-file "${OUT}/init.req.json" \
  --response-file "${OUT}/init.res.json"
assert_status_ok "${OUT}/init.res.json" "init-selection-state"

apply_facet() {
  local prev="$1" facet="$2" option="$3" tag="$4"
  python3 -c 'import json,sys
o=json.load(open(sys.argv[1],encoding="utf-8")); s=json.load(open(sys.argv[2],encoding="utf-8"))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":sys.argv[5],"option":sys.argv[6]}},
          open(sys.argv[3],"w",encoding="utf-8"))' \
    "${OUT}/open.res.json" "${prev}" "${OUT}/${tag}.req.json" "${SCOPE}" "${facet}" "${option}"
  "${INTERPRETER}" select --request-file "${OUT}/${tag}.req.json" \
    --response-file "${OUT}/${tag}.res.json"
  assert_status_ok "${OUT}/${tag}.res.json" "select ${facet}=${option}"
}
apply_facet "${OUT}/init.res.json" motor_class brushed_dc sel1
apply_facet "${OUT}/sel1.res.json" power_rating high sel2

python3 -c 'import json,sys
o=json.load(open(sys.argv[1],encoding="utf-8")); s=json.load(open(sys.argv[2],encoding="utf-8"))
json.dump({"schema_version":5,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"]},
          open(sys.argv[3],"w",encoding="utf-8"))' \
  "${OUT}/open.res.json" "${OUT}/sel2.res.json" "${OUT}/resolve.req.json" "${SCOPE}"
"${INTERPRETER}" resolve --request-file "${OUT}/resolve.req.json" \
  --response-file "${OUT}/resolve.res.json"
assert_status_ok "${OUT}/resolve.res.json" "resolve"
echo "  -> resolve_hash=$(jget "${OUT}/resolve.res.json" 'd["resolve_hash"]')"

# ---------------------------------------------------------------------------
# assert_bom_shape RESOLVE_RESPONSE EXPORT_RESPONSE — the canonical-shape
# assertion for the whole test (traceability symbol for REQ-INT-022). Everything
# it compares is derived from the resolve response it is handed, so the only
# literals are the ones docs/software-bom-schema.md FREEZES.
# ---------------------------------------------------------------------------
assert_bom_shape() {
  python3 - "$1" "$2" "${PID_PATH}" <<'PY' || fail "BOM canonical shape assertions failed"
import json
import sys

resolve = json.load(open(sys.argv[1], encoding="utf-8"))
result = json.load(open(sys.argv[2], encoding="utf-8"))
pid_path = sys.argv[3]

bom = result.get("software_bom")
if bom is None:
    raise SystemExit("export result carries no `software_bom` object: %r" % (sorted(result),))

# §2 — every top-level key of the frozen payload. selection_state_hash is
# optional in the schema but the resolve here has one, so it must be carried.
required = {
    "schema_version", "bom_version", "bom_hash", "hash_algo",
    "canonicalization_version", "model_hash", "resolve_hash", "scope_root",
    "generated_at", "generator", "context_tags", "choices", "components",
    "parameters", "artifacts", "stats",
}
missing = required - set(bom)
if missing:
    raise SystemExit("BOM is missing frozen top-level keys: %s" % (sorted(missing),))
if resolve.get("selection_state_hash") and "selection_state_hash" not in bom:
    raise SystemExit("resolve carried a selection_state_hash; the BOM dropped it")

for key, want in (("bom_version", 1), ("hash_algo", "sha256"),
                  ("canonicalization_version", 1),
                  ("schema_version", resolve["schema_version"])):
    if bom[key] != want:
        raise SystemExit("BOM %s is %r, expected %r" % (key, bom[key], want))

# The constants that make a BOM REPRODUCIBLE: a wall-clock stamp or a
# build-varying generator version would rotate bom_hash on every export. Frozen.
if bom["generated_at"] != "1970-01-01T00:00:00Z":
    raise SystemExit("generated_at is %r, not the frozen epoch constant" % (bom["generated_at"],))
if bom["generator"] != {"name": "configflux-sbom", "version": "0.1.0"}:
    raise SystemExit("generator is %r, not the frozen v1 constant" % (bom["generator"],))

# Identity continuity (contract §3.1): the resolve's hashes are preserved.
for key in ("model_hash", "resolve_hash"):
    if bom[key] != resolve[key]:
        raise SystemExit("BOM %s %r != resolve %r" % (key, bom[key], resolve[key]))
    if result[key] != resolve[key]:
        raise SystemExit("envelope %s %r != resolve %r" % (key, result[key], resolve[key]))
if not bom["bom_hash"] or result.get("bom_hash") != bom["bom_hash"]:
    raise SystemExit("bom_hash missing or disagrees between envelope and payload")
if bom["scope_root"] != resolve["scope"]:
    raise SystemExit("scope_root %r != resolve scope %r" % (bom["scope_root"], resolve["scope"]))
for key in ("context_tags", "choices"):
    if bom[key] != resolve[key]:
        raise SystemExit("BOM %s %r != resolve %r" % (key, bom[key], resolve[key]))

# §5 — ordering. bom_hash covers these arrays AS EMITTED: order IS contract.
for label, key, sort_key in (("components", "components", "component_id"),
                             ("parameters", "parameters", "path"),
                             ("artifacts", "artifacts", "artifact_id")):
    got = [entry[sort_key] for entry in bom[key]]
    if got != sorted(got):
        raise SystemExit("%s are not sorted by %s: %r" % (label, sort_key, got))
    if len(set(got)) != len(got):
        raise SystemExit("%s carry a duplicate %s: %r" % (label, sort_key, got))
for artifact in bom["artifacts"]:
    paths = artifact["bound_paths"]
    if paths != sorted(set(paths)):
        raise SystemExit("artifact %r bound_paths not sorted-unique: %r"
                         % (artifact["artifact_id"], paths))

# §6 — stats must describe the lists actually emitted.
want_stats = {"component_count": len(bom["components"]),
              "parameter_count": len(bom["parameters"]),
              "artifact_count": len(bom["artifacts"])}
if bom["stats"] != want_stats:
    raise SystemExit("stats %r do not match list lengths %r" % (bom["stats"], want_stats))

# The BILL vs the CATALOGUE (header note), both derived from the resolve.
components = {}
for scope_config in resolve["resolved_output"].values():
    components.update(scope_config["components"])
bound = {}
for component_id, component in components.items():
    for param_key, param in component["params"].items():
        if param["type"] == "artifact":
            path = "component.%s.param.%s" % (component_id, param_key)
            bound.setdefault(param["value"], set()).add(path)
if not bound:
    raise SystemExit("the resolved output binds no artifacts; this example must bind some")

catalogue = set(resolve["resolved_artifacts"])
if {entry["artifact_id"] for entry in bom["artifacts"]} != set(bound):
    raise SystemExit("BOM artifact ids %r != the ids bound by artifact-typed parameters %r"
                     % (sorted(e["artifact_id"] for e in bom["artifacts"]), sorted(bound)))
if not set(bound) < catalogue:
    raise SystemExit("expected the bound set %r to be a PROPER subset of the resolved artifact "
                     "catalogue %r — a BOM that repeats the catalogue is not a bill of materials"
                     % (sorted(bound), sorted(catalogue)))
for artifact in bom["artifacts"]:
    want_paths = sorted(bound[artifact["artifact_id"]])
    if artifact["bound_paths"] != want_paths:
        raise SystemExit("artifact %r bound_paths %r != the binding parameter paths %r"
                         % (artifact["artifact_id"], artifact["bound_paths"], want_paths))
    if not artifact["name"].strip():
        raise SystemExit("artifact %r has an empty name" % (artifact["artifact_id"],))

# Every resolved parameter is in the bill at its resolved value.
by_path = {entry["path"]: entry for entry in bom["parameters"]}
want_paths = {"component.%s.param.%s" % (cid, key)
              for cid, component in components.items() for key in component["params"]}
if set(by_path) != want_paths:
    raise SystemExit("BOM parameter paths %r != resolved parameter paths %r"
                     % (sorted(by_path), sorted(want_paths)))
pid = by_path.get(pid_path)
if pid is None:
    raise SystemExit("BOM has no parameter %r" % (pid_path,))
resolved_pid = components["motion_controller"]["params"]["pid_gain_trim"]
if pid["value"] != resolved_pid["value"]:
    raise SystemExit("BOM %s value %r != resolved value %r"
                     % (pid_path, pid["value"], resolved_pid["value"]))
if (pid["lifecycle"], pid["binding_phase"]) != ("runtime", "runtime"):
    raise SystemExit("%s lifecycle/binding_phase %r is not the runtime pair"
                     % (pid_path, (pid["lifecycle"], pid["binding_phase"])))

print("  -> %d components, %d parameters, %d artifacts (of %d catalogued); bom_hash=%s"
      % (want_stats["component_count"], want_stats["parameter_count"],
         want_stats["artifact_count"], len(catalogue), bom["bom_hash"]))
PY
}

# --- S2 export-software-bom, profile full_audit ----------------------------
step "S2 export-software-bom (full_audit)"
export_request "${OUT}/resolve.res.json" full_audit "${OUT}/bom_full.req.json"
RC="$(run_export export-software-bom "${OUT}/bom_full.req.json" "${OUT}/bom_full.res.json")"
[[ "${RC}" == "0" ]] || { cat "${OUT}/bom_full.res.json" >&2 || true; fail "S2: expected exit 0, got ${RC}"; }
assert_status_ok "${OUT}/bom_full.res.json" "export-software-bom (full_audit)"
assert_bom_shape "${OUT}/resolve.res.json" "${OUT}/bom_full.res.json"

# ---------------------------------------------------------------------------
# S3 — determinism. Not reproducible, not evidence: two auditors exporting the
# same resolve must get the same bytes, or the hash proves nothing.
# ---------------------------------------------------------------------------
step "S3 the same export twice is byte-identical"
RC="$(run_export export-software-bom "${OUT}/bom_full.req.json" "${OUT}/bom_full2.res.json")"
[[ "${RC}" == "0" ]] || fail "S3: rerun expected exit 0, got ${RC}"
cmp -s "${OUT}/bom_full.res.json" "${OUT}/bom_full2.res.json" \
  || { diff "${OUT}/bom_full.res.json" "${OUT}/bom_full2.res.json" >&2 || true
       fail "S3: two identical exports differ"; }
echo "  -> two runs byte-identical"

# ---------------------------------------------------------------------------
# S4 — profile value_redacted (§7.2)
# ---------------------------------------------------------------------------
step "S4 export-software-bom (value_redacted)"
export_request "${OUT}/resolve.res.json" value_redacted "${OUT}/bom_red.req.json"
RC="$(run_export export-software-bom "${OUT}/bom_red.req.json" "${OUT}/bom_red.res.json")"
[[ "${RC}" == "0" ]] || { cat "${OUT}/bom_red.res.json" >&2 || true; fail "S4: expected exit 0, got ${RC}"; }
assert_status_ok "${OUT}/bom_red.res.json" "export-software-bom (value_redacted)"
python3 - "${OUT}/bom_full.res.json" "${OUT}/bom_red.res.json" <<'PY' || fail "S4: redaction assertions failed"
import json
import sys

full = json.load(open(sys.argv[1], encoding="utf-8"))["software_bom"]
red = json.load(open(sys.argv[2], encoding="utf-8"))["software_bom"]

# The rule: withhold the values NOT fixed at build time. An early-bound value is
# compiled into the binary anyway; an artifact value is an id, not a setting.
if len(full["parameters"]) != len(red["parameters"]):
    raise SystemExit("redaction changed the parameter count: %d vs %d"
                     % (len(full["parameters"]), len(red["parameters"])))
redacted = withheld = kept = 0
for before, after in zip(full["parameters"], red["parameters"]):
    if before["path"] != after["path"]:
        raise SystemExit("parameter order moved under redaction: %r vs %r"
                         % (before["path"], after["path"]))
    should_redact = before["binding_phase"] != "early" and before["type"] != "artifact"
    if should_redact:
        withheld += 1
        if after["value"] != "<redacted>":
            raise SystemExit("%s (%s/%s) was not redacted: %r"
                             % (after["path"], before["binding_phase"], before["type"],
                                after["value"]))
        redacted += 1
    else:
        kept += 1
        if after["value"] != before["value"]:
            raise SystemExit("%s (%s/%s) must keep its value, got %r (was %r)"
                             % (after["path"], before["binding_phase"], before["type"],
                                after["value"], before["value"]))
    for field in ("component_id", "param_key", "type", "safety", "lifecycle",
                  "binding_phase", "access"):
        if after[field] != before[field]:
            raise SystemExit("%s %s moved under redaction: %r vs %r"
                             % (after["path"], field, before[field], after[field]))
if not withheld or not kept:
    raise SystemExit("the profile must both withhold and keep values here; got %d/%d"
                     % (withheld, kept))

# Structure and identity are preserved; only bom_hash may move.
for key in ("model_hash", "resolve_hash", "scope_root", "stats", "components",
            "artifacts", "context_tags", "choices", "generated_at", "generator"):
    if red[key] != full[key]:
        raise SystemExit("value_redacted moved %s: %r vs %r" % (key, full[key], red[key]))
if red["bom_hash"] == full["bom_hash"]:
    raise SystemExit("bom_hash did not move even though parameter values did")
print("  -> %d value(s) withheld, %d preserved; identity and artifacts unmoved" % (redacted, kept))
PY

# --- S5 an unknown profile is refused, with the frozen code (§8) -----------
step "S5 export-software-bom (unknown profile) is refused"
export_request "${OUT}/resolve.res.json" nonexistent "${OUT}/bom_bad.req.json"
RC="$(run_export export-software-bom "${OUT}/bom_bad.req.json" "${OUT}/bom_bad.res.json")"
[[ "${RC}" == "2" ]] || fail "S5: expected exit 2 (command error), got ${RC}"
BAD_STATUS="$(jget "${OUT}/bom_bad.res.json" 'd["status"]')"
[[ "${BAD_STATUS}" == "error" ]] || fail "S5: expected status error, got ${BAD_STATUS}"
# §8 freezes this code, so it is spelled ONCE here and compared by variable:
# a diagnostic code is an API, and a caller keying on it must not have to guess.
WANT_CODE="E_SBOM_PROFILE_INVALID"
BAD_CODE="$(jget "${OUT}/bom_bad.res.json" 'd["diagnostics"]["diagnostics"][0]["code"]')"
if [[ "${BAD_CODE}" != "${WANT_CODE}" ]]; then
  cat "${OUT}/bom_bad.res.json" >&2 || true
  fail "S5: expected ${WANT_CODE}, got ${BAD_CODE}"
fi
HAS_BOM="$(jget "${OUT}/bom_bad.res.json" '"software_bom" in d')"
[[ "${HAS_BOM}" == "False" ]] || fail "S5: a refused export still emitted a software_bom"
echo "  -> refused: exit 2, ${BAD_CODE}, no partial BOM"

# ---------------------------------------------------------------------------
# S6 — export-resolved, the early-binding sibling
# ---------------------------------------------------------------------------
step "S6 export-resolved (cpp_early_binding_v1)"
export_request "${OUT}/resolve.res.json" cpp_early_binding_v1 "${OUT}/exp.req.json"
RC="$(run_export export-resolved "${OUT}/exp.req.json" "${OUT}/exp.res.json")"
[[ "${RC}" == "0" ]] || { cat "${OUT}/exp.res.json" >&2 || true; fail "S6: expected exit 0, got ${RC}"; }
assert_status_ok "${OUT}/exp.res.json" "export-resolved"
FILES="$(jget "${OUT}/exp.res.json" 'sorted(f["path"] for f in d["generated_artifacts"]["files"])')"
WANT="['generated/config.hpp', 'generated/config_artifact_manifest.json', 'generated/config_build_flags.cmake']"
[[ "${FILES}" == "${WANT}" ]] || fail "S6: generated files ${FILES} != ${WANT}"
RC="$(run_export export-resolved "${OUT}/exp.req.json" "${OUT}/exp2.res.json")"
[[ "${RC}" == "0" ]] || fail "S6: rerun expected exit 0, got ${RC}"
cmp -s "${OUT}/exp.res.json" "${OUT}/exp2.res.json" \
  || { diff "${OUT}/exp.res.json" "${OUT}/exp2.res.json" >&2 || true
       fail "S6: two identical export-resolved runs differ"; }
echo "  -> 3 generated files, two runs byte-identical"

step "DONE — SBOM export through the interpreter binary green"
echo "resolve -> full_audit(shape/order/bill) -> determinism -> value_redacted -> bad profile(2) -> export-resolved"
