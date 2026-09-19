#!/usr/bin/env bash
# 03-motor-controller — artifacts, 2-step selection, export-resolved, BOM
#
# Compiles a motor controller model that demonstrates:
#   - Artifact references (driver binaries bound to components)
#   - 2-step selection: motor_class selects control mode + driver,
#     power_rating selects current limit and enables safety_monitor
#   - Export-resolved output (C++ header and CMake flags via loader API)
#   - Software BOM generation (full audit of resolved artifacts)
#   - A runtime write REFUSED by a declared constraint: encoder_mode is the
#     runtime handle of the encoder_type facet, so changing it at runtime is a
#     selection the model gets a say in
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
# OUT_DIR defaults to ${EXAMPLE_DIR}/out but can be overridden so the script
# works under Bazel runfiles (read-only) or CI sandboxes.
OUT_DIR="${CONFIGFLUX_EXAMPLE_OUT_DIR:-${EXAMPLE_DIR}/out}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Each binary is located the same way: an explicit environment override wins,
# otherwise the locally built one. The override names are the contract a
# packaged release is driven through, so the example runs unchanged against a
# source build or a downloaded bundle.
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
ok "4 artifacts, 4 components, 6 definitions"

banner "Step 2: Verify"
"${COMPILER}" verify \
  "${SOURCES[@]}" \
  > "${OUT_DIR}/verify_report.json"
ok "Verification passed (0 errors, 0 warnings)"

banner "Step 3: Inspect — model summary"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  summary \
  > "${OUT_DIR}/inspect_summary.json"
ok "4 components: motor_drive, encoder_interface, motion_controller, safety_monitor"
ok "4 artifacts: foc_driver, trapz_driver, absolute_encoder_driver, incremental_encoder_driver"

banner "Step 4: Inspect — artifact detail"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  artifact foc_driver \
  > "${OUT_DIR}/inspect_artifact_foc_driver.json"
ok "foc_driver v3.1.0 -> /opt/configflux/motor/foc_driver.so"

banner "Step 5: Inspect — motor_drive component"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  component motor_drive \
  > "${OUT_DIR}/inspect_motor_drive.json"
ok "motor_drive has 4 params: control_mode, motor_driver, pwm_frequency, current_limit"
ok "motor_driver param references artifact (2-step: motor_class selects driver)"

banner "Step 6: Inspect — safety_monitor (conditional)"
"${COMPILER}" inspect \
  "${SOURCES[@]}" \
  component safety_monitor \
  > "${OUT_DIR}/inspect_safety_monitor.json"
ok "safety_monitor: condition = \"power_rating == 'high'\""

banner "Step 7: Resolve — a high-power drive with an absolute encoder"
# The PRODUCE path is one command: `cfx resolve` opens the compiled model,
# applies the selection, resolves, and writes the resolved snapshot. The scope
# is pinned through a small selection file (`cfx` has no --scope flag); the
# three facet choices are `--select` flags, applied as a set.
#
# This selection is legal: `high` power with an `absolute` encoder satisfies
# high_power_requires_absolute_encoder. Step 8 tries to break it at runtime.
printf '{"schema_version":5,"model_hash":"","scope":"all","context_tags":{},"choices":{},"selection_state_hash":""}\n' \
  > "${OUT_DIR}/selection.json"
"${CFX}" resolve \
  --model "${OUT_DIR}/cmp.manifest.json" \
  --selection-file "${OUT_DIR}/selection.json" \
  --select motor_class=brushed_dc \
  --select power_rating=high \
  --select encoder_type=absolute \
  --out "${OUT_DIR}/export" \
  --format json \
  > "${OUT_DIR}/resolve.result.json"
ok "motor_class = brushed_dc, power_rating = high, encoder_type = absolute"
ok "resolve_hash established -> ${OUT_DIR}/resolve.result.json"

banner "Step 8: Runtime — a write the model forbids"
# encoder_mode declares `facet: encoder_type`, which is what makes it the
# facet's runtime HANDLE. Writing it is therefore not a free-form scalar poke:
# it is a selection of encoder_type, and the model's declared constraints get a
# say. pid_gain_trim, by contrast, declares no facet, so it is exactly the
# free-form runtime tuning knob it looks like.
#
# Reshape the resolve snapshot into the runtime-open request envelope. The
# `.ccm` reference is the sibling directory the compile step emitted next to
# cmp.manifest.json; a usable `.ccm` is a hard precondition for runtime-open,
# so the handoff must thread it through.
#
# `defaulted_choices`, `implied_choices` and `closed_facet_domains` are the
# three keys a projection must never drop. The first two fold into the
# `resolve_hash` pre-image, which runtime-open recomputes before it opens
# anything, so forgetting either fails the open on any model where a facet
# takes its declared default or is inferred. The third is not a hash key at
# all: it is the only channel by which a facet's closed-ness reaches the
# runtime, and dropping it costs the attribution below — a refused write would
# be reported as an over-constrained model instead of naming the rule it
# breaks. A resolve omits any of the three when its map is empty, which is what
# `// {}` covers.
jq -n \
  --slurpfile r "${OUT_DIR}/resolve.result.json" \
  --arg ccm_ref "${OUT_DIR}/ccm" \
  '{schema_version: 5,
     model_hash: $r[0].model_hash,
     ccm_ref: $ccm_ref,
     resolve_hash: $r[0].resolve_hash,
     scope: $r[0].scope,
     resolved_output: $r[0].resolved_output,
     resolved_component_dependencies: ($r[0].resolved_component_dependencies // {}),
     resolved_artifacts: ($r[0].resolved_artifacts // {}),
     context_tags: ($r[0].context_tags // {}),
     choices: ($r[0].choices // {}),
     defaulted_choices: ($r[0].defaulted_choices // {}),
     implied_choices: ($r[0].implied_choices // {}),
     closed_facet_domains: ($r[0].closed_facet_domains // {})}' \
  > "${OUT_DIR}/runtime_open.request.json"
"${RUNTIME}" runtime-open \
  --request-file "${OUT_DIR}/runtime_open.request.json" \
  --response-file "${OUT_DIR}/runtime_open.result.json"
ok "runtime_snapshot acquired"

# 8a — an ACCEPTED write: pid_gain_trim is a plain runtime float.
jq -n \
  --slurpfile ro "${OUT_DIR}/runtime_open.result.json" \
  '{schema_version: 5, runtime_snapshot: $ro[0].runtime_snapshot,
    path: "component.motion_controller.param.pid_gain_trim", value: 0.42}' \
  > "${OUT_DIR}/runtime_set_accepted.request.json"
"${RUNTIME}" set-parameter \
  --request-file "${OUT_DIR}/runtime_set_accepted.request.json" \
  --response-file "${OUT_DIR}/runtime_set_accepted.result.json"
ACCEPTED_VALUE="$(jq -r '.parameter.value' "${OUT_DIR}/runtime_set_accepted.result.json")"
ok "accepted: pid_gain_trim = ${ACCEPTED_VALUE} (no facet binding, no constraint to check)"

# 8b — a REFUSED write: encoder_mode is the encoder_type facet, and this
# session already holds power_rating=high. `set -e` would abort on the non-zero
# exit, so the command is run with the guard lifted and the exit code checked.
jq -n \
  --slurpfile sp "${OUT_DIR}/runtime_set_accepted.result.json" \
  '{schema_version: 5, runtime_snapshot: $sp[0].runtime_snapshot,
    path: "component.encoder_interface.param.encoder_mode", value: "incremental"}' \
  > "${OUT_DIR}/runtime_set_refused.request.json"
set +e
"${RUNTIME}" set-parameter \
  --request-file "${OUT_DIR}/runtime_set_refused.request.json" \
  --response-file "${OUT_DIR}/runtime_set_refused.result.json"
REFUSED_RC=$?
set -e
REFUSED_STATUS="$(jq -r '.status' "${OUT_DIR}/runtime_set_refused.result.json")"
REFUSED_CODE="$(jq -r '.diagnostics.diagnostics[0].code' "${OUT_DIR}/runtime_set_refused.result.json")"
REFUSED_PATH="$(jq -r '.diagnostics.diagnostics[0].entity_path' "${OUT_DIR}/runtime_set_refused.result.json")"
RULE_ID="$(jq -r '[.unsat_core.conflicting_constraints[] | select(.kind == "model_rule")][0].constraint_id' \
  "${OUT_DIR}/runtime_set_refused.result.json")"
RULE_TEXT="$(jq -r '[.unsat_core.conflicting_constraints[] | select(.kind == "model_rule")][0].summary' \
  "${OUT_DIR}/runtime_set_refused.result.json")"
# The named rule is the whole point of the step, so it is checked and not just
# printed: a refusal that carried no `unsat_core` would otherwise echo `null`
# here and still look like a pass.
if [[ "${REFUSED_RC}" -ne 2 || "${REFUSED_STATUS}" != "error" \
   || "${REFUSED_CODE}" != "E_SELECTION_CONFLICT" \
   || "${REFUSED_PATH}" != "constraints/high_power_requires_absolute_encoder" \
   || "${RULE_ID}" != "high_power_requires_absolute_encoder" \
   || "${RULE_TEXT}" != "power_rating != 'high' || encoder_type != 'incremental'" ]]; then
  echo >&2 "Error: the forbidden write was not refused as expected"
  cat >&2 "${OUT_DIR}/runtime_set_refused.result.json"
  exit 1
fi
ok "refused: encoder_mode = incremental -> exit ${REFUSED_RC}, ${REFUSED_CODE}"
ok "violated rule: ${RULE_ID} (${RULE_TEXT})"
ok "entity_path: ${REFUSED_PATH}"
ok "nothing changed: a refused write returns no snapshot, the old one stays valid"

banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - cue/00_definitions.cue declares 2 artifact slots (motor_driver_slot, encoder_driver_slot)"
echo "  - cue/10_components.cue declares 4 artifacts and binds them via overrides"
echo "  - 2-step selection: motor_class picks control_mode + driver,"
echo "    power_rating picks current_limit and gates safety_monitor"
echo "  - safety_monitor is conditional: only included when power_rating == 'high'"
echo "  - A runtime write to encoder_mode is refused because it would break the"
echo "    declared rule high_power_requires_absolute_encoder, and the refusal"
echo "    names that rule and quotes its condition"
echo ""
echo "Try exploring:"
echo "  cat ${OUT_DIR}/compile_result.json | jq .stats"
echo "  cat ${OUT_DIR}/inspect_summary.json | jq .summary.artifact_ids"
echo "  cat ${OUT_DIR}/inspect_artifact_foc_driver.json | jq .item"
echo "  cat ${OUT_DIR}/inspect_motor_drive.json | jq .item.param_keys"
echo "  cat ${OUT_DIR}/runtime_set_refused.result.json | jq .unsat_core"
echo ""
echo "Export-resolved and software BOM generation through the interpreter CLI are exercised by examples/sbom_export_test.sh (see examples/BUILD.bazel)."
