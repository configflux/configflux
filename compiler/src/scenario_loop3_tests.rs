// SPDX-License-Identifier: BUSL-1.1

use crate::ir;
use crate::loader_api::{
    apply_selection, canonical_selection_state, explain_rejection, get_selection_options,
    list_selection_facets, open_model, ApplySelectionRequest, ExplainRejectionRequest,
    GetSelectionOptionsRequest, ModelHandle, OpenModelRequest, SelectionDelta, SelectionState,
    E_SELECTION_CONFLICT, E_SELECTION_INVALID_OPTION, E_SELECTION_UNKNOWN_FACET,
    E_SELECTION_UNSATISFIABLE,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::Compiler;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_GOLDEN_OPTIONS_COOLING_BRAND: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/selection_options.cooling_brand.json");
const S1_GOLDEN_OPTIONS_COOLING_MODEL_AFTER_HYDRA: &str = include_str!(
    "../scenarios/s1_water_pump/smoke/golden/selection_options.cooling_model.after_cooling_brand_hydra.json"
);

const S3_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/smoke/chunks/00_definitions.toml";
const S3_SOURCE_COMPONENTS: &str = "scenarios/s3_automation_cell/smoke/chunks/10_components.toml";
const S3_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/00_definitions.json");
const S3_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/10_components.json");
const S3_GOLDEN_OPTIONS_CONVEYOR_BRAND: &str = include_str!(
    "../scenarios/s3_automation_cell/smoke/golden/selection_options.conveyor_brand.json"
);
const S3_GOLDEN_OPTIONS_VISION_AFTER_SWIFTMOVE: &str = include_str!(
    "../scenarios/s3_automation_cell/smoke/golden/selection_options.vision_stack.after_conveyor_brand_swiftmove.json"
);

#[derive(Debug, Deserialize)]
struct OptionsGolden {
    facet: String,
    valid_options: Vec<String>,
}

fn emitted_cmp_dir(chunks: &[(&str, &str)], label: &str) -> Result<(PathBuf, ir::IrIndex)> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to compute unique timestamp")?
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "configflux-loop3-selection-{}-{}-{}",
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

fn empty_state(model_handle: &ModelHandle, scope: &str) -> Result<SelectionState> {
    canonical_selection_state(
        model_handle.model_hash.clone(),
        scope.to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
}

fn get_options(
    model_handle: &ModelHandle,
    scope: &str,
    state: &SelectionState,
    facet: &str,
    include_pruned_reasons: bool,
) -> crate::loader_api::GetSelectionOptionsResult {
    get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: model_handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
        facet: facet.to_string(),
        include_pruned_reasons,
    })
}

fn apply(
    model_handle: &ModelHandle,
    scope: &str,
    state: &SelectionState,
    facet: &str,
    option: &str,
) -> crate::loader_api::ApplySelectionResult {
    apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: model_handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
        selection_delta: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    })
}

fn load_options_golden(content: &str) -> Result<OptionsGolden> {
    serde_json::from_str(content).context("Failed to parse options golden JSON")
}

fn to_set(values: &[String]) -> BTreeSet<String> {
    values.iter().cloned().collect()
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
fn loop3_contract_s1_get_selection_options_envelope_has_required_fields() -> Result<()> {
    let (temp_dir, index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-contract",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = empty_state(&handle, scope)?;

    let result = get_options(&handle, scope, &state, "cooling_brand", true);
    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(result.model_hash, index.config_hash);
    assert_eq!(result.scope, scope);
    assert_eq!(result.facet, "cooling_brand");
    assert_eq!(result.selection_state_hash, state.selection_state_hash);
    assert_eq!(result.error_count, 0);
    assert_eq!(result.warning_count, 0);
    assert!(result.pruned_options.is_some());
    assert_eq!(result.diagnostics.error_count, 0);
    assert_eq!(result.diagnostics.warning_count, 0);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_list_selection_facets_returns_sorted_s1_facet_universe() -> Result<()> {
    // `list_selection_facets` (configflux-2awb.4 / CFX-3) enumerates exactly the
    // facets `get_selection_options`/`apply_selection` validate against — the S1
    // model's condition facets — in sorted order, so `cfx options` can iterate
    // the per-facet options op deterministically.
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-facets",
    )?;
    let handle = open_handle(&temp_dir)?;

    let facets = list_selection_facets(&handle)?;
    assert_eq!(
        facets,
        vec![
            "cooling_brand".to_string(),
            "cooling_model".to_string(),
            "pump_type".to_string(),
            "region".to_string(),
        ],
        "S1 facet universe must be the sorted condition facets"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_golden_s1_option_sets_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-golden",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = empty_state(&handle, scope)?;

    let expected_brand = load_options_golden(S1_GOLDEN_OPTIONS_COOLING_BRAND)?;
    let brand_options = get_options(&handle, scope, &state, &expected_brand.facet, false);
    assert_eq!(brand_options.status, OperationStatus::Ok);
    assert_eq!(brand_options.valid_options, expected_brand.valid_options);

    let apply_hydra = apply(&handle, scope, &state, "cooling_brand", "hydra");
    assert_eq!(apply_hydra.status, OperationStatus::Ok);
    let after_hydra = apply_hydra
        .selection_state
        .context("missing selection_state")?;

    let expected_model = load_options_golden(S1_GOLDEN_OPTIONS_COOLING_MODEL_AFTER_HYDRA)?;
    let model_options = get_options(&handle, scope, &after_hydra, &expected_model.facet, false);
    assert_eq!(model_options.status, OperationStatus::Ok);
    assert_eq!(model_options.valid_options, expected_model.valid_options);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_golden_s3_option_sets_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
        "s3-golden",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:cell_root";
    let state = empty_state(&handle, scope)?;

    let expected_conveyor = load_options_golden(S3_GOLDEN_OPTIONS_CONVEYOR_BRAND)?;
    let conveyor_options = get_options(&handle, scope, &state, &expected_conveyor.facet, false);
    assert_eq!(conveyor_options.status, OperationStatus::Ok);
    assert_eq!(
        conveyor_options.valid_options,
        expected_conveyor.valid_options
    );

    let apply_swift = apply(&handle, scope, &state, "conveyor_brand", "swiftmove");
    assert_eq!(apply_swift.status, OperationStatus::Ok);
    let after_swift = apply_swift
        .selection_state
        .context("missing selection_state")?;

    let expected_vision = load_options_golden(S3_GOLDEN_OPTIONS_VISION_AFTER_SWIFTMOVE)?;
    let vision_options = get_options(&handle, scope, &after_swift, &expected_vision.facet, false);
    assert_eq!(vision_options.status, OperationStatus::Ok);
    assert_eq!(vision_options.valid_options, expected_vision.valid_options);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_mutation_invalid_facet_option_conflict_and_unsat_are_rejected() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-mutations",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = empty_state(&handle, scope)?;

    let invalid_facet = apply(&handle, scope, &state, "unknown_facet", "x");
    assert_eq!(invalid_facet.status, OperationStatus::Error);
    assert_eq!(
        invalid_facet.diagnostics.diagnostics[0].code,
        E_SELECTION_UNKNOWN_FACET.to_string()
    );

    let invalid_option = apply(&handle, scope, &state, "cooling_brand", "legacy");
    assert_eq!(invalid_option.status, OperationStatus::Error);
    assert_eq!(
        invalid_option.diagnostics.diagnostics[0].code,
        E_SELECTION_INVALID_OPTION.to_string()
    );

    let apply_hydra = apply(&handle, scope, &state, "cooling_brand", "hydra");
    assert_eq!(apply_hydra.status, OperationStatus::Ok);
    let after_hydra = apply_hydra
        .selection_state
        .context("missing selection_state")?;

    let conflict = apply(&handle, scope, &after_hydra, "cooling_brand", "aeroflux");
    assert_eq!(conflict.status, OperationStatus::Error);
    assert_eq!(
        conflict.diagnostics.diagnostics[0].code,
        E_SELECTION_CONFLICT.to_string()
    );

    let unsat = apply(&handle, scope, &after_hydra, "cooling_model", "a9");
    assert_eq!(unsat.status, OperationStatus::Error);
    assert_eq!(
        unsat.diagnostics.diagnostics[0].code,
        E_SELECTION_UNSATISFIABLE.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_explain_rejection_returns_stable_code_and_payload() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-explain",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = empty_state(&handle, scope)?;
    let apply_hydra = apply(&handle, scope, &state, "cooling_brand", "hydra");
    let after_hydra = apply_hydra
        .selection_state
        .context("missing selection_state")?;

    let result = explain_rejection(ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state: after_hydra,
        rejected_option: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "a9".to_string(),
        },
    });
    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(result.rejection.code, E_SELECTION_UNSATISFIABLE.to_string());
    assert_eq!(
        result.diagnostics.diagnostics[0].code,
        E_SELECTION_UNSATISFIABLE
    );
    assert!(result
        .rejection
        .blocking_choices
        .contains_key("cooling_brand"));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_determinism_selection_state_hash_and_option_sets_are_stable() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-determinism",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";

    let mut context_a = BTreeMap::new();
    context_a.insert("cooling_brand".to_string(), "hydra".to_string());
    context_a.insert("cooling_model".to_string(), "x200".to_string());
    let state_a = canonical_selection_state(
        handle.model_hash.clone(),
        scope.to_string(),
        context_a,
        BTreeMap::new(),
    )?;

    let mut context_b = BTreeMap::new();
    context_b.insert("cooling_model".to_string(), "x200".to_string());
    context_b.insert("cooling_brand".to_string(), "hydra".to_string());
    let state_b = canonical_selection_state(
        handle.model_hash.clone(),
        scope.to_string(),
        context_b,
        BTreeMap::new(),
    )?;

    assert_eq!(state_a.selection_state_hash, state_b.selection_state_hash);

    let first = get_options(&handle, scope, &state_a, "pump_type", true);
    let second = get_options(&handle, scope, &state_a, "pump_type", true);
    assert_eq!(first.status, OperationStatus::Ok);
    assert_eq!(first, second);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop3_monotonic_narrowing_on_s1_and_s3_smoke() -> Result<()> {
    let (s1_dir, _s1_index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-monotonic",
    )?;
    let s1_handle = open_handle(&s1_dir)?;
    let s1_scope = "component:thermal_control";
    let s1_state = empty_state(&s1_handle, s1_scope)?;

    let s1_before = get_options(&s1_handle, s1_scope, &s1_state, "cooling_model", false);
    assert_eq!(s1_before.status, OperationStatus::Ok);
    let apply_hydra = apply(&s1_handle, s1_scope, &s1_state, "cooling_brand", "hydra");
    let s1_after_state = apply_hydra
        .selection_state
        .context("missing selection_state")?;
    let s1_after = get_options(
        &s1_handle,
        s1_scope,
        &s1_after_state,
        "cooling_model",
        false,
    );
    assert_eq!(s1_after.status, OperationStatus::Ok);

    let s1_before_set = to_set(&s1_before.valid_options);
    let s1_after_set = to_set(&s1_after.valid_options);
    assert!(s1_after_set.is_subset(&s1_before_set));
    assert!(s1_after_set.len() <= s1_before_set.len());

    let (s3_dir, _s3_index) = emitted_cmp_dir(
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
        "s3-monotonic",
    )?;
    let s3_handle = open_handle(&s3_dir)?;
    let s3_scope = "component:cell_root";
    let s3_state = empty_state(&s3_handle, s3_scope)?;

    let s3_before = get_options(&s3_handle, s3_scope, &s3_state, "vision_stack", false);
    assert_eq!(s3_before.status, OperationStatus::Ok);
    let apply_swift = apply(
        &s3_handle,
        s3_scope,
        &s3_state,
        "conveyor_brand",
        "swiftmove",
    );
    let s3_after_state = apply_swift
        .selection_state
        .context("missing selection_state")?;
    let s3_after = get_options(&s3_handle, s3_scope, &s3_after_state, "vision_stack", false);
    assert_eq!(s3_after.status, OperationStatus::Ok);

    let s3_before_set = to_set(&s3_before.valid_options);
    let s3_after_set = to_set(&s3_after.valid_options);
    assert!(s3_after_set.is_subset(&s3_before_set));
    assert!(s3_after_set.len() <= s3_before_set.len());

    std::fs::remove_dir_all(&s1_dir).ok();
    std::fs::remove_dir_all(&s3_dir).ok();
    Ok(())
}

#[test]
fn loop3_selection_metrics_snapshot() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-metrics",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = empty_state(&handle, scope)?;

    let options_start = Instant::now();
    let options = get_options(&handle, scope, &state, "cooling_brand", true);
    let get_options_us = options_start.elapsed().as_micros();
    assert_eq!(options.status, OperationStatus::Ok);

    let apply_start = Instant::now();
    let applied = apply(&handle, scope, &state, "cooling_brand", "hydra");
    let apply_selection_us = apply_start.elapsed().as_micros();
    assert_eq!(applied.status, OperationStatus::Ok);
    let applied_state = applied.selection_state.context("missing selection_state")?;

    let explain_start = Instant::now();
    let explained = explain_rejection(ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state: applied_state,
        rejected_option: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "a9".to_string(),
        },
    });
    let explain_rejection_us = explain_start.elapsed().as_micros();
    assert_eq!(explained.status, OperationStatus::Error);

    eprintln!(
        "loop3_selection_metrics get_options_us={} apply_selection_us={} explain_rejection_us={} rss_kib={}",
        get_options_us,
        apply_selection_us,
        explain_rejection_us,
        read_vm_rss_kib().unwrap_or(0),
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}
