// SPDX-License-Identifier: BUSL-1.1

use crate::ir;
use crate::loader_api::{
    canonical_selection_state, open_model, resolve_from_selection, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, SelectionState, E_RESOLVE_CONTEXT_UNSATISFIED,
    E_RESOLVE_SCOPE_INVALID, E_SELECTION_STATE_INVALID,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::Compiler;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    resolved_output: JsonValue,
}

fn emitted_cmp_dir(chunks: &[(&str, &str)], label: &str) -> Result<(PathBuf, ir::IrIndex)> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to compute unique timestamp")?
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "configflux-loop4-resolve-{}-{}-{}",
        label,
        std::process::id(),
        unique
    ));
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
    })
}

fn load_resolve_golden(contents: &str) -> Result<ResolveGolden> {
    serde_json::from_str(contents).context("Failed to parse Loop 4 resolve-result golden JSON")
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
fn loop4_contract_s1_resolve_result_envelope_has_required_fields() -> Result<()> {
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
fn loop4_golden_s1_resolve_result_and_hash_match() -> Result<()> {
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
    assert_eq!(result.resolved_output, Some(golden.resolved_output));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop4_golden_s5_resolve_result_and_hash_match() -> Result<()> {
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
    assert_eq!(result.resolved_output, Some(golden.resolved_output));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop4_determinism_identical_calls_stable_hash_and_payload() -> Result<()> {
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
fn loop4_determinism_selection_key_order_does_not_change_resolve_hash() -> Result<()> {
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
fn loop4_mutation_invalid_scope_selection_hash_and_missing_context_are_rejected() -> Result<()> {
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
fn loop4_resolve_metrics_snapshot() -> Result<()> {
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
        "loop4_resolve_metrics resolve_us={} rss_kib={}",
        resolve_us,
        read_vm_rss_kib().unwrap_or(0)
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}
