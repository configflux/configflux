#!/usr/bin/env bash
# 00-service-multi-env — the multi-environment web-service hero example.
#
# ONE model, THREE named environments (dev, staging, prod) expressed as
# selections over that model, each resolved to a deterministic snapshot with the
# one-shot `cfx` CLI. It also shows the differentiator: the model declares a
# named constraint — prod_forbids_debug, in 00_definitions.json — and
# `cfx options` / `cfx explain` surface that policy before a bad config ever
# ships. Policy is its own construct: the `environment == 'prod'` conditions on
# the webapp overrides are inclusion selectors and constrain nobody's choices.
#
# Steps:
#   1. Compile the shared model (the CUE sources were exported out-of-band to the
#      committed *.json; this script never invokes cue).
#   2. Resolve every environment named in environments.json in ONE command —
#      `cfx resolve --manifest environments.json --all` — and record each
#      snapshot.
#   3. Re-resolve the whole manifest a second time and assert the snapshots are
#      byte-identical — determinism is the contract.
#   4. Constraint showcase: the prod_forbids_debug constraint is enforced on all
#      three surfaces — after selecting environment=prod, `cfx options` no longer
#      offers log_level=debug; `cfx explain` names prod_forbids_debug as the
#      minimal conflict for the deliberately-invalid prod+debug selection; and
#      `cfx resolve` refuses that selection outright (exit 3) instead of
#      emitting a snapshot.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
OUT_DIR="${CONFIGFLUX_EXAMPLE_OUT_DIR:-${EXAMPLE_DIR}/out}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
find_binary() {
  local env_var="$1" bazel_rel="$2" pretty="$3"
  local override="${!env_var:-}"
  if [[ -n "${override}" ]]; then
    echo "${override}"
    return
  fi
  local bazel_bin="${REPO_ROOT}/${bazel_rel}"
  if [[ -x "${bazel_bin}" ]]; then
    echo "${bazel_bin}"
    return
  fi
  echo >&2 "Error: ${pretty} binary not found."
  echo >&2 "  Either build it:  bazel build //compiler //cfx"
  echo >&2 "  Or set:           export ${env_var}=/path/to/${pretty}"
  exit 1
}

banner() { printf "\n=== %s ===\n" "$1"; }
ok()     { printf "  -> %s\n" "$1"; }

COMPILER="$(find_binary CONFIGFLUX_COMPILER bazel-bin/compiler/compiler compiler)"
CFX="$(find_binary CONFIGFLUX_CFX bazel-bin/cfx/cfx cfx)"

DEFS="${EXAMPLE_DIR}/00_definitions.json"
COMPONENTS="${EXAMPLE_DIR}/10_components.json"
MANIFEST="${EXAMPLE_DIR}/environments.json"

# The resolved snapshot `cfx resolve --manifest --all` writes for ONE named
# environment: exactly one resolve_result.<root>.<selection>.json under
# <out>/<environment>/<scope-root>/, beside the generated/ C++ early-binding
# files. <root> is the scope root and <selection> the environment's choices
# joined in sorted-facet order, so each environment lands on its own name in
# its own directory and two targets can never collide.
snapshot_path() {
  local env_dir="$1"
  local -a found
  shopt -s nullglob
  found=("${env_dir}"/*/resolve_result.*.json)
  shopt -u nullglob
  if [[ ${#found[@]} -ne 1 ]]; then
    echo "expected exactly one resolve_result.*.json under ${env_dir}, found ${#found[@]}" >&2
    exit 1
  fi
  printf '%s' "${found[0]}"
}

# Print a one-line summary of a resolved snapshot (the ResolveResult envelope
# cfx wrote).
snapshot_summary() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
def find(x,k):
    if isinstance(x,dict):
        if k in x and isinstance(x[k],dict) and "value" in x[k]: return x[k]["value"]
        for v in x.values():
            r=find(v,k)
            if r is not None: return r
    elif isinstance(x,list):
        for i in x:
            r=find(i,k)
            if r is not None: return r
    return None
o=d.get("resolved_output",{})
print("request_timeout_ms=%s deploy_tier=%s resolve_hash=%s"
      % (find(o,"request_timeout_ms"), find(o,"deploy_tier"), d["resolve_hash"]))' "$1"
}

# ---------------------------------------------------------------------------
# Reset output
# ---------------------------------------------------------------------------
rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

# ---------------------------------------------------------------------------
# Step 1: Compile the shared model
# ---------------------------------------------------------------------------
# Stage the committed sources into the out dir and compile with FILENAME-ONLY
# source ids, so model_hash (and the cascading resolve_hash) is independent of
# where this checkout lives.
banner "Step 1: Compile shared model"
SRC_DIR="${OUT_DIR}/src"
mkdir -p "${SRC_DIR}"
cp "${DEFS}" "${SRC_DIR}/00_definitions.json"
cp "${COMPONENTS}" "${SRC_DIR}/10_components.json"
(
  cd "${SRC_DIR}"
  "${COMPILER}" compile \
    --source 00_definitions.json \
    --source 10_components.json \
    --out "${OUT_DIR}/cmp" \
    > "${OUT_DIR}/compile_result.json"
)
MODEL="${OUT_DIR}/cmp/cmp.manifest.json"
[[ -f "${MODEL}" ]] || { echo "compile: cmp.manifest.json missing" >&2; exit 1; }
ok "compiled -> ${MODEL} (+ sibling ccm/)"

# ---------------------------------------------------------------------------
# Step 2: Resolve every named environment
# ---------------------------------------------------------------------------
banner "Step 2: Resolve every environment in environments.json with cfx resolve"
RESOLVED_DIR="${OUT_DIR}/resolved"

# ONE command for the whole manifest. `cfx resolve` reads environments.json
# directly — the same small JSON file you version-control — and `--all` resolves
# every environment it names into its own directory, writing each resolved
# snapshot (resolve_result.<root>.<selection>.json, the JSON this service would
# read at startup) alongside the generated/ early-binding files. Nothing here
# desugars the manifest, threads a request envelope, or rebuilds a snapshot from
# stdout: the manifest is a product input.
"${CFX}" resolve \
  --model "${MODEL}" \
  --manifest "${MANIFEST}" \
  --all \
  --out "${RESOLVED_DIR}" \
  > "${OUT_DIR}/resolve.log"

# The environments are the directories cfx just wrote, in sorted order — the
# same order cfx processed them in.
mapfile -t ENVIRONMENTS < <(cd "${RESOLVED_DIR}" && find . -mindepth 1 -maxdepth 1 -type d -printf '%P\n' | sort)
[[ ${#ENVIRONMENTS[@]} -eq 3 ]] || { echo "expected 3 resolved environments, got ${#ENVIRONMENTS[@]}" >&2; exit 1; }

for env_name in "${ENVIRONMENTS[@]}"; do
  snap="$(snapshot_path "${RESOLVED_DIR}/${env_name}")"
  ok "${env_name}: ${snap}"
  printf "       %s\n" "$(snapshot_summary "${snap}")"
done

# ---------------------------------------------------------------------------
# Step 3: Determinism — re-resolve and assert byte-identical snapshots
# ---------------------------------------------------------------------------
banner "Step 3: Determinism check (re-resolve; snapshots must be byte-identical)"
RECHECK_DIR="${OUT_DIR}/.recheck"
rm -rf "${RECHECK_DIR}"
"${CFX}" resolve \
  --model "${MODEL}" \
  --manifest "${MANIFEST}" \
  --all \
  --out "${RECHECK_DIR}" \
  > "${OUT_DIR}/recheck.log"
for env_name in "${ENVIRONMENTS[@]}"; do
  first="$(snapshot_path "${RESOLVED_DIR}/${env_name}")"
  recheck="$(snapshot_path "${RECHECK_DIR}/${env_name}")"
  # The NAME is part of the contract too: it is derived from the scope and the
  # selection, so a differing name would mean a differing target.
  if [[ "$(basename "${recheck}")" != "$(basename "${first}")" ]]; then
    echo "determinism: ${env_name} snapshot file name differed between two runs" >&2
    exit 1
  fi
  if ! cmp -s "${first}" "${recheck}"; then
    echo "determinism: ${env_name} snapshot differed between two runs" >&2
    exit 1
  fi
  ok "${env_name}: byte-identical across two runs"
done

# ---------------------------------------------------------------------------
# Step 4: Constraint showcase — prod_forbids_debug is enforced
# ---------------------------------------------------------------------------
# 4a. cfx options: after selecting environment=prod, the guided walk no longer
#     offers log_level=debug. The invalid combination is unreachable by
#     construction, so a well-behaved config tool never lands on it.
banner "Step 4a: cfx options — debug is not offered once prod is selected"
OPTS="${OUT_DIR}/options.prod.json"
"${CFX}" options --model "${MODEL}" --select environment=prod --format json > "${OPTS}"
python3 -c 'import json,sys
rows=json.load(open(sys.argv[1]))
ll=[r for r in rows if r.get("facet")=="log_level"]
assert ll, "log_level facet missing from cfx options output"
opts=ll[0].get("valid_options",[])
assert "debug" not in opts, f"expected debug to be unavailable in prod, got {opts}"
assert opts==["info"], f"expected log_level=[info] in prod, got {opts}"
print("  -> log_level valid options in prod:", opts)' "${OPTS}"

# 4b. cfx explain: if you DO try the deliberately-invalid prod+debug selection,
#     cfx explain returns the minimal conflicting-choice core in plain text,
#     naming the constraint that forbids it by id (ADR-0054 section 5.4).
banner "Step 4b: cfx explain — why prod + log_level=debug is unsatisfiable"
BAD_SEL="${OUT_DIR}/.selection.prod-debug.json"
python3 -c 'import json,sys
json.dump({"schema_version":5,"model_hash":"","scope":"component:webapp",
          "context_tags":{},"choices":{"environment":"prod","log_level":"debug"},
          "selection_state_hash":""},open(sys.argv[1],"w"))' "${BAD_SEL}"
EXPLAIN_OUT="${OUT_DIR}/explain.prod-debug.txt"
explain_rc=0
"${CFX}" explain --model "${MODEL}" --selection-file "${BAD_SEL}" --format text \
  > "${EXPLAIN_OUT}" 2>&1 || explain_rc=$?
cat "${EXPLAIN_OUT}"
[[ "${explain_rc}" -eq 0 ]] || { echo "explain: expected a printed core (exit 0), got ${explain_rc}" >&2; exit 1; }
grep -q "log_level.debug" "${EXPLAIN_OUT}" || { echo "explain: core did not name log_level.debug" >&2; exit 1; }
grep -q "environment.prod" "${EXPLAIN_OUT}" || { echo "explain: core did not name environment.prod" >&2; exit 1; }
# ADR-0054 section 5.4: the core must name the CONSTRAINT, not just the two
# variables that collide. A core that only listed facets would leave the reader
# guessing which declaration ruled the combination out.
grep -q "blocked by constraint prod_forbids_debug" "${EXPLAIN_OUT}" \
  || { echo "explain: core did not name the prod_forbids_debug constraint" >&2; exit 1; }
ok "explain named the minimal conflict and the prod_forbids_debug constraint"

# 4c. cfx resolve: the constraint is enforced on the RESOLVE surface too. If you
#     bypass the guided walk and hand resolve the invalid selection directly, it
#     FAILS CLOSED — exit 3, naming the constraint it broke — and writes nothing.
#     A policy you can route around is not a policy.
banner "Step 4c: cfx resolve — prod + log_level=debug is rejected, no snapshot"
BAD_OUT="${OUT_DIR}/resolved-prod-debug"
RESOLVE_ERR="${OUT_DIR}/resolve.prod-debug.stderr"
resolve_rc=0
"${CFX}" resolve \
  --model "${MODEL}" \
  --select environment=prod \
  --select log_level=debug \
  --out "${BAD_OUT}" \
  --format text \
  > /dev/null 2> "${RESOLVE_ERR}" || resolve_rc=$?
cat "${RESOLVE_ERR}"
[[ "${resolve_rc}" -eq 3 ]] || { echo "resolve: expected exit 3 (unsatisfiable), got ${resolve_rc}" >&2; exit 1; }
grep -q "E_SELECTION_CONFLICT" "${RESOLVE_ERR}" || { echo "resolve: error did not carry E_SELECTION_CONFLICT" >&2; exit 1; }
grep -q "prod_forbids_debug" "${RESOLVE_ERR}" || { echo "resolve: error did not name the violated constraint" >&2; exit 1; }
grep -q "cfx explain" "${RESOLVE_ERR}" || { echo "resolve: error did not point at cfx explain" >&2; exit 1; }
# No partial output: the --out directory must not exist at all.
[[ ! -e "${BAD_OUT}" ]] || { echo "resolve: rejected selection still wrote ${BAD_OUT}" >&2; exit 1; }
ok "resolve rejected prod+debug (exit 3, named prod_forbids_debug) and wrote no snapshot"

# 4d. The three surfaces must agree about the SAME selection: options never
#     offers it, explain explains it, resolve refuses it. 4a/4b/4c above each
#     asserted one surface; this is the agreement itself, stated once.
ok "options / explain / resolve agree: prod + log_level=debug is not a valid configuration"

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - ONE model, THREE named environments (dev, staging, prod) resolved as"
echo "    selections over that model — three deterministic, hash-pinned snapshots,"
echo "    produced by a single 'cfx resolve --manifest environments.json --all'."
echo "  - prod hardens the shared defaults (request_timeout_ms 30000 -> 5000,"
echo "    deploy_tier nonprod -> production); dev and staging keep the defaults"
echo "    but record their own log_level / replica_class / beta_dashboard choices."
echo "  - Re-resolving is byte-identical: the snapshot is a pure function of the"
echo "    model and the selection."
echo "  - The prod_forbids_debug CONSTRAINT forbids debug logging in prod, and"
echo "    all three surfaces agree: cfx options hides the invalid option, cfx"
echo "    explain names prod_forbids_debug in the minimal conflict, and cfx"
echo "    resolve refuses the selection (exit 3) rather than emitting a"
echo "    snapshot. Policy is a constraints: declaration, not a component —"
echo "    the override conditions above select, they do not restrict."
