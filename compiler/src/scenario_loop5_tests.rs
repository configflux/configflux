// SPDX-License-Identifier: BUSL-1.1

use crate::ir;
use crate::loader_api::{
    canonical_selection_state, export_resolved, open_model, resolve_from_selection,
    ExportResolvedRequest, ModelHandle, OpenModelRequest, ResolveFromSelectionRequest,
    ResolveResult, SelectionState, EXPORT_PROFILE_CPP_EARLY_BINDING_V1, E_EXPORT_ARTIFACT_INVALID,
    E_EXPORT_PROFILE_INVALID, GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH,
    GENERATED_CONFIG_BUILD_FLAGS_PATH, GENERATED_CONFIG_HPP_PATH,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::Compiler;
use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_GOLDEN_CONFIG_HPP: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/export.config.hpp");
const S1_GOLDEN_CONFIG_BUILD_FLAGS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/export.config_build_flags.cmake");
const S1_GOLDEN_CONFIG_ARTIFACT_MANIFEST: &str =
    include_str!("../scenarios/s1_water_pump/smoke/golden/export.config_artifact_manifest.json");

const S4_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/smoke/chunks/00_definitions.toml";
const S4_SOURCE_COMPONENTS: &str = "scenarios/s4_mobile_robot/smoke/chunks/10_components.toml";
const S4_CHUNK_DEFS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/cue/00_definitions.json");
const S4_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/cue/10_components.json");
const S4_GOLDEN_CONFIG_HPP: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/golden/export.config.hpp");
const S4_GOLDEN_CONFIG_BUILD_FLAGS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/golden/export.config_build_flags.cmake");
const S4_GOLDEN_CONFIG_ARTIFACT_MANIFEST: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/golden/export.config_artifact_manifest.json");

const S5_SOURCE_DEFS: &str = "scenarios/s5_building_hvac/smoke/chunks/00_definitions.toml";
const S5_SOURCE_COMPONENTS: &str = "scenarios/s5_building_hvac/smoke/chunks/10_components.toml";
const S5_CHUNK_DEFS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/00_definitions.json");
const S5_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/cue/10_components.json");
const S5_GOLDEN_CONFIG_HPP: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/golden/export.config.hpp");
const S5_GOLDEN_CONFIG_BUILD_FLAGS: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/golden/export.config_build_flags.cmake");
const S5_GOLDEN_CONFIG_ARTIFACT_MANIFEST: &str =
    include_str!("../scenarios/s5_building_hvac/smoke/golden/export.config_artifact_manifest.json");

fn emitted_cmp_dir(chunks: &[(&str, &str)], label: &str) -> Result<(PathBuf, ir::IrIndex)> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to compute unique timestamp")?
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "configflux-loop5-export-{}-{}-{}",
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

fn state_from_payload(
    model_handle: &ModelHandle,
    scope: &str,
    context_tags: BTreeMap<String, String>,
) -> Result<SelectionState> {
    canonical_selection_state(
        model_handle.model_hash.clone(),
        scope.to_string(),
        context_tags,
        BTreeMap::new(),
    )
}

fn resolve(model_handle: &ModelHandle, scope: &str, state: &SelectionState) -> ResolveResult {
    resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: model_handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
    })
}

fn export(resolve_result: ResolveResult) -> crate::loader_api::ExportResolvedResult {
    export_resolved(ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: EXPORT_PROFILE_CPP_EARLY_BINDING_V1.to_string(),
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

fn generated_contents<'a>(
    result: &'a crate::loader_api::ExportResolvedResult,
    path: &str,
) -> Result<&'a str> {
    let artifacts = result
        .generated_artifacts
        .as_ref()
        .context("missing generated_artifacts")?;
    artifacts
        .files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.contents.as_str())
        .with_context(|| format!("missing generated file '{}'", path))
}

fn with_dynamic_manifest_fields(template: &str, model_hash: &str, resolve_hash: &str) -> String {
    template
        .replace("__DYNAMIC_MODEL_HASH__", model_hash)
        .replace("__DYNAMIC_RESOLVE_HASH__", resolve_hash)
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

fn compile_generated_header(label: &str, header_contents: &str, body: &str) -> Result<()> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to compute unique timestamp")?
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "configflux-loop5-cpp-check-{}-{}-{}",
        label,
        std::process::id(),
        unique
    ));
    let generated_dir = temp_dir.join("generated");
    std::fs::create_dir_all(&generated_dir).with_context(|| {
        format!(
            "Failed to create generated dir '{}'",
            generated_dir.display()
        )
    })?;

    let header_path = generated_dir.join("config.hpp");
    std::fs::write(&header_path, header_contents)
        .with_context(|| format!("Failed to write '{}'", header_path.display()))?;

    let source_path = temp_dir.join("integration_stub.cc");
    std::fs::write(&source_path, body)
        .with_context(|| format!("Failed to write '{}'", source_path.display()))?;

    let object_path = temp_dir.join("integration_stub.o");
    let output = Command::new("c++")
        .arg("-std=c++17")
        .arg("-I")
        .arg(&temp_dir)
        .arg("-c")
        .arg(&source_path)
        .arg("-o")
        .arg(&object_path)
        .output()
        .context("Failed to launch C++ compiler for integration stub")?;

    if !output.status.success() {
        anyhow::bail!(
            "C++ integration compile failed for {}: {}",
            label,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
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
fn loop5_contract_s1_export_result_envelope_has_required_fields() -> Result<()> {
    let (temp_dir, index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-contract",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);

    let result = export(resolve_result.clone());

    assert_eq!(resolve_result.status, OperationStatus::Ok);
    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(result.model_hash, index.config_hash);
    assert_eq!(result.scope, scope);
    assert_eq!(result.resolve_hash, resolve_result.resolve_hash);
    assert_eq!(result.error_count, 0);
    assert_eq!(result.warning_count, 0);
    assert_eq!(result.diagnostics.error_count, 0);
    assert_eq!(result.diagnostics.warning_count, 0);

    let artifacts = result
        .generated_artifacts
        .context("missing generated_artifacts")?;
    assert_eq!(artifacts.profile, EXPORT_PROFILE_CPP_EARLY_BINDING_V1);
    assert!(!artifacts.generator_hash.is_empty());
    assert_eq!(
        artifacts
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            GENERATED_CONFIG_HPP_PATH,
            GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH,
            GENERATED_CONFIG_BUILD_FLAGS_PATH,
        ]
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_golden_s1_generated_outputs_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-golden",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);
    let result = export(resolve_result.clone());

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_HPP_PATH)?.trim_end(),
        S1_GOLDEN_CONFIG_HPP.trim_end()
    );
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_BUILD_FLAGS_PATH)?.trim_end(),
        S1_GOLDEN_CONFIG_BUILD_FLAGS.trim_end()
    );

    let expected_manifest = with_dynamic_manifest_fields(
        S1_GOLDEN_CONFIG_ARTIFACT_MANIFEST,
        &result.model_hash,
        result
            .resolve_hash
            .as_deref()
            .context("missing resolve_hash")?,
    );
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH)?.trim_end(),
        expected_manifest.trim_end()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_golden_s4_generated_outputs_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S4_SOURCE_DEFS, S4_CHUNK_DEFS),
            (S4_SOURCE_COMPONENTS, S4_CHUNK_COMPONENTS),
        ],
        "s4-golden",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:robot_platform";
    let state = state_from_payload(&handle, scope, s4_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);
    let result = export(resolve_result.clone());

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_HPP_PATH)?.trim_end(),
        S4_GOLDEN_CONFIG_HPP.trim_end()
    );
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_BUILD_FLAGS_PATH)?.trim_end(),
        S4_GOLDEN_CONFIG_BUILD_FLAGS.trim_end()
    );

    let expected_manifest = with_dynamic_manifest_fields(
        S4_GOLDEN_CONFIG_ARTIFACT_MANIFEST,
        &result.model_hash,
        result
            .resolve_hash
            .as_deref()
            .context("missing resolve_hash")?,
    );
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH)?.trim_end(),
        expected_manifest.trim_end()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_golden_s5_generated_outputs_match() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S5_SOURCE_DEFS, S5_CHUNK_DEFS),
            (S5_SOURCE_COMPONENTS, S5_CHUNK_COMPONENTS),
        ],
        "s5-golden",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:climate_controller";
    let state = state_from_payload(&handle, scope, s5_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);
    let result = export(resolve_result.clone());

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_HPP_PATH)?.trim_end(),
        S5_GOLDEN_CONFIG_HPP.trim_end()
    );
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_BUILD_FLAGS_PATH)?.trim_end(),
        S5_GOLDEN_CONFIG_BUILD_FLAGS.trim_end()
    );

    let expected_manifest = with_dynamic_manifest_fields(
        S5_GOLDEN_CONFIG_ARTIFACT_MANIFEST,
        &result.model_hash,
        result
            .resolve_hash
            .as_deref()
            .context("missing resolve_hash")?,
    );
    assert_eq!(
        generated_contents(&result, GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH)?.trim_end(),
        expected_manifest.trim_end()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_mutation_invalid_artifact_reference_and_profile_are_rejected() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-mutations",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);

    let invalid_profile = export_resolved(ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_result.clone(),
        profile: "unknown_profile".to_string(),
    });
    assert_eq!(invalid_profile.status, OperationStatus::Error);
    assert_eq!(
        invalid_profile.diagnostics.diagnostics[0].code,
        E_EXPORT_PROFILE_INVALID.to_string()
    );

    let mut tampered = resolve_result;
    let resolved_output = tampered
        .resolved_output
        .as_mut()
        .context("missing resolved_output")?;
    resolved_output["thermal_control"]["components"]["thermal_control"]["params"]
        ["control_driver"]["value"] = JsonValue::Number(123_u64.into());

    let invalid_artifact = export(tampered);
    assert_eq!(invalid_artifact.status, OperationStatus::Error);
    assert_eq!(
        invalid_artifact.diagnostics.diagnostics[0].code,
        E_EXPORT_ARTIFACT_INVALID.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_mutation_illegal_identifier_is_normalized_deterministically() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-symbol-normalization",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);

    let mut tampered_a = resolve_result.clone();
    let root_components = tampered_a
        .resolved_output
        .as_mut()
        .context("missing resolved_output")?
        .get_mut("thermal_control")
        .context("missing scope root")?
        .get_mut("components")
        .context("missing components")?
        .as_object_mut()
        .context("components is not object")?;
    let thermal_component = root_components
        .remove("thermal_control")
        .context("missing thermal_control component")?;
    root_components.insert("thermal-control".to_string(), thermal_component);

    let tampered_b = tampered_a.clone();
    let export_a = export(tampered_a);
    let export_b = export(tampered_b);

    assert_eq!(export_a.status, OperationStatus::Ok);
    assert_eq!(export_b.status, OperationStatus::Ok);

    let generated_a = export_a
        .generated_artifacts
        .context("missing generated_artifacts")?;
    let generated_b = export_b
        .generated_artifacts
        .context("missing generated_artifacts")?;

    assert_eq!(generated_a.generator_hash, generated_b.generator_hash);
    assert_eq!(generated_a.files, generated_b.files);
    assert!(generated_a
        .files
        .iter()
        .find(|file| file.path == GENERATED_CONFIG_HPP_PATH)
        .context("missing config.hpp")?
        .contents
        .contains("kThermalControlControlDriver"));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_lifecycle_leakage_startup_and_runtime_values_are_not_emitted() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-lifecycle-leakage",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let result = export(resolve(&handle, scope, &state));

    assert_eq!(result.status, OperationStatus::Ok);
    let header = generated_contents(&result, GENERATED_CONFIG_HPP_PATH)?;
    let cmake = generated_contents(&result, GENERATED_CONFIG_BUILD_FLAGS_PATH)?;

    assert!(!header.contains("MaxFlowAtCommissioning"));
    assert!(!header.contains("RuntimeTrimGain"));
    assert!(!cmake.contains("MAX_FLOW_AT_STARTUP"));
    assert!(!cmake.contains("RUNTIME_TRIM_GAIN"));

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_determinism_identical_calls_stable_hash_and_artifacts() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-determinism-identical",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);

    let first = export(resolve_result.clone());
    let second = export(resolve_result);

    assert_eq!(first.status, OperationStatus::Ok);
    assert_eq!(first, second);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_determinism_equivalent_key_order_payload_stable_hash_and_artifacts() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-determinism-key-order",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;

    let resolve_a = resolve(&handle, scope, &state);
    let mut resolve_b = resolve_a.clone();
    let reordered = reverse_object_key_order(
        resolve_b
            .resolved_output
            .as_ref()
            .context("missing resolved_output")?,
    );
    resolve_b.resolved_output = Some(reordered);

    let export_a = export(resolve_a);
    let export_b = export(resolve_b);

    assert_eq!(export_a.status, OperationStatus::Ok);
    assert_eq!(export_b.status, OperationStatus::Ok);
    assert_eq!(export_a.generated_artifacts, export_b.generated_artifacts);

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

#[test]
fn loop5_integration_generated_s1_and_s4_headers_compile() -> Result<()> {
    let (s1_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-integration",
    )?;
    let s1_handle = open_handle(&s1_dir)?;
    let s1_state = state_from_payload(
        &s1_handle,
        "component:thermal_control",
        s1_default_context(),
    )?;
    let s1_export = export(resolve(&s1_handle, "component:thermal_control", &s1_state));
    assert_eq!(s1_export.status, OperationStatus::Ok);

    compile_generated_header(
        "s1",
        generated_contents(&s1_export, GENERATED_CONFIG_HPP_PATH)?,
        "#include \"generated/config.hpp\"\nint main() { return configflux::buildcfg::kThermalControlControlDriver[0] == '\\0'; }\n",
    )?;

    let (s4_dir, _index) = emitted_cmp_dir(
        &[
            (S4_SOURCE_DEFS, S4_CHUNK_DEFS),
            (S4_SOURCE_COMPONENTS, S4_CHUNK_COMPONENTS),
        ],
        "s4-integration",
    )?;
    let s4_handle = open_handle(&s4_dir)?;
    let s4_state =
        state_from_payload(&s4_handle, "component:robot_platform", s4_default_context())?;
    let s4_export = export(resolve(&s4_handle, "component:robot_platform", &s4_state));
    assert_eq!(s4_export.status, OperationStatus::Ok);

    compile_generated_header(
        "s4",
        generated_contents(&s4_export, GENERATED_CONFIG_HPP_PATH)?,
        "#include \"generated/config.hpp\"\nint main() {\n  return configflux::buildcfg::kDriveStackDriveDriver[0] + configflux::buildcfg::kPayloadStackPayloadDriver[0];\n}\n",
    )?;

    std::fs::remove_dir_all(&s1_dir).ok();
    std::fs::remove_dir_all(&s4_dir).ok();
    Ok(())
}

#[test]
fn loop5_generator_metrics_snapshot() -> Result<()> {
    let (temp_dir, _index) = emitted_cmp_dir(
        &[
            (S1_SOURCE_DEFS, S1_CHUNK_DEFS),
            (S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS),
        ],
        "s1-metrics",
    )?;
    let handle = open_handle(&temp_dir)?;
    let scope = "component:thermal_control";
    let state = state_from_payload(&handle, scope, s1_default_context())?;
    let resolve_result = resolve(&handle, scope, &state);

    let export_start = Instant::now();
    let export_result = export(resolve_result);
    let export_us = export_start.elapsed().as_micros();

    assert_eq!(export_result.status, OperationStatus::Ok);
    eprintln!(
        "loop5_generator_metrics export_us={} rss_kib={}",
        export_us,
        read_vm_rss_kib().unwrap_or(0)
    );

    std::fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}
