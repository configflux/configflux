#!/usr/bin/env bash
# 04-fleet-edge-node — full pipeline with runtime handoff
#
# Compiles a fleet edge-node model, resolves it against a 3-facet selection
# (device_class, update_channel, region) with the one-shot `cfx resolve`, and
# hands the resolved output to the runtime binary. Demonstrates:
#   - Full pipeline: compiler -> cfx resolve -> runtime
#   - PRODUCE a config with a single `cfx resolve` — no hand-built request
#     envelopes, no jq plumbing (the produce path is one command)
#   - 3 selection facets driving overrides across multiple components
#   - Region-based conditional component (regional_compliance)
#   - Runtime handoff: runtime-open consuming the resolve snapshot, then
#     get-scope-metadata and list-parameters against it. The runtime speaks
#     the JSON request/response ENVELOPE protocol (ADR-0042: the machine /
#     agent seam); that appendix keeps its raw envelopes on purpose.
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
  echo >&2 "  Either build it:  bazel build //compiler //cfx //runtime"
  echo >&2 "  Or set:           export ${env_var}=/path/to/${pretty}"
  exit 1
}

banner() { printf "\n=== %s ===\n" "$1"; }
ok()     { printf "  -> %s\n" "$1"; }

COMPILER="$(find_binary CONFIGFLUX_COMPILER bazel-bin/compiler/compiler compiler)"
CFX="$(find_binary CONFIGFLUX_CFX bazel-bin/cfx/cfx cfx)"
RUNTIME="$(find_binary CONFIGFLUX_RUNTIME bazel-bin/runtime/runtime runtime)"

SOURCES=(
  --source "${EXAMPLE_DIR}/00_definitions.json"
  --source "${EXAMPLE_DIR}/10_components.json"
)

SCOPE="component:runtime_tuner"
SCOPE_ROOT="runtime_tuner"

# ---------------------------------------------------------------------------
# Pipeline
# ---------------------------------------------------------------------------

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

banner "Step 1: Compile"
"${COMPILER}" compile \
  "${SOURCES[@]}" \
  --out "${OUT_DIR}" \
  > "${OUT_DIR}/compile_result.json"
ok "Compiled model -> ${OUT_DIR}/cmp.manifest.json"

banner "Step 2: Verify"
"${COMPILER}" verify \
  "${SOURCES[@]}" \
  > "${OUT_DIR}/verify_report.json"
ok "Verification passed (0 errors, 0 warnings)"

banner "Step 3: Produce the resolved config with cfx resolve"
# The PRODUCE path is one command. `cfx resolve` opens the compiled model,
# applies the selection, resolves, and writes the resolved snapshot — no
# hand-built request envelopes, no jq threading of model handles and
# selection state between verbs (contrast the machine/envelope appendix
# below, which the runtime genuinely needs).
#
# The scope (`component:runtime_tuner`) is pinned via a small selection file:
# `cfx` has no --scope flag; the selection file carries the scope, context
# tags, and any base choices. The three facet choices are `--select` flags
# (order-independent: cfx applies them as a set). `--format json` prints the
# resolved snapshot — the SAME ResolveResult envelope the interpreter emits,
# byte-for-byte — which the runtime handoff below consumes. `--out` also
# exports the C++ early-binding snapshot (a side artifact this example does
# not otherwise use).
printf '{"schema_version":3,"model_hash":"","scope":"%s","context_tags":{},"choices":{},"selection_state_hash":""}\n' \
  "${SCOPE}" > "${OUT_DIR}/selection.json"
"${CFX}" resolve \
  --model "${OUT_DIR}/cmp.manifest.json" \
  --selection-file "${OUT_DIR}/selection.json" \
  --select device_class=gateway \
  --select update_channel=canary \
  --select region=eu \
  --out "${OUT_DIR}/export" \
  --format json \
  > "${OUT_DIR}/resolve.result.json"
ok "device_class = gateway, update_channel = canary, region = eu"
ok "resolve_hash established -> ${OUT_DIR}/resolve.result.json"

# ---------------------------------------------------------------------------
# Runtime handoff — the machine / envelope seam (ADR-0042)
# ---------------------------------------------------------------------------
# `cfx` is the human produce seam; the runtime speaks the JSON request/response
# ENVELOPE protocol (the machine/agent seam, ADR-0042). The steps below
# deliberately keep their raw envelopes: a machine integrator wiring the
# resolved snapshot into a validated runtime session needs exactly this shape,
# and there is no cfx runtime verb. This is the envelope appendix, not the
# produce path.

banner "Step 4: Runtime — runtime-open (handoff from the resolve snapshot)"
# Reshape the resolve snapshot into the runtime-open request envelope. The
# `.ccm` reference is the sibling ccm/ directory the compile step emitted next
# to cmp.manifest.json: ADR-0030 D2 makes a usable `.ccm` a hard precondition
# for runtime-open, so the handoff must thread it through. All other fields
# beyond the core lineage default safely.
jq -n \
  --slurpfile r "${OUT_DIR}/resolve.result.json" \
  --arg ccm_ref "${OUT_DIR}/ccm" \
  '{schema_version: 3,
     model_hash: $r[0].model_hash,
     ccm_ref: $ccm_ref,
     resolve_hash: $r[0].resolve_hash,
     scope: $r[0].scope,
     resolved_output: $r[0].resolved_output,
     resolved_component_dependencies: ($r[0].resolved_component_dependencies // {}),
     resolved_artifacts: ($r[0].resolved_artifacts // {}),
     context_tags: ($r[0].context_tags // {}),
     choices: ($r[0].choices // {})}' \
  > "${OUT_DIR}/runtime_open.request.json"
"${RUNTIME}" runtime-open \
  --request-file "${OUT_DIR}/runtime_open.request.json" \
  --response-file "${OUT_DIR}/runtime_open.result.json"
ok "runtime_snapshot acquired"

banner "Step 5: Runtime — get-scope-metadata"
jq -n \
  --slurpfile ro "${OUT_DIR}/runtime_open.result.json" \
  --arg scope_root "${SCOPE_ROOT}" \
  '{schema_version: 3, runtime_snapshot: $ro[0].runtime_snapshot, scope_root: $scope_root}' \
  > "${OUT_DIR}/get_scope_metadata.request.json"
"${RUNTIME}" get-scope-metadata \
  --request-file "${OUT_DIR}/get_scope_metadata.request.json" \
  --response-file "${OUT_DIR}/get_scope_metadata.result.json"
COMPONENT_COUNT="$(jq '.metadata.component_count' "${OUT_DIR}/get_scope_metadata.result.json")"
PARAMETER_COUNT="$(jq '.metadata.parameter_count' "${OUT_DIR}/get_scope_metadata.result.json")"
ARTIFACT_COUNT="$(jq '.metadata.artifact_count' "${OUT_DIR}/get_scope_metadata.result.json")"
ok "scope metadata: ${COMPONENT_COUNT} components, ${PARAMETER_COUNT} parameters, ${ARTIFACT_COUNT} artifact"

banner "Step 6: Runtime — list-parameters"
jq -n \
  --slurpfile ro "${OUT_DIR}/runtime_open.result.json" \
  --arg scope_root "${SCOPE_ROOT}" \
  '{schema_version: 3, runtime_snapshot: $ro[0].runtime_snapshot, scope_root: $scope_root}' \
  > "${OUT_DIR}/list_parameters.request.json"
"${RUNTIME}" list-parameters \
  --request-file "${OUT_DIR}/list_parameters.request.json" \
  --response-file "${OUT_DIR}/list_parameters.result.json"
PARAM_PATH_COUNT="$(jq '.parameter_paths | length' "${OUT_DIR}/list_parameters.result.json")"
ok "${PARAM_PATH_COUNT} parameter paths listed"

banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - The config is PRODUCED by one 'cfx resolve' command — no hand-built"
echo "    request envelopes on the produce path"
echo "  - 3 selection facets (device_class, update_channel, region) drive"
echo "    overrides on firmware, endpoint, and poll_interval"
echo "  - Region-conditional component regional_compliance is present in"
echo "    the compiled model only when region == 'eu'"
echo "  - The runtime handoff (the machine/envelope seam) reuses the resolve"
echo "    lineage (model_hash + resolve_hash) so runtime state is bound to the"
echo "    resolved 100% configuration"
echo ""
echo "Try exploring:"
echo "  jq .resolved_output ${OUT_DIR}/resolve.result.json"
echo "  jq .parameter_paths ${OUT_DIR}/list_parameters.result.json"
echo "  jq .metadata        ${OUT_DIR}/get_scope_metadata.result.json"
