#!/usr/bin/env bash
# Regression test for examples/deploy_guard.sh — the reference deploy-guard
# script (ADR-0035 C2).
#
# The deploy guard gates the DEPLOY ACTION on provenance: it refuses to deploy
# an OVERLAY-ACTIVE resolution (a snapshot whose context_tags.overlay is set —
# the ADR-0035 C1 stamp) to a NON-LOCAL-CLASS target unless an explicit
# --allow-overlay override is passed. A target is local-class iff its manifest
# entry declares `class: "local"`; an absent or any-other `class` is non-local-
# class by default (fail-safe: an unclassified target is the protected case).
# It reads ALREADY-PRODUCED JSON only (the snapshot + the manifest), invokes no
# ConfigFlux binary, and mirrors verify_bundle.sh's exit-code discipline.
#
# This test produces a REAL resolved snapshot by driving the product binaries
# over the examples/05-compose-fleet sources (compile -> open ->
# init-selection-state -> select x N -> resolve), then derives an overlay-active
# COPY by stamping context_tags.overlay="dev" into it (this simulates "an
# overlay was active"; building user-side overlay COMPOSITION is out of scope —
# the test exercises the GUARD over a stamped snapshot). It then asserts the
# guard's full decision table:
#
#   1. REFUSE  : overlay-active + non-local-class + no flag            (exit 1)
#   2. ALLOW   : overlay-active + non-local-class + --allow-overlay    (exit 0)
#   3. ALLOW   : overlay-active + local-class (class: "local")         (exit 0)
#   4. REFUSE  : overlay-active + UNCLASSIFIED target (no class)       (exit 1)
#   5. ALLOW   : NON-overlay snapshot + non-local-class               (exit 0)
#   6. ALLOW/REFUSE via the direct --target-class override            (exit 0/1)
#   7. exit 2  : usage errors (missing args, missing snapshot/manifest)
#
# JSON shaping in this harness uses python3 (no jq dependency in the asserts,
# mirroring examples/e2e_05_compose_fleet_test.sh). The script UNDER TEST uses
# jq, so jq must be on PATH; this test requires it (like verify_bundle_test.sh).
#
# Binaries, the example sources, and the guard script are located via the
# runfiles tree (rootpath env vars set by the sh_test `env` attribute),
# mirroring examples/verify_bundle_test.sh.
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
require_var GUARD_SCRIPT_RLOCATION
require_var DEFS_RLOCATION
require_var COMPONENTS_RLOCATION
require_var MANIFEST_RLOCATION

COMPILER="${RUNFILES_ROOT}/${COMPILER_RLOCATION}"
INTERPRETER="${RUNFILES_ROOT}/${INTERPRETER_RLOCATION}"
GUARD_SCRIPT="${RUNFILES_ROOT}/${GUARD_SCRIPT_RLOCATION}"
DEFS="${RUNFILES_ROOT}/${DEFS_RLOCATION}"
COMPONENTS="${RUNFILES_ROOT}/${COMPONENTS_RLOCATION}"
MANIFEST="${RUNFILES_ROOT}/${MANIFEST_RLOCATION}"

for bin in "${COMPILER}" "${INTERPRETER}"; do
  if [[ ! -x "${bin}" ]]; then
    echo "ERROR: binary not executable at ${bin}" >&2
    exit 2
  fi
done
for f in "${GUARD_SCRIPT}" "${DEFS}" "${COMPONENTS}" "${MANIFEST}"; do
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

# run_guard ARGS... — run the guard with set +e, capture rc + combined output.
# Sets the globals GUARD_RC and GUARD_OUT_FILE.
GUARD_OUT_FILE="${WORK}/guard.out"
run_guard() {
  set +e
  bash "${GUARD_SCRIPT}" "$@" > "${GUARD_OUT_FILE}" 2>&1
  GUARD_RC=$?
  set -e
}

# ---------------------------------------------------------------------------
# 0) Compile the example-05 model and resolve ONE real snapshot
# ---------------------------------------------------------------------------
step "compile the example-05 model + resolve one snapshot"
MODEL_OUT="${WORK}/model"
rm -rf "${MODEL_OUT}"
mkdir -p "${MODEL_OUT}"
# Stage sources with filename-only ids so model_hash is path-independent.
cp "${DEFS}" "${MODEL_OUT}/00_definitions.json"
cp "${COMPONENTS}" "${MODEL_OUT}/10_components.json"
(
  cd "${MODEL_OUT}"
  "${COMPILER}" compile \
    --source 00_definitions.json \
    --source 10_components.json \
    --out "${MODEL_OUT}/cmp" \
    > "${MODEL_OUT}/compile_result.json" 2> "${MODEL_OUT}/compile.err"
) || { cat "${MODEL_OUT}/compile.err" >&2 || true; fail "compile did not complete cleanly"; }
CMP="${MODEL_OUT}/cmp/cmp.manifest.json"
[[ -f "${CMP}" ]] || fail "compile: cmp.manifest.json missing"

# Resolve the robot-alpha / vision_service cell into a clean (no-overlay)
# snapshot, in python3 (mirrors the e2e_05 resolve_cell helper).
SCOPE="component:vision_service"
ENV_NAME="robot-alpha"
RWORK="${WORK}/resolve"
mkdir -p "${RWORK}"

python3 -c 'import json,sys
json.dump({"schema_version":3,"cmp_manifest_ref":sys.argv[1]},open(sys.argv[2],"w"))' \
  "${CMP}" "${RWORK}/open.req.json"
"${INTERPRETER}" open --request-file "${RWORK}/open.req.json" --response-file "${RWORK}/open.res.json"

python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); m=json.load(open(sys.argv[2]))
env=m["environments"][sys.argv[4]]
json.dump({"schema_version":3,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "context_tags":env.get("context_tags",{})},open(sys.argv[5],"w"))' \
  "${RWORK}/open.res.json" "${MANIFEST}" "${SCOPE}" "${ENV_NAME}" "${RWORK}/init.req.json"
"${INTERPRETER}" init-selection-state \
  --request-file "${RWORK}/init.req.json" --response-file "${RWORK}/init.res.json"

prev="${RWORK}/init.res.json"
facets="$(python3 -c 'import json,sys
m=json.load(open(sys.argv[1]))
print(" ".join(sorted(m["environments"][sys.argv[2]].get("choices",{}).keys())))' \
  "${MANIFEST}" "${ENV_NAME}")"
idx=0
for facet in ${facets}; do
  idx=$((idx + 1))
  python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2])); m=json.load(open(sys.argv[3]))
opt=m["environments"][sys.argv[5]]["choices"][sys.argv[6]]
json.dump({"schema_version":3,"model_handle":o["model_handle"],"scope":sys.argv[4],
          "selection_state":s["selection_state"],
          "selection_delta":{"facet":sys.argv[6],"option":opt}},open(sys.argv[7],"w"))' \
    "${RWORK}/open.res.json" "${prev}" "${MANIFEST}" "${SCOPE}" "${ENV_NAME}" "${facet}" \
    "${RWORK}/select_${idx}.req.json"
  "${INTERPRETER}" select \
    --request-file "${RWORK}/select_${idx}.req.json" \
    --response-file "${RWORK}/select_${idx}.res.json"
  prev="${RWORK}/select_${idx}.res.json"
done

python3 -c 'import json,sys
o=json.load(open(sys.argv[1])); s=json.load(open(sys.argv[2]))
json.dump({"schema_version":3,"model_handle":o["model_handle"],"scope":sys.argv[3],
          "selection_state":s["selection_state"]},open(sys.argv[4],"w"))' \
  "${RWORK}/open.res.json" "${prev}" "${SCOPE}" "${RWORK}/resolve.req.json"

CLEAN_SNAP="${WORK}/resolve_result.clean.json"
"${INTERPRETER}" resolve --request-file "${RWORK}/resolve.req.json" --response-file "${CLEAN_SNAP}"
STATUS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("status"))' "${CLEAN_SNAP}")"
[[ "${STATUS}" == "ok" ]] || { cat "${CLEAN_SNAP}" >&2; fail "resolve did not return status ok"; }
echo "  -> clean (no-overlay) snapshot resolved (status ok)"

# ---------------------------------------------------------------------------
# Derive an OVERLAY-ACTIVE copy by stamping context_tags.overlay="dev".
# This is the C1 stamp; the guard reads it. We never mutate a real resolved
# artifact in place — only a copy — and we do not pretend the product wrote it.
# ---------------------------------------------------------------------------
OVERLAY_SNAP="${WORK}/resolve_result.overlay.json"
python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
d.setdefault("context_tags",{})["overlay"]="dev"
json.dump(d,open(sys.argv[2],"w"),indent=2)' "${CLEAN_SNAP}" "${OVERLAY_SNAP}"
# Sanity: the clean snapshot must NOT carry an overlay tag; the copy must.
CLEAN_OV="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("context_tags",{}).get("overlay",""))' "${CLEAN_SNAP}")"
OVL_OV="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("context_tags",{}).get("overlay",""))' "${OVERLAY_SNAP}")"
[[ -z "${CLEAN_OV}" ]] || fail "clean snapshot unexpectedly carries context_tags.overlay='${CLEAN_OV}'"
[[ "${OVL_OV}" == "dev" ]] || fail "overlay copy missing context_tags.overlay=dev (got '${OVL_OV}')"
echo "  -> overlay-active copy stamped (context_tags.overlay=dev)"

# ---------------------------------------------------------------------------
# Build a small manifest with three classes: local, non-local (explicit
# non-"local" value), and unclassified (no class field at all).
# ---------------------------------------------------------------------------
GUARD_MANIFEST="${WORK}/guard.environments.json"
python3 -c 'import json,sys
m={"schema_version":3,"environments":{
  "dev-box":   {"scope":"component:vision_service","class":"local"},
  "shared":    {"scope":"component:vision_service","class":"shared"},
  "unmarked":  {"scope":"component:vision_service"}}}
json.dump(m,open(sys.argv[1],"w"),indent=2)' "${GUARD_MANIFEST}"

# ---------------------------------------------------------------------------
# 1) REFUSE: overlay-active + non-local-class + no flag  => exit 1
# ---------------------------------------------------------------------------
step "REFUSE overlay-active deploy to a non-local-class target (no flag)"
run_guard --snapshot "${OVERLAY_SNAP}" --manifest "${GUARD_MANIFEST}" --environment shared
if [[ "${GUARD_RC}" -ne 1 ]]; then
  cat "${GUARD_OUT_FILE}" >&2
  fail "overlay-active + non-local + no flag: expected exit 1, got ${GUARD_RC}"
fi
grep -qi "FAIL" "${GUARD_OUT_FILE}" || { cat "${GUARD_OUT_FILE}" >&2; fail "refusal did not print a FAIL line"; }
grep -qi -- "--allow-overlay" "${GUARD_OUT_FILE}" || { cat "${GUARD_OUT_FILE}" >&2; fail "refusal did not mention the --allow-overlay override"; }
echo "  -> refused (exit 1), FAIL line mentions the override"

# ---------------------------------------------------------------------------
# 2) ALLOW: overlay-active + non-local-class + --allow-overlay  => exit 0
# ---------------------------------------------------------------------------
step "ALLOW overlay-active deploy to a non-local-class target WITH --allow-overlay"
run_guard --snapshot "${OVERLAY_SNAP}" --manifest "${GUARD_MANIFEST}" --environment shared --allow-overlay
[[ "${GUARD_RC}" -eq 0 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "overlay + non-local + --allow-overlay: expected exit 0, got ${GUARD_RC}"; }
grep -qi "PASS" "${GUARD_OUT_FILE}" || { cat "${GUARD_OUT_FILE}" >&2; fail "allow did not print a PASS line"; }
echo "  -> allowed with override (exit 0)"

# ---------------------------------------------------------------------------
# 3) ALLOW: overlay-active + local-class (class: "local")  => exit 0
# ---------------------------------------------------------------------------
step "ALLOW overlay-active deploy to a LOCAL-CLASS target (always allowed)"
run_guard --snapshot "${OVERLAY_SNAP}" --manifest "${GUARD_MANIFEST}" --environment dev-box
[[ "${GUARD_RC}" -eq 0 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "overlay + local-class: expected exit 0, got ${GUARD_RC}"; }
grep -qi "PASS" "${GUARD_OUT_FILE}" || { cat "${GUARD_OUT_FILE}" >&2; fail "local-class allow did not print a PASS line"; }
# The stamp must be surfaced even on an allow.
grep -qi "overlay" "${GUARD_OUT_FILE}" || { cat "${GUARD_OUT_FILE}" >&2; fail "local-class allow did not surface the overlay stamp"; }
echo "  -> allowed against local-class (exit 0), stamp surfaced"

# ---------------------------------------------------------------------------
# 4) REFUSE: overlay-active + UNCLASSIFIED target (no class) + no flag  => exit 1
#    Fail-safe: an unclassified target is treated as the protected (non-local)
#    case, so a forgotten classification refuses rather than waving through.
# ---------------------------------------------------------------------------
step "REFUSE overlay-active deploy to an UNCLASSIFIED target (fail-safe)"
run_guard --snapshot "${OVERLAY_SNAP}" --manifest "${GUARD_MANIFEST}" --environment unmarked
[[ "${GUARD_RC}" -eq 1 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "overlay + unclassified + no flag: expected exit 1, got ${GUARD_RC}"; }
echo "  -> unclassified target treated as protected; refused (exit 1)"

# ---------------------------------------------------------------------------
# 5) ALLOW: NON-overlay snapshot + non-local-class  => exit 0
#    The guard is a no-op on the provenance check for a non-overlay resolution.
# ---------------------------------------------------------------------------
step "ALLOW a NON-overlay snapshot to a non-local-class target"
run_guard --snapshot "${CLEAN_SNAP}" --manifest "${GUARD_MANIFEST}" --environment shared
[[ "${GUARD_RC}" -eq 0 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "non-overlay + non-local: expected exit 0, got ${GUARD_RC}"; }
grep -qi "PASS" "${GUARD_OUT_FILE}" || { cat "${GUARD_OUT_FILE}" >&2; fail "non-overlay allow did not print a PASS line"; }
echo "  -> non-overlay snapshot allowed unconditionally (exit 0)"

# Also: non-overlay against an unclassified target is allowed (provenance no-op).
run_guard --snapshot "${CLEAN_SNAP}" --manifest "${GUARD_MANIFEST}" --environment unmarked
[[ "${GUARD_RC}" -eq 0 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "non-overlay + unclassified: expected exit 0, got ${GUARD_RC}"; }
echo "  -> non-overlay snapshot allowed against an unclassified target too (exit 0)"

# ---------------------------------------------------------------------------
# 6) Direct --target-class override (no manifest lookup)
# ---------------------------------------------------------------------------
step "direct --target-class override path"
# overlay + explicit local => allow
run_guard --snapshot "${OVERLAY_SNAP}" --target-class local
[[ "${GUARD_RC}" -eq 0 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "overlay + --target-class local: expected exit 0, got ${GUARD_RC}"; }
# overlay + explicit non-local => refuse
run_guard --snapshot "${OVERLAY_SNAP}" --target-class shared
[[ "${GUARD_RC}" -eq 1 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "overlay + --target-class shared: expected exit 1, got ${GUARD_RC}"; }
# overlay + explicit non-local + --allow-overlay => allow
run_guard --snapshot "${OVERLAY_SNAP}" --target-class shared --allow-overlay
[[ "${GUARD_RC}" -eq 0 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "overlay + --target-class shared + flag: expected exit 0, got ${GUARD_RC}"; }
echo "  -> direct --target-class override decides correctly (allow/refuse/allow)"

# ---------------------------------------------------------------------------
# 7) Usage / argument / tool errors  => exit 2
# ---------------------------------------------------------------------------
step "usage errors exit 2"
# No args at all.
run_guard
[[ "${GUARD_RC}" -eq 2 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "no args: expected exit 2, got ${GUARD_RC}"; }
# Missing snapshot file.
run_guard --snapshot "${WORK}/does-not-exist.json" --target-class local
[[ "${GUARD_RC}" -eq 2 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "missing snapshot: expected exit 2, got ${GUARD_RC}"; }
# --environment given without --manifest.
run_guard --snapshot "${OVERLAY_SNAP}" --environment shared
[[ "${GUARD_RC}" -eq 2 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "--environment without --manifest: expected exit 2, got ${GUARD_RC}"; }
# Neither --environment/--manifest nor --target-class supplied.
run_guard --snapshot "${OVERLAY_SNAP}"
[[ "${GUARD_RC}" -eq 2 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "no class source: expected exit 2, got ${GUARD_RC}"; }
# Environment not present in the manifest.
run_guard --snapshot "${OVERLAY_SNAP}" --manifest "${GUARD_MANIFEST}" --environment nope
[[ "${GUARD_RC}" -eq 2 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "unknown environment: expected exit 2, got ${GUARD_RC}"; }
# Unknown flag.
run_guard --snapshot "${OVERLAY_SNAP}" --target-class local --bogus
[[ "${GUARD_RC}" -eq 2 ]] || { cat "${GUARD_OUT_FILE}" >&2; fail "unknown flag: expected exit 2, got ${GUARD_RC}"; }
echo "  -> usage/argument errors all exit 2"

step "DONE — deploy_guard.sh enforces the ADR-0035 C2 decision table"
