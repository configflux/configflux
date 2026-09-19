// SPDX-License-Identifier: BUSL-1.1

use crate::ir;
use crate::loader_api::{
    canonical_selection_state, export_software_bom, open_model, resolve_from_selection,
    ExportSoftwareBomRequest, ModelHandle, OpenModelRequest, ResolveFromSelectionRequest,
    ResolveResult, SelectionState, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    EXPORT_SOFTWARE_BOM_PROFILE_VALUE_REDACTED, E_SBOM_ARTIFACT_INVALID, E_SBOM_BINDING_INVALID,
    E_SBOM_PROFILE_INVALID, E_SBOM_STATS_INVALID,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::scenario_test_support::unique_temp_path;
use crate::Compiler;
use anyhow::{Context, Result};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_GOLDEN_SBOM_FULL_AUDIT: &str = include_str!(
    "../scenarios/s1_water_pump/smoke/golden/sbom.full_audit.thermal_control.hydra_x200_dual_us.json"
);

const S2_SOURCE_DEFS: &str = "scenarios/s2_wind_turbine/smoke/chunks/00_definitions.toml";
const S2_SOURCE_COMPONENTS: &str = "scenarios/s2_wind_turbine/smoke/chunks/10_components.toml";
const S2_CHUNK_DEFS: &str =
    include_str!("../scenarios/s2_wind_turbine/smoke/cue/00_definitions.json");
const S2_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s2_wind_turbine/smoke/cue/10_components.json");
const S2_GOLDEN_SBOM_FULL_AUDIT: &str = include_str!(
    "../scenarios/s2_wind_turbine/smoke/golden/sbom.full_audit.turbine_controller.direct_drive_iec_61400.json"
);

const S3_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/smoke/chunks/00_definitions.toml";
const S3_SOURCE_COMPONENTS: &str = "scenarios/s3_automation_cell/smoke/chunks/10_components.toml";
const S3_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/00_definitions.json");
const S3_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/10_components.json");
const S3_GOLDEN_SBOM_FULL_AUDIT: &str = include_str!(
    "../scenarios/s3_automation_cell/smoke/golden/sbom.full_audit.swift_ring_standard.swiftmove_ring_pl_d.json"
);

const S4_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/smoke/chunks/00_definitions.toml";
const S4_SOURCE_COMPONENTS: &str = "scenarios/s4_mobile_robot/smoke/chunks/10_components.toml";
const S4_CHUNK_DEFS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/cue/00_definitions.json");
const S4_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/cue/10_components.json");
const S4_GOLDEN_SBOM_FULL_AUDIT: &str = include_str!(
    "../scenarios/s4_mobile_robot/smoke/golden/sbom.full_audit.robot_platform.mecanum_lidar_heavy_us.json"
);

const S5_SOURCE_DEFS: &str = "scenarios/s5_building_hvac/smoke/chunks/00_definitions.toml";
const S5_SOURCE_COMPONENTS: &str = "scenarios/s5_building_hvac/smoke/chunks/10_components.toml";
const S5_CHUNK_DEFS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/00_definitions.json");
const S5_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/10_components.json");
const S5_GOLDEN_SBOM_FULL_AUDIT: &str = include_str!(
    "../scenarios/s5_building_hvac/smoke/golden/sbom.full_audit.climate_controller.hospital_hepa_us.json"
);

const S1_MEDIUM_SOURCE_DEFS: &str = "scenarios/s1_water_pump/medium/chunks/00_definitions.toml";
const S1_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s1_water_pump/medium/chunks/10_components.toml";
const S1_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/medium/cue/00_definitions.json");
const S1_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/medium/cue/10_components.json");

const S3_MEDIUM_SOURCE_DEFS: &str =
    "scenarios/s3_automation_cell/medium/chunks/00_definitions.toml";
const S3_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s3_automation_cell/medium/chunks/10_components.toml";
const S3_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/medium/cue/00_definitions.json");
const S3_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/medium/cue/10_components.json");

fn emitted_cmp_dir(chunks: &[(&str, &str)], label: &str) -> Result<(PathBuf, ir::IrIndex)> {
    let temp_dir = unique_temp_path("cfx-software-bom", label);
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

fn resolve(model_handle: &ModelHandle, scope: &str, state: &SelectionState) -> ResolveResult {
    resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: model_handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
        implied_choices: Default::default(),
    })
}

fn export(
    resolve_result: ResolveResult,
    profile: &str,
) -> crate::loader_api::ExportSoftwareBomResult {
    export_software_bom(ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: profile.to_string(),
    })
}

fn s1_default_context() -> BTreeMap<String, String> {
    let mut context = BTreeMap::new();
    context.insert("cooling_brand".to_string(), "hydra".to_string());
    context.insert("cooling_model".to_string(), "x200".to_string());
    context.insert("pump_type".to_string(), "dual".to_string());
    context.insert("region".to_string(), "us".to_string());
    context
}

fn s2_default_context() -> BTreeMap<String, String> {
    let mut context = BTreeMap::new();
    context.insert("gearbox_type".to_string(), "direct_drive".to_string());
    context.insert("blade_class".to_string(), "onshore".to_string());
    context.insert("grid_code".to_string(), "iec_61400".to_string());
    context.insert("sensor_pack".to_string(), "core".to_string());
    context
}

fn s3_default_context() -> BTreeMap<String, String> {
    let mut context = BTreeMap::new();
    context.insert("conveyor_brand".to_string(), "swiftmove".to_string());
    context.insert("vision_stack".to_string(), "opticore".to_string());
    context.insert("safety_mode".to_string(), "pl_d".to_string());
    context.insert("network_topology".to_string(), "ring".to_string());
    context
}

fn s4_default_context() -> BTreeMap<String, String> {
    let mut context = BTreeMap::new();
    context.insert("drive_type".to_string(), "mecanum".to_string());
    context.insert("localization_stack".to_string(), "lidar".to_string());
    context.insert("battery_pack".to_string(), "high_density".to_string());
    context.insert("payload_module".to_string(), "heavy_lift".to_string());
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

fn with_dynamic_bom_fields(
    template: &str,
    model_hash: &str,
    resolve_hash: &str,
    selection_state_hash: &str,
    bom_hash: &str,
) -> String {
    template
        .replace("__DYNAMIC_MODEL_HASH__", model_hash)
        .replace("__DYNAMIC_RESOLVE_HASH__", resolve_hash)
        .replace("__DYNAMIC_SELECTION_STATE_HASH__", selection_state_hash)
        .replace("__DYNAMIC_BOM_HASH__", bom_hash)
}

fn reverse_object_key_order(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_by(|a, b| b.0.cmp(a.0));
            let mut reordered = serde_json::Map::new();
            for (key, value) in entries {
                reordered.insert(key.clone(), reverse_object_key_order(value));
            }
            JsonValue::Object(reordered)
        }
        JsonValue::Array(array) => JsonValue::Array(
            array
                .iter()
                .map(reverse_object_key_order)
                .collect::<Vec<_>>(),
        ),
        other => other.clone(),
    }
}

fn parameter_value<'a>(
    bom: &'a crate::loader_api::SoftwareBomV1,
    path: &str,
) -> Result<&'a crate::schema::Value> {
    bom.parameters
        .iter()
        .find(|entry| entry.path == path)
        .map(|entry| &entry.value)
        .with_context(|| format!("missing parameter '{}'", path))
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

fn assert_scenario_golden(
    chunks: &[(&str, &str)],
    scope: &str,
    context: BTreeMap<String, String>,
    golden: &str,
    label: &str,
) -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(chunks, label)?;
    let handle = open_handle(&temp_dir)?;
    let state = state_from_payload(&handle, scope, context, BTreeMap::new())?;
    let resolve_result = resolve(&handle, scope, &state);
    let export_result = export(
        resolve_result.clone(),
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );

    assert_eq!(resolve_result.status, OperationStatus::Ok);
    assert_eq!(export_result.status, OperationStatus::Ok);

    let software_bom = export_result
        .software_bom
        .as_ref()
        .context("missing software_bom")?;
    let expected = with_dynamic_bom_fields(
        golden,
        &export_result.model_hash,
        export_result
            .resolve_hash
            .as_deref()
            .context("missing resolve_hash")?,
        &resolve_result.selection_state_hash,
        export_result
            .bom_hash
            .as_deref()
            .context("missing bom_hash")?,
    );
    let expected_json: JsonValue =
        serde_json::from_str(&expected).context("Failed to parse golden")?;
    let actual_json =
        serde_json::to_value(software_bom).context("Failed to serialize software_bom")?;
    assert_eq!(actual_json, expected_json);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn software_bom_contract_s1_sbom_result_envelope_has_required_fields() -> Result<()> {
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
    let resolve_result = resolve(&handle, scope, &state);

    let result = export(
        resolve_result.clone(),
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );

    assert_eq!(resolve_result.status, OperationStatus::Ok);
    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(result.model_hash, index.config_hash);
    assert_eq!(result.scope, scope);
    assert_eq!(result.resolve_hash, resolve_result.resolve_hash);
    assert!(result.bom_hash.is_some());
    assert!(result.software_bom.is_some());
    assert_eq!(result.error_count, 0);
    assert_eq!(result.warning_count, 0);
    assert_eq!(result.diagnostics.error_count, 0);
    assert_eq!(result.diagnostics.warning_count, 0);

    let software_bom = result.software_bom.context("missing software_bom")?;
    assert_eq!(software_bom.model_hash, index.config_hash);
    assert_eq!(
        software_bom.resolve_hash,
        resolve_result
            .resolve_hash
            .context("missing resolve_hash")?
    );
    assert_eq!(
        software_bom.selection_state_hash,
        Some(resolve_result.selection_state_hash)
    );
    assert_eq!(
        software_bom.bom_hash,
        result.bom_hash.context("missing bom_hash")?
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn software_bom_golden_s1_smoke_software_bom_matches() -> Result<()> {
    assert_scenario_golden(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "component:thermal_control",
        s1_default_context(),
        S1_GOLDEN_SBOM_FULL_AUDIT,
        "s1-golden",
    )
}

#[test]
fn software_bom_golden_s2_smoke_software_bom_matches() -> Result<()> {
    assert_scenario_golden(
        &[
            (S2_SOURCE_DEFS, S2_CHUNK_DEFS),
            (S2_SOURCE_COMPONENTS, S2_CHUNK_COMPONENTS),
        ],
        "component:turbine_controller",
        s2_default_context(),
        S2_GOLDEN_SBOM_FULL_AUDIT,
        "s2-golden",
    )
}

#[test]
fn software_bom_golden_s3_smoke_software_bom_matches() -> Result<()> {
    assert_scenario_golden(
        &[
            (S3_SOURCE_DEFS, S3_CHUNK_DEFS),
            (S3_SOURCE_COMPONENTS, S3_CHUNK_COMPONENTS),
        ],
        "component:swift_ring_standard",
        s3_default_context(),
        S3_GOLDEN_SBOM_FULL_AUDIT,
        "s3-golden",
    )
}

#[test]
fn software_bom_golden_s4_smoke_software_bom_matches() -> Result<()> {
    assert_scenario_golden(
        &[
            (S4_SOURCE_DEFS, S4_CHUNK_DEFS),
            (S4_SOURCE_COMPONENTS, S4_CHUNK_COMPONENTS),
        ],
        "component:robot_platform",
        s4_default_context(),
        S4_GOLDEN_SBOM_FULL_AUDIT,
        "s4-golden",
    )
}

#[test]
fn software_bom_golden_s5_smoke_software_bom_matches() -> Result<()> {
    assert_scenario_golden(
        &[
            (S5_SOURCE_DEFS, S5_CHUNK_DEFS),
            (S5_SOURCE_COMPONENTS, S5_CHUNK_COMPONENTS),
        ],
        "component:climate_controller",
        s5_default_context(),
        S5_GOLDEN_SBOM_FULL_AUDIT,
        "s5-golden",
    )
}

#[test]
fn software_bom_mutation_invalid_profile_artifact_binding_binding_phase_and_stats_are_rejected(
) -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-mutations",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;
    let resolve_result = resolve(&handle, scope, &state);

    let invalid_profile = export(resolve_result.clone(), "unknown_profile");
    assert_eq!(invalid_profile.status, OperationStatus::Error);
    assert_eq!(
        invalid_profile.diagnostics.diagnostics[0].code,
        E_SBOM_PROFILE_INVALID.to_string()
    );

    let mut missing_artifact_catalog = resolve_result.clone();
    missing_artifact_catalog
        .resolved_artifacts
        .remove("hydra_x200_dual_driver");
    let invalid_artifact = export(
        missing_artifact_catalog,
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );
    assert_eq!(invalid_artifact.status, OperationStatus::Error);
    assert_eq!(
        invalid_artifact.diagnostics.diagnostics[0].code,
        E_SBOM_ARTIFACT_INVALID.to_string()
    );

    let mut invalid_binding = resolve_result.clone();
    let resolved_output = invalid_binding
        .resolved_output
        .as_mut()
        .context("missing resolved_output")?;
    resolved_output["thermal_control"]["components"]["thermal_control"]["params"]
        ["runtime_trim_gain"]["lifecycle"] = JsonValue::String("commissioning".to_string());
    let binding_error = export(invalid_binding, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);
    assert_eq!(binding_error.status, OperationStatus::Error);
    assert_eq!(
        binding_error.diagnostics.diagnostics[0].code,
        E_SBOM_BINDING_INVALID.to_string()
    );

    let valid_export = export(resolve_result, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);
    assert_eq!(valid_export.status, OperationStatus::Ok);
    let mut tampered_bom = valid_export.software_bom.context("missing software_bom")?;
    tampered_bom.stats.parameter_count += 1;
    let stats_err = crate::loader_api::validate_software_bom_payload(&tampered_bom)
        .expect_err("expected stats mismatch");
    assert_eq!(stats_err.code, E_SBOM_STATS_INVALID);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn software_bom_determinism_identical_calls_stable_bom_hash_and_payload() -> Result<()> {
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
    let resolve_result = resolve(&handle, scope, &state);

    let first = export(
        resolve_result.clone(),
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );
    let second = export(resolve_result, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);

    assert_eq!(first.status, OperationStatus::Ok);
    assert_eq!(first, second);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn software_bom_determinism_equivalent_key_order_payload_stable_bom_hash_and_payload() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-determinism-key-order",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;

    let resolve_a = resolve(&handle, scope, &state);
    let mut resolve_b = resolve_a.clone();
    let reordered = reverse_object_key_order(
        resolve_b
            .resolved_output
            .as_ref()
            .context("missing resolved_output")?,
    );
    resolve_b.resolved_output = Some(reordered);

    let export_a = export(resolve_a, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);
    let export_b = export(resolve_b, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);

    assert_eq!(export_a.status, OperationStatus::Ok);
    assert_eq!(export_b.status, OperationStatus::Ok);
    assert_eq!(export_a.bom_hash, export_b.bom_hash);
    assert_eq!(export_a.software_bom, export_b.software_bom);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn software_bom_profile_full_audit_preserves_values_and_value_redacted_redacts_selected_values(
) -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-profile",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context(), BTreeMap::new())?;
    let resolve_result = resolve(&handle, scope, &state);

    let full = export(
        resolve_result.clone(),
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );
    let redacted = export(resolve_result, EXPORT_SOFTWARE_BOM_PROFILE_VALUE_REDACTED);

    assert_eq!(full.status, OperationStatus::Ok);
    assert_eq!(redacted.status, OperationStatus::Ok);

    let full_bom = full.software_bom.context("missing full software_bom")?;
    let redacted_bom = redacted
        .software_bom
        .context("missing redacted software_bom")?;

    assert_eq!(full_bom.components, redacted_bom.components);
    assert_eq!(full_bom.artifacts, redacted_bom.artifacts);
    assert_eq!(full_bom.stats, redacted_bom.stats);

    assert_eq!(
        parameter_value(
            &full_bom,
            "component.thermal_control.param.runtime_trim_gain"
        )?,
        &crate::schema::Value::Float(0.15)
    );
    assert_eq!(
        parameter_value(
            &redacted_bom,
            "component.thermal_control.param.runtime_trim_gain"
        )?,
        &crate::schema::Value::String("<redacted>".to_string())
    );

    assert_eq!(
        parameter_value(&full_bom, "component.thermal_control.param.control_driver")?,
        &crate::schema::Value::String("hydra_x200_dual_driver".to_string())
    );
    assert_eq!(
        parameter_value(
            &redacted_bom,
            "component.thermal_control.param.control_driver"
        )?,
        &crate::schema::Value::String("hydra_x200_dual_driver".to_string())
    );

    assert_ne!(full.bom_hash, redacted.bom_hash);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn software_bom_medium_s1_and_s3_bom_validation_paths_pass() -> Result<()> {
    let (s1_dir, _index) = emitted_cmp_dir(
        &[
            (S1_MEDIUM_SOURCE_DEFS, S1_MEDIUM_CHUNK_DEFS),
            (S1_MEDIUM_SOURCE_COMPONENTS, S1_MEDIUM_CHUNK_COMPONENTS),
        ],
        "s1-medium",
    )?;
    let s1_handle = open_handle(&s1_dir)?;
    let s1_scope = "component:thermal_control";
    let s1_state = state_from_payload(&s1_handle, s1_scope, s1_default_context(), BTreeMap::new())?;
    let s1_export = export(
        resolve(&s1_handle, s1_scope, &s1_state),
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );
    assert_eq!(s1_export.status, OperationStatus::Ok);
    let s1_bom = s1_export
        .software_bom
        .context("missing s1 medium software_bom")?;
    assert!(crate::loader_api::validate_software_bom_payload(&s1_bom).is_ok());

    let (s3_dir, _index) = emitted_cmp_dir(
        &[
            (S3_MEDIUM_SOURCE_DEFS, S3_MEDIUM_CHUNK_DEFS),
            (S3_MEDIUM_SOURCE_COMPONENTS, S3_MEDIUM_CHUNK_COMPONENTS),
        ],
        "s3-medium",
    )?;
    let s3_handle = open_handle(&s3_dir)?;
    let s3_scope = "component:swift_ring_standard";
    let s3_state = state_from_payload(&s3_handle, s3_scope, s3_default_context(), BTreeMap::new())?;
    let s3_export = export(
        resolve(&s3_handle, s3_scope, &s3_state),
        EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    );
    assert_eq!(s3_export.status, OperationStatus::Ok);
    let s3_bom = s3_export
        .software_bom
        .context("missing s3 medium software_bom")?;
    assert!(crate::loader_api::validate_software_bom_payload(&s3_bom).is_ok());

    std::fs::remove_dir_all(&s1_dir).ok();
    std::fs::remove_dir_all(&s3_dir).ok();
    Ok(())
}

#[test]
fn software_bom_metrics_snapshot_smoke_and_medium() -> Result<()> {
    let (smoke_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-smoke-metrics",
    )?;
    let smoke_handle = open_handle(&smoke_dir)?;
    let smoke_scope = "component:thermal_control";
    let smoke_state = state_from_payload(
        &smoke_handle,
        smoke_scope,
        s1_default_context(),
        BTreeMap::new(),
    )?;
    let smoke_resolve = resolve(&smoke_handle, smoke_scope, &smoke_state);
    let smoke_export_start = Instant::now();
    let smoke_export = export(smoke_resolve, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);
    let smoke_export_us = smoke_export_start.elapsed().as_micros();
    assert_eq!(smoke_export.status, OperationStatus::Ok);

    let (s1_medium_dir, _index) = emitted_cmp_dir(
        &[
            (S1_MEDIUM_SOURCE_DEFS, S1_MEDIUM_CHUNK_DEFS),
            (S1_MEDIUM_SOURCE_COMPONENTS, S1_MEDIUM_CHUNK_COMPONENTS),
        ],
        "s1-medium-metrics",
    )?;
    let s1_medium_handle = open_handle(&s1_medium_dir)?;
    let s1_medium_state = state_from_payload(
        &s1_medium_handle,
        "component:thermal_control",
        s1_default_context(),
        BTreeMap::new(),
    )?;
    let s1_medium_resolve = resolve(
        &s1_medium_handle,
        "component:thermal_control",
        &s1_medium_state,
    );
    let s1_medium_export_start = Instant::now();
    let s1_medium_export = export(s1_medium_resolve, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);
    let s1_medium_export_us = s1_medium_export_start.elapsed().as_micros();
    assert_eq!(s1_medium_export.status, OperationStatus::Ok);

    let (s3_medium_dir, _index) = emitted_cmp_dir(
        &[
            (S3_MEDIUM_SOURCE_DEFS, S3_MEDIUM_CHUNK_DEFS),
            (S3_MEDIUM_SOURCE_COMPONENTS, S3_MEDIUM_CHUNK_COMPONENTS),
        ],
        "s3-medium-metrics",
    )?;
    let s3_medium_handle = open_handle(&s3_medium_dir)?;
    let s3_medium_state = state_from_payload(
        &s3_medium_handle,
        "component:swift_ring_standard",
        s3_default_context(),
        BTreeMap::new(),
    )?;
    let s3_medium_resolve = resolve(
        &s3_medium_handle,
        "component:swift_ring_standard",
        &s3_medium_state,
    );
    let s3_medium_export_start = Instant::now();
    let s3_medium_export = export(s3_medium_resolve, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT);
    let s3_medium_export_us = s3_medium_export_start.elapsed().as_micros();
    assert_eq!(s3_medium_export.status, OperationStatus::Ok);

    eprintln!(
        "software_bom_metrics smoke_export_us={} medium_s1_export_us={} medium_s3_export_us={} rss_kib={}",
        smoke_export_us,
        s1_medium_export_us,
        s3_medium_export_us,
        read_vm_rss_kib().unwrap_or(0)
    );

    std::fs::remove_dir_all(&smoke_dir).ok();
    std::fs::remove_dir_all(&s1_medium_dir).ok();
    std::fs::remove_dir_all(&s3_medium_dir).ok();
    Ok(())
}

#[test]
fn software_bom_value_redacted_profile_retains_required_structure() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S4_SOURCE_DEFS, S4_CHUNK_DEFS),
            (S4_SOURCE_COMPONENTS, S4_CHUNK_COMPONENTS),
        ],
        "s4-redacted-structure",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:robot_platform";
    let state = state_from_payload(&handle, scope, s4_default_context(), BTreeMap::new())?;
    let resolve_result = resolve(&handle, scope, &state);

    let redacted = export(resolve_result, EXPORT_SOFTWARE_BOM_PROFILE_VALUE_REDACTED);
    assert_eq!(redacted.status, OperationStatus::Ok);

    let bom = redacted.software_bom.context("missing software_bom")?;
    assert_eq!(bom.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(bom.bom_version, 1);
    assert_eq!(bom.hash_algo, "sha256");
    assert_eq!(bom.generator.name, "configflux-sbom");
    assert!(!bom.components.is_empty());
    assert!(!bom.parameters.is_empty());
    assert!(!bom.artifacts.is_empty());

    let payload = serde_json::to_value(&bom).context("Failed to serialize bom")?;
    assert_eq!(payload["schema_version"], json!(PRODUCT_SCHEMA_VERSION));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}
