#!/usr/bin/env bash
# 05-compose-fleet — docker-compose fleet example.
#
# Models the real edge topology in miniature: ONE shared model, TWO named
# environments (robot-alpha and local), TWO service scopes, consumed by a
# containerized Python service. It wires together surfaces that already shipped
# (ADR-0032 decided all four are convention-only — no product code):
#
#   1. Compile the shared model (CUE was exported out-of-band to the committed
#      *.json; this script never invokes cue).
#   2. Matrix-resolve BOTH named environments x BOTH service scopes with the
#      one-shot `cfx resolve` — one command per (environment, scope) cell, no
#      hand-built request envelopes on the produce path. (The reference
#      resolver examples/resolve_environment.sh, which unfolds a named
#      environment through the raw interpreter envelope chain, remains as the
#      machine/envelope appendix for integrators who need it — ADR-0042.)
#   3. Assemble one delivery bundle per (environment, scope) — snapshot + ccm/ —
#      and verify each with the reference verifier examples/verify_bundle.sh.
#   4. Generate a docker-compose override from a resolve snapshot using the
#      USER-SIDE transform tools/gen_compose_override.py (the product does NOT
#      emit this shape — ADR-0032 D3; the script shows where it lives).
#   5. Run a docker-compose stack (robot-alpha) where the containerized service
#      loads its SCOPED snapshot at startup (Pattern 1: read the snapshot JSON
#      directly, verify resolve_hash, fail closed). If docker is unavailable the
#      compose phase is skipped with an explicit message.
#   6. Run the SAME service standalone against the local-environment bundle for
#      debugging (no compose) via run_standalone.sh.
#
# Distribution here is the honest minimum: a `cp` into the compose build
# context. No backend, no agent, no MQTT.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
# OUT_DIR defaults to ${EXAMPLE_DIR}/out but can be overridden so the script
# works under Bazel runfiles (read-only) or CI sandboxes.
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

# Read a JSON scalar from a file via python3 (no jq dependency in run.sh's own
# logic; the reference scripts use jq, which is checked before they are called).
jval() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }

COMPILER="$(find_binary CONFIGFLUX_COMPILER bazel-bin/compiler/compiler compiler)"
CFX="$(find_binary CONFIGFLUX_CFX bazel-bin/cfx/cfx cfx)"

VERIFIER="${REPO_ROOT}/examples/verify_bundle.sh"
GUARD="${REPO_ROOT}/examples/deploy_guard.sh"
OVERRIDE_GEN="${EXAMPLE_DIR}/tools/gen_compose_override.py"
RENDER_GEN="${EXAMPLE_DIR}/tools/render_app_config.py"

DEFS="${EXAMPLE_DIR}/00_definitions.json"
COMPONENTS="${EXAMPLE_DIR}/10_components.json"
MANIFEST="${EXAMPLE_DIR}/environments.json"

SCOPES="component:vision_service,component:telemetry_service"

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
# where this checkout lives. The compiler derives model_hash from the --source
# argument STRINGS plus content; filename-only ids keep it stable.
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
[[ -f "${OUT_DIR}/cmp/cmp.manifest.json" ]] || { echo "compile: cmp.manifest.json missing" >&2; exit 1; }
[[ -f "${OUT_DIR}/cmp/ccm/ccm.manifest.json" ]] || { echo "compile: sibling ccm/ missing" >&2; exit 1; }
ok "compiled -> ${OUT_DIR}/cmp/cmp.manifest.json (+ sibling ccm/)"

# ---------------------------------------------------------------------------
# Step 2: Matrix-resolve both named environments x both service scopes
# ---------------------------------------------------------------------------
banner "Step 2: Matrix-resolve (robot-alpha, local) x (vision_service, telemetry_service) with cfx resolve"
# Each (environment, scope) cell is one independent, deterministic resolution.
# The PRODUCE path is a single `cfx resolve` per cell: a small selection file
# carries the cell's scope + context_tags + choices (read straight from the
# environment manifest you own), and cfx opens -> applies -> resolves in one
# command. No open/init/select/resolve envelope threading, no jq on the produce
# path. The snapshot cfx prints (--format json) is byte-for-byte the same
# ResolveResult the raw envelope chain would produce.
RESOLVED_DIR="${OUT_DIR}/resolved"
CFX_EXPORT_DIR="${OUT_DIR}/.cfx-export"

# Resolve one (environment, scope) cell into
# ${RESOLVED_DIR}/<env>/<root>/resolve_result.<root>.<selection>.json — the
# standard per-scope bundle-snapshot layout. <selection> is the cell's choice
# values in sorted-key order joined with '-', falling back to the environment
# name when it has no choices.
resolve_cell() {
  local env_name="$1" scope="$2"
  local root="${scope#component:}"
  local cell_out="${RESOLVED_DIR}/${env_name}/${root}"
  mkdir -p "${cell_out}"
  # Build the cfx selection file from the manifest entry (scope + the env's
  # context_tags + choices). This is a declarative cell config, not a
  # hand-threaded request envelope.
  local sel="${OUT_DIR}/.selection.${env_name}.${root}.json"
  python3 -c 'import json,sys
m=json.load(open(sys.argv[1]));e=m["environments"][sys.argv[2]]
json.dump({"schema_version":2,"model_hash":"","scope":sys.argv[3],
          "context_tags":e.get("context_tags",{}),"choices":e.get("choices",{}),
          "selection_state_hash":""},open(sys.argv[4],"w"))' \
    "${MANIFEST}" "${env_name}" "${scope}" "${sel}"
  local label
  label="$(python3 -c 'import json,sys
c=json.load(open(sys.argv[1]))["environments"][sys.argv[2]].get("choices",{})
print("-".join(str(c[k]) for k in sorted(c)) if c else sys.argv[2])' "${MANIFEST}" "${env_name}")"
  "${CFX}" resolve \
    --model "${OUT_DIR}/cmp/cmp.manifest.json" \
    --selection-file "${sel}" \
    --out "${CFX_EXPORT_DIR}/${env_name}/${root}" \
    --format json \
    > "${cell_out}/resolve_result.${root}.${label}.json"
  rm -f "${sel}"
  echo "  -> ${env_name} x ${scope}: resolve_result.${root}.${label}.json"
}

# Enumerate the manifest's environments; resolve each across both service scopes.
IFS=',' read -r -a SCOPE_LIST <<< "${SCOPES}"
while IFS= read -r env_name; do
  [[ -z "${env_name}" ]] && continue
  for scope in "${SCOPE_LIST[@]}"; do
    resolve_cell "${env_name}" "${scope}"
  done
done < <(python3 -c 'import json,sys
for k in json.load(open(sys.argv[1]))["environments"]: print(k)' "${MANIFEST}")
ok "matrix resolve wrote per-(environment,scope) snapshots under ${RESOLVED_DIR}/"

# ---------------------------------------------------------------------------
# Step 3: Assemble one delivery bundle per (environment, scope) and verify each
# ---------------------------------------------------------------------------
# A delivery bundle is the standard one-unit delivery: exactly one
# resolve_result.*.json at its root plus a copy of the ccm/ solver model. We
# build four (2 environments x 2 scopes) and verify each with the reference
# verifier (snapshot model_hash == ccm/ bound_model_hash + content sha256).
banner "Step 3: Assemble + verify delivery bundles"
BUNDLES_DIR="${OUT_DIR}/bundles"
mkdir -p "${BUNDLES_DIR}"
bundle_count=0
while IFS= read -r snapshot; do
  [[ -z "${snapshot}" ]] && continue
  rel="${snapshot#"${RESOLVED_DIR}/"}"   # <env>/<root>/resolve_result.*.json
  env_name="${rel%%/*}"
  rest="${rel#*/}"
  root="${rest%%/*}"
  bundle_dir="${BUNDLES_DIR}/${env_name}--${root}"
  mkdir -p "${bundle_dir}"
  cp "${snapshot}" "${bundle_dir}/"
  cp -R "${OUT_DIR}/cmp/ccm" "${bundle_dir}/ccm"
  "${VERIFIER}" "${bundle_dir}" >/dev/null
  ok "bundle ${env_name}/${root}: verified (snapshot + ccm/ matched)"
  bundle_count=$((bundle_count + 1))
done < <(find "${RESOLVED_DIR}" -name 'resolve_result.*.json' | sort)
ok "${bundle_count} bundles assembled and verified"

# Resolve the four canonical bundle snapshot paths for later steps.
ra_vision_bundle="${BUNDLES_DIR}/robot-alpha--vision_service"
ra_telem_bundle="${BUNDLES_DIR}/robot-alpha--telemetry_service"
local_vision_bundle="${BUNDLES_DIR}/local--vision_service"
ra_vision_snap="$(find "${ra_vision_bundle}" -maxdepth 1 -name 'resolve_result.*.json' | head -n1)"
ra_telem_snap="$(find "${ra_telem_bundle}" -maxdepth 1 -name 'resolve_result.*.json' | head -n1)"

# ---------------------------------------------------------------------------
# Step 4: Generate a docker-compose override (USER-SIDE transform)
# ---------------------------------------------------------------------------
# WHERE override generation lives: in the user's repo, as a thin transform over
# the resolved snapshot. The product does NOT emit compose overrides (ADR-0032
# D3). This is purely to demonstrate the seam.
banner "Step 4: Generate compose override (user-side, not a product output)"
python3 "${OVERRIDE_GEN}" \
  --snapshot "${ra_vision_snap}" \
  --service vision-service \
  --out "${OUT_DIR}/vision-service.compose.override.yml"
ok "wrote ${OUT_DIR}/vision-service.compose.override.yml (env-block from resolve output)"

# ---------------------------------------------------------------------------
# Step 4a: Render a NESTED app-config document (USER-SIDE transform)
# ---------------------------------------------------------------------------
# A SECOND user-side transform, parallel to Step 4 but emitting the NESTED shape
# layered app-config files take in common service frameworks (a hierarchical
# settings document, not flat KEY=value). Same seam, same posture: the product
# emits the rich snapshot; shaping it into a framework's layered config file is
# user code (ADR-0035 C3 / ADR-0032 D3). Services "keep mounting the same file".
banner "Step 4a: Render nested app-config (user-side, not a product output)"
python3 "${RENDER_GEN}" \
  --snapshot "${ra_vision_snap}" \
  --out "${OUT_DIR}/vision-service.app-config.json"
ok "wrote ${OUT_DIR}/vision-service.app-config.json (nested component->param->value + lineage)"

# ---------------------------------------------------------------------------
# Step 4b: Deploy guard — provenance gate over a (stamped) snapshot
# ---------------------------------------------------------------------------
# The deploy guard (examples/deploy_guard.sh, ADR-0035 C2) refuses to deploy an
# OVERLAY-ACTIVE resolution to a NON-LOCAL-CLASS target unless --allow-overlay is
# passed. It reads the snapshot's context_tags.overlay stamp and the target
# environment's `class` from the manifest (here, `local` is marked
# class: "local"; robot-alpha is unclassified => non-local-class). To demonstrate
# the guard WITHOUT building user-side overlay composition (out of scope), we
# stamp context_tags.overlay="dev" into a COPY of an already-resolved snapshot.
banner "Step 4b: Deploy guard (provenance gate, reference script)"

# (i) ALLOW: a non-overlay snapshot to the local-class environment.
"${GUARD}" --snapshot "${ra_vision_snap}" --manifest "${MANIFEST}" --environment local >/dev/null
ok "guard ALLOW: non-overlay snapshot -> 'local' (local-class)"

# Stamp an overlay-active copy (simulates "an overlay was active").
overlay_snap="${OUT_DIR}/vision.overlay.json"
python3 -c 'import json,sys
d=json.load(open(sys.argv[1])); d.setdefault("context_tags",{})["overlay"]="dev"
json.dump(d,open(sys.argv[2],"w"),indent=2)' "${ra_vision_snap}" "${overlay_snap}"

# (ii) REFUSE: that overlay-active snapshot to robot-alpha (non-local-class),
#      no override. The guard exits 1; we assert that and continue.
guard_rc=0
"${GUARD}" --snapshot "${overlay_snap}" --manifest "${MANIFEST}" --environment robot-alpha >/dev/null 2>&1 \
  || guard_rc=$?
[[ "${guard_rc}" -eq 1 ]] || { echo "guard: expected REFUSE (exit 1) for overlay->robot-alpha, got ${guard_rc}" >&2; exit 1; }
ok "guard REFUSE: overlay-active snapshot -> 'robot-alpha' (non-local-class), no --allow-overlay (exit 1)"

# (iii) ALLOW with the explicit override.
"${GUARD}" --snapshot "${overlay_snap}" --manifest "${MANIFEST}" --environment robot-alpha --allow-overlay >/dev/null
ok "guard ALLOW: same overlay snapshot -> 'robot-alpha' WITH --allow-overlay (explicit operator choice)"

# ---------------------------------------------------------------------------
# Step 5: docker-compose stack (robot-alpha) — or skip if docker is absent
# ---------------------------------------------------------------------------
banner "Step 5: docker-compose stack (robot-alpha node, two scoped services)"
# Assemble the compose build context: the service code + the robot-alpha
# bundles + a generated .env carrying the expected resolve_hashes. Distribution
# is a plain cp into the build context (the honest minimum).
COMPOSE_BUILD="${OUT_DIR}/compose-build"
mkdir -p "${COMPOSE_BUILD}/bundles"
cp "${EXAMPLE_DIR}/service/app.py" "${COMPOSE_BUILD}/app.py"
cp "${EXAMPLE_DIR}/service/Dockerfile" "${COMPOSE_BUILD}/Dockerfile"
cp -R "${ra_vision_bundle}" "${COMPOSE_BUILD}/bundles/robot-alpha--vision_service"
cp -R "${ra_telem_bundle}"  "${COMPOSE_BUILD}/bundles/robot-alpha--telemetry_service"

# Generate the .env compose auto-loads: snapshot basenames + expected hashes.
{
  echo "VISION_SNAPSHOT_NAME=$(basename "${ra_vision_snap}")"
  echo "TELEMETRY_SNAPSHOT_NAME=$(basename "${ra_telem_snap}")"
  echo "VISION_RESOLVE_HASH=$(jval "${ra_vision_snap}" resolve_hash)"
  echo "TELEMETRY_RESOLVE_HASH=$(jval "${ra_telem_snap}" resolve_hash)"
} > "${COMPOSE_BUILD}/.env"
cp "${EXAMPLE_DIR}/docker-compose.yml" "${COMPOSE_BUILD}/docker-compose.yml"
ok "compose build context assembled at ${COMPOSE_BUILD}/"

# Detect docker + the compose subcommand. Fall back to docker-compose v1.
DOCKER_OK=0
COMPOSE_CMD=()
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  if docker compose version >/dev/null 2>&1; then
    DOCKER_OK=1; COMPOSE_CMD=(docker compose)
  elif command -v docker-compose >/dev/null 2>&1 && docker-compose version >/dev/null 2>&1; then
    DOCKER_OK=1; COMPOSE_CMD=(docker-compose)
  fi
fi

if [[ "${DOCKER_OK}" -eq 1 ]]; then
  ok "docker available — building and bringing up the robot-alpha compose stack"
  compose_rc=0
  (
    cd "${COMPOSE_BUILD}"
    # Build the shared image first so the second service reuses it rather than
    # attempting a pull of a not-yet-built local image. Then bring the stack up,
    # aborting (and propagating the exit code) when the first service exits.
    "${COMPOSE_CMD[@]}" build
    "${COMPOSE_CMD[@]}" up --no-build --abort-on-container-exit --exit-code-from vision-service
  ) || compose_rc=$?
  # Always tear the stack down (best-effort), regardless of the run outcome.
  ( cd "${COMPOSE_BUILD}" && "${COMPOSE_CMD[@]}" down --rmi local --volumes --remove-orphans ) \
    >/dev/null 2>&1 || true
  if [[ "${compose_rc}" -ne 0 ]]; then
    echo "compose: stack did not exit cleanly (rc=${compose_rc})" >&2
    exit "${compose_rc}"
  fi
  ok "compose stack came up; both scoped services loaded their config and exited 0"
else
  ok "docker not available — skipping compose phase; the standalone path below exercises the SAME service"
fi

# ---------------------------------------------------------------------------
# Step 6: Run the SAME service standalone against the local bundle (debugging)
# ---------------------------------------------------------------------------
banner "Step 6: Standalone debug run (local environment, no compose)"
[[ -d "${local_vision_bundle}" ]] || { echo "local bundle missing" >&2; exit 1; }
CONFIGFLUX_EXAMPLE_OUT_DIR="${OUT_DIR}" "${EXAMPLE_DIR}/run_standalone.sh" vision_service
ok "standalone service loaded the local bundle and verified its lineage"

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - ONE shared model resolved for TWO named environments (robot-alpha,"
echo "    local) x TWO service scopes (vision_service, telemetry_service) ="
echo "    four independent, deterministic snapshots."
echo "  - Each snapshot ships as a delivery bundle (snapshot + ccm/) verified"
echo "    before delivery; the on-target guarantee is the fail-closed runtime"
echo "    open, which needs the ccm/ alongside the snapshot (ADR-0030 D2)."
echo "  - The containerized service is a Pattern 1 consumer: it reads the"
echo "    scoped snapshot JSON directly, pins resolve_hash, and fails closed."
echo "  - The compose override is generated by a USER-SIDE transform"
echo "    (tools/gen_compose_override.py); the product emits the snapshot, not"
echo "    the override (ADR-0032 D3)."
echo "  - A SECOND user-side transform (tools/render_app_config.py) renders the"
echo "    same snapshot into a NESTED app-config document — the layered shape"
echo "    service frameworks mount — again from the snapshot, not the product"
echo "    (ADR-0035 C3)."
echo "  - The deploy guard (deploy_guard.sh) refuses an overlay-active snapshot"
echo "    targeting a non-local-class environment unless --allow-overlay is"
echo "    passed; a local-class target is always allowed (ADR-0035 C2)."
echo "  - The SAME service runs standalone against the local bundle for"
echo "    debugging — identical code, different snapshot."
