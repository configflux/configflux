// SPDX-License-Identifier: BUSL-1.1

use crate::ir;
use crate::loader_api::{
    canonical_selection_state, open_model, resolve_from_selection, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, SelectionState, E_RESOLVE_CONTEXT_UNSATISFIED,
    E_RESOLVE_SCOPE_INVALID, E_SELECTION_STATE_INVALID,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::scenario_test_support::unique_temp_path;
use crate::Compiler;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_GOLDEN_RESOLVE_RESULT: &str = include_str!(
    "../scenarios/s1_water_pump/smoke/golden/resolve_result.thermal_control.hydra_x200_dual_us.json"
);

const S5_SOURCE_DEFS: &str = "scenarios/s5_building_hvac/smoke/chunks/00_definitions.toml";
const S5_SOURCE_COMPONENTS: &str = "scenarios/s5_building_hvac/smoke/chunks/10_components.toml";
const S5_CHUNK_DEFS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/00_definitions.json");
const S5_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/10_components.json");
const S5_GOLDEN_RESOLVE_RESULT: &str = include_str!(
    "../scenarios/s5_building_hvac/smoke/golden/resolve_result.climate_controller.hospital_hepa_us.json"
);

#[derive(Debug, Deserialize)]
struct ResolveGolden {
    scope: String,
    resolve_hash: String,
    // ADR-0059 D3. Pinned from the golden FILE as well as from the frozen
    // literal in `resolved_output_hash_preimage_is_pinned`, so the shipped
    // snapshot and the pre-image contract cannot drift apart silently.
    resolved_output_hash: String,
    resolved_output: JsonValue,
}

fn emitted_cmp_dir(chunks: &[(&str, &str)], label: &str) -> Result<(PathBuf, ir::IrIndex)> {
    let temp_dir = unique_temp_path("cfx-resolve", label);
    std::fs::create_dir_all(&temp_dir)
        .with_context(|| format!("Failed to create temp dir '{}'", temp_dir.display()))?;

    let mut compiler = Compiler::new();
    for (source_id, chunk) in chunks {
        compiler
            .add_chunk_auto(*source_id, chunk)
            .with_context(|| format!("Failed to add chunk '{}'", source_id))?;
    }

    let index = compiler.emit_ir(&temp_dir)?;
    Ok((temp_dir, index))
}

fn open_handle(cmp_dir: &Path) -> Result<ModelHandle> {
    let manifest_path = cmp_dir.join(ir::CMP_DEFAULT_MANIFEST_FILENAME);
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });
    if result.status != OperationStatus::Ok {
        anyhow::bail!("open_model failed: {:?}", result.diagnostics.diagnostics);
    }
    result.model_handle.context("missing model_handle")
}

fn s1_default_context() -> BTreeMap<String, String> {
    let mut context = BTreeMap::new();
    context.insert("cooling_brand".to_string(), "hydra".to_string());
    context.insert("cooling_model".to_string(), "x200".to_string());
    context.insert("pump_type".to_string(), "dual".to_string());
    context.insert("region".to_string(), "us".to_string());
    context
}

fn s5_default_context() -> BTreeMap<String, String> {
    let mut context = BTreeMap::new();
    context.insert("occupancy_class".to_string(), "hospital".to_string());
    context.insert("filtration_grade".to_string(), "hepa".to_string());
    context.insert("region".to_string(), "us".to_string());
    context
}

fn state_from_payload(
    model_handle: &ModelHandle,
    scope: &str,
    context_tags: BTreeMap<String, String>,
    choices: BTreeMap<String, String>,
) -> Result<SelectionState> {
    canonical_selection_state(
        model_handle.model_hash.clone(),
        scope.to_string(),
        context_tags,
        choices,
    )
}

fn resolve(
    model_handle: &ModelHandle,
    scope: &str,
    state: &SelectionState,
) -> crate::loader_api::ResolveResult {
    resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: model_handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
        implied_choices: Default::default(),
    })
}

fn load_resolve_golden(contents: &str) -> Result<ResolveGolden> {
    serde_json::from_str(contents).context("Failed to parse resolve-result golden JSON")
}

fn read_vm_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let value = rest.split_whitespace().next()?;
            return value.parse::<u64>().ok();
        }
    }
    None
}

#[test]
fn resolve_contract_s1_resolve_result_envelope_has_required_fields() -> Result<()> {
    let (temp_dir, index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-contract",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;

    let result = resolve(&handle, scope, &state);
    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(result.model_hash, index.config_hash);
    assert_eq!(result.scope, scope);
    assert_eq!(result.selection_state_hash, state.selection_state_hash);
    assert_eq!(result.error_count, 0);
    assert_eq!(result.warning_count, 0);
    assert!(result.resolve_hash.is_some());
    assert!(result.resolved_output.is_some());
    assert_eq!(result.diagnostics.error_count, 0);
    assert_eq!(result.diagnostics.warning_count, 0);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn resolve_golden_s1_resolve_result_and_hash_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-golden",
    )?;
    let handle = open_handle(&temp_dir)?;

    let golden = load_resolve_golden(S1_GOLDEN_RESOLVE_RESULT)?;
    let state = state_from_payload(
        &handle,
        &golden.scope,
        s1_default_context(),
        BTreeMap::new(),
    )?;
    let result = resolve(&handle, &golden.scope, &state);

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(
        result.resolve_hash.as_deref(),
        Some(golden.resolve_hash.as_str())
    );
    assert_eq!(
        result.resolved_output_hash.as_deref(),
        Some(golden.resolved_output_hash.as_str())
    );
    assert_eq!(result.resolved_output, Some(golden.resolved_output));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn resolve_golden_s5_resolve_result_and_hash_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S5_SOURCE_DEFS, S5_CHUNK_DEFS),
            (S5_SOURCE_COMPONENTS, S5_CHUNK_COMPONENTS),
        ],
        "s5-golden",
    )?;
    let handle = open_handle(&temp_dir)?;

    let golden = load_resolve_golden(S5_GOLDEN_RESOLVE_RESULT)?;
    let state = state_from_payload(
        &handle,
        &golden.scope,
        s5_default_context(),
        BTreeMap::new(),
    )?;
    let result = resolve(&handle, &golden.scope, &state);

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(
        result.resolve_hash.as_deref(),
        Some(golden.resolve_hash.as_str())
    );
    assert_eq!(
        result.resolved_output_hash.as_deref(),
        Some(golden.resolved_output_hash.as_str())
    );
    assert_eq!(result.resolved_output, Some(golden.resolved_output));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn resolve_determinism_identical_calls_stable_hash_and_payload() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-determinism-identical",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;

    let first = resolve(&handle, scope, &state);
    let second = resolve(&handle, scope, &state);

    assert_eq!(first.status, OperationStatus::Ok);
    assert_eq!(first, second);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn resolve_determinism_selection_key_order_does_not_change_resolve_hash() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-determinism-key-order",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";

    let mut context_a = BTreeMap::new();
    context_a.insert("region".to_string(), "us".to_string());
    let mut choices_a = BTreeMap::new();
    choices_a.insert("cooling_brand".to_string(), "hydra".to_string());
    choices_a.insert("cooling_model".to_string(), "x200".to_string());
    choices_a.insert("pump_type".to_string(), "dual".to_string());

    let mut context_b = BTreeMap::new();
    context_b.insert("region".to_string(), "us".to_string());
    let mut choices_b = BTreeMap::new();
    choices_b.insert("pump_type".to_string(), "dual".to_string());
    choices_b.insert("cooling_model".to_string(), "x200".to_string());
    choices_b.insert("cooling_brand".to_string(), "hydra".to_string());

    let state_a = state_from_payload(&handle, scope, context_a, choices_a)?;
    let state_b = state_from_payload(&handle, scope, context_b, choices_b)?;

    let resolve_a = resolve(&handle, scope, &state_a);
    let resolve_b = resolve(&handle, scope, &state_b);

    assert_eq!(resolve_a.status, OperationStatus::Ok);
    assert_eq!(resolve_b.status, OperationStatus::Ok);
    assert_eq!(resolve_a.resolve_hash, resolve_b.resolve_hash);
    assert_eq!(resolve_a.resolved_output, resolve_b.resolved_output);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn resolve_mutation_invalid_scope_selection_hash_and_missing_context_are_rejected() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-mutations",
    )?;
    let handle = open_handle(&temp_dir)?;

    let invalid_scope = "component:missing_component";
    let invalid_scope_state = state_from_payload(
        &handle,
        invalid_scope,
        s1_default_context(),
        BTreeMap::new(),
    )?;
    let invalid_scope_result = resolve(&handle, invalid_scope, &invalid_scope_state);
    assert_eq!(invalid_scope_result.status, OperationStatus::Error);
    assert_eq!(
        invalid_scope_result.diagnostics.diagnostics[0].code,
        E_RESOLVE_SCOPE_INVALID.to_string()
    );

    let scope = "component:thermal_control";
    let mut tampered_state =
        state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;
    tampered_state.selection_state_hash = "00".repeat(32);
    let hash_mismatch_result = resolve(&handle, scope, &tampered_state);
    assert_eq!(hash_mismatch_result.status, OperationStatus::Error);
    assert_eq!(
        hash_mismatch_result.diagnostics.diagnostics[0].code,
        E_SELECTION_STATE_INVALID.to_string()
    );

    let mut sparse_context = BTreeMap::new();
    sparse_context.insert("region".to_string(), "us".to_string());
    let missing_context_state =
        state_from_payload(&handle, scope, sparse_context, BTreeMap::new())?;
    let missing_context_result = resolve(&handle, scope, &missing_context_state);
    assert_eq!(missing_context_result.status, OperationStatus::Error);
    assert_eq!(
        missing_context_result.diagnostics.diagnostics[0].code,
        E_RESOLVE_CONTEXT_UNSATISFIED.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn resolve_metrics_snapshot() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-metrics",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;

    let resolve_start = Instant::now();
    let result = resolve(&handle, scope, &state);
    let resolve_us = resolve_start.elapsed().as_micros();

    assert_eq!(result.status, OperationStatus::Ok);
    eprintln!(
        "resolve_metrics resolve_us={} rss_kib={}",
        resolve_us,
        read_vm_rss_kib().unwrap_or(0)
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

// --- ADR-0059 D3: `resolved_output_hash` ------------------------------------
//
// The payload identity. `resolve_hash` folds `model_hash` into its pre-image,
// so it rotates on ANY model edit — including one that leaves the bytes a
// service receives byte-identical. `resolved_output_hash` covers
// `{schema_version, scope, resolved_output}` and nothing else, which is what
// makes "did my change touch this deployment" answerable by hash comparison.
//
// The three tests below pin the three things that make it useful: it ignores
// model identity (T1), it does not ignore the payload (T2), and its pre-image
// is frozen (T3). They read the WIRE key rather than the struct field because
// the wire form is what `cfx diff`, the interpreter response, and every
// snapshot consumer actually read.

/// A definition-only chunk that no component references. Merging it changes the
/// compiled model — and therefore `model_hash` — while leaving every resolved
/// value untouched. That is exactly the edit T1 needs: unrelated to the target.
const S1_UNRELATED_DEFS: &str = r#"{
  "package": "s1_water_pump",
  "version": "1.0.0",
  "definitions": {
    "unused_probe_gain": {
      "type": "float",
      "unit": "ratio",
      "doc": "Unreferenced definition; exists only to move model_hash",
      "lifecycle": "runtime",
      "safety": "q_m",
      "access": "technician"
    }
  }
}"#;
const S1_SOURCE_UNRELATED_DEFS: &str =
    "scenarios/s1_water_pump/smoke/chunks/90_unrelated_definitions.toml";

/// Read `resolved_output_hash` off the SERIALIZED result. The struct field is
/// asserted in T3; every other assertion here goes through the wire form,
/// because a consumer that never links the compiler crate is the audience.
fn wire_resolved_output_hash(result: &crate::loader_api::ResolveResult) -> Option<String> {
    serde_json::to_value(result)
        .expect("resolve result must serialize")
        .get("resolved_output_hash")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

#[test]
fn resolved_output_hash_ignores_model_identity() -> Result<()> {
    let scope = "component:thermal_control";

    let (base_dir, _base_index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-output-hash-base",
    )?;
    let base_handle = open_handle(&base_dir)?;
    let base_state =
        state_from_payload(&base_handle, scope, s1_default_context(), BTreeMap::new())?;
    let base = resolve(&base_handle, scope, &base_state);

    let (edited_dir, _edited_index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
            (S1_SOURCE_UNRELATED_DEFS, S1_UNRELATED_DEFS),
        ],
        "s1-output-hash-edited",
    )?;
    let edited_handle = open_handle(&edited_dir)?;
    let edited_state =
        state_from_payload(&edited_handle, scope, s1_default_context(), BTreeMap::new())?;
    let edited = resolve(&edited_handle, scope, &edited_state);

    assert_eq!(base.status, OperationStatus::Ok);
    assert_eq!(edited.status, OperationStatus::Ok);

    // The edit must actually be an edit, or the test proves nothing.
    assert_ne!(
        base.model_hash, edited.model_hash,
        "the unrelated chunk must change model_hash, else T1 is vacuous"
    );
    // ...and it must be unrelated: the delivered bytes are identical.
    assert_eq!(
        base.resolved_output, edited.resolved_output,
        "an unreferenced definition must not change the resolved payload"
    );

    // resolve_hash cannot tell the two apart from a real change: it rotates.
    assert_ne!(
        base.resolve_hash, edited.resolve_hash,
        "resolve_hash folds model_hash, so it must rotate on any model edit"
    );

    // resolved_output_hash can: it is the payload's identity, and the payload
    // did not move.
    let base_output_hash = wire_resolved_output_hash(&base);
    let edited_output_hash = wire_resolved_output_hash(&edited);
    assert!(
        base_output_hash.is_some(),
        "an ok resolve must carry resolved_output_hash"
    );
    assert_eq!(
        base_output_hash, edited_output_hash,
        "resolved_output_hash must ignore model identity"
    );

    std::fs::remove_dir_all(&base_dir).ok();
    std::fs::remove_dir_all(&edited_dir).ok();
    Ok(())
}

#[test]
fn resolved_output_hash_changes_with_output() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-output-hash-differs",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";

    // `pump_type` selects the control_driver override: dual -> the dual driver,
    // single -> the declared default. One value moves; everything else holds.
    let dual = resolve(
        &handle,
        scope,
        &state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?,
    );
    let mut single_context = s1_default_context();
    single_context.insert("pump_type".to_string(), "single".to_string());
    let single = resolve(
        &handle,
        scope,
        &state_from_payload(&handle, scope, single_context, BTreeMap::new())?,
    );

    assert_eq!(dual.status, OperationStatus::Ok);
    assert_eq!(single.status, OperationStatus::Ok);
    assert_ne!(
        dual.resolved_output, single.resolved_output,
        "the two selections must deliver different bytes, else T2 is vacuous"
    );

    let dual_hash = wire_resolved_output_hash(&dual);
    let single_hash = wire_resolved_output_hash(&single);
    assert!(dual_hash.is_some(), "an ok resolve must carry the hash");
    assert_ne!(
        dual_hash, single_hash,
        "resolved_output_hash must change when the delivered payload changes"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

/// The frozen pre-image contract. This literal is NOT copied from the
/// implementation: it is `sha256(serde_json::to_vec({schema_version: 5, scope,
/// resolved_output}))` derived independently from the checked-in S1 smoke
/// golden. If the implementation ever reorders the pre-image fields, folds in
/// `model_hash`, or stops canonicalizing the payload, this value moves and the
/// test says so.
const S1_SMOKE_RESOLVED_OUTPUT_HASH: &str =
    "fadff45010340eebae0ea17ae1c9936bef19c9edff5c5cafff0da7daa150f270";

#[test]
fn resolved_output_hash_preimage_is_pinned() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-output-hash-pinned",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;

    let result = resolve(&handle, scope, &state);
    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(
        wire_resolved_output_hash(&result).as_deref(),
        Some(S1_SMOKE_RESOLVED_OUTPUT_HASH),
        "resolved_output_hash pre-image drifted"
    );
    // The typed field and the wire key are the same value: a consumer that
    // links the crate and one that reads the JSON must agree.
    assert_eq!(
        result.resolved_output_hash.as_deref(),
        Some(S1_SMOKE_RESOLVED_OUTPUT_HASH)
    );

    // Absent on an error result: the field is present iff `resolved_output` is,
    // so a rejected resolve offers no payload identity to compare.
    let rejected = resolve(
        &handle,
        scope,
        &state_from_payload(
            &handle,
            scope,
            BTreeMap::from([("region".to_string(), "us".to_string())]),
            BTreeMap::new(),
        )?,
    );
    assert_eq!(rejected.status, OperationStatus::Error);
    assert!(
        rejected.resolved_output.is_none(),
        "a rejected resolve carries no payload"
    );
    assert_eq!(
        wire_resolved_output_hash(&rejected),
        None,
        "a rejected resolve must not carry resolved_output_hash"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}
