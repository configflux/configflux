// SPDX-License-Identifier: BUSL-1.1

use crate::loader_api::{
    canonical_selection_state, open_model, resolve_from_selection, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest,
};
use crate::product_api::{OperationStatus, PRODUCT_SCHEMA_VERSION};
use crate::runtime_api::{
    commit_configuration, get_configuration_identity, get_parameter, get_scope_metadata,
    list_parameters, rollback_dirty, runtime_open, set_parameter, AutoResetPolicy,
    AutoResetSchedulerState, CommitConfigurationRequest, GetConfigurationIdentityRequest,
    GetParameterRequest, GetScopeMetadataRequest, ListParametersRequest, RollbackDirtyRequest,
    RollbackMode, RuntimeEventBusState, RuntimeEventKind, RuntimeOpenRequest, RuntimeSnapshot,
    SetParameterRequest, E_RUNTIME_ARTIFACT_UNKNOWN, E_RUNTIME_COMMIT_BASE_MISMATCH,
    E_RUNTIME_HASH_MISMATCH, E_RUNTIME_LIFECYCLE_IMMUTABLE, E_RUNTIME_LIMIT_VIOLATION,
    E_RUNTIME_TYPE_MISMATCH, E_RUNTIME_UNKNOWN_PATH, E_RUNTIME_UNKNOWN_SCOPE,
};
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};
use crate::Compiler;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

const S3_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/smoke/chunks/00_definitions.toml";
const S3_SOURCE_COMPONENTS: &str = "scenarios/s3_automation_cell/smoke/chunks/10_components.toml";
const S3_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/00_definitions.json");
const S3_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/smoke/cue/10_components.json");

#[derive(Clone, Copy)]
struct ScenarioSpec {
    label: &'static str,
    source_defs: &'static str,
    source_components: &'static str,
    chunk_defs: &'static str,
    chunk_components: &'static str,
    scope: &'static str,
    scope_root: &'static str,
    expected_component_count: u32,
    expected_parameter_count: u32,
    expected_artifact_count: u32,
    probe_path: &'static str,
}

const S1_SPEC: ScenarioSpec = ScenarioSpec {
    label: "s1-smoke",
    source_defs: S1_SOURCE_DEFS,
    source_components: S1_SOURCE_COMPONENTS,
    chunk_defs: S1_CHUNK_DEFS,
    chunk_components: S1_CHUNK_COMPONENTS,
    scope: "component:thermal_control",
    scope_root: "thermal_control",
    expected_component_count: 2,
    expected_parameter_count: 3,
    expected_artifact_count: 1,
    probe_path: "component.thermal_control.param.runtime_trim_gain",
};

const S3_SPEC: ScenarioSpec = ScenarioSpec {
    label: "s3-smoke",
    source_defs: S3_SOURCE_DEFS,
    source_components: S3_SOURCE_COMPONENTS,
    chunk_defs: S3_CHUNK_DEFS,
    chunk_components: S3_CHUNK_COMPONENTS,
    scope: "component:swift_ring_standard",
    scope_root: "swift_ring_standard",
    expected_component_count: 2,
    expected_parameter_count: 2,
    expected_artifact_count: 1,
    probe_path: "component.swift_ring_standard.param.profile",
};

fn context_tags_for_spec(spec: &ScenarioSpec) -> BTreeMap<String, String> {
    match spec.label {
        "s1-smoke" => BTreeMap::from([("region".to_string(), "us".to_string())]),
        _ => BTreeMap::new(),
    }
}

fn choices_for_spec(spec: &ScenarioSpec) -> BTreeMap<String, String> {
    match spec.label {
        "s1-smoke" => BTreeMap::from([
            ("cooling_brand".to_string(), "hydra".to_string()),
            ("cooling_model".to_string(), "x200".to_string()),
            ("pump_type".to_string(), "dual".to_string()),
        ]),
        "s3-smoke" => BTreeMap::from([
            ("conveyor_brand".to_string(), "swiftmove".to_string()),
            ("vision_stack".to_string(), "opticore".to_string()),
            ("safety_mode".to_string(), "pl_d".to_string()),
            ("network_topology".to_string(), "ring".to_string()),
        ]),
        _ => BTreeMap::new(),
    }
}

fn emitted_cmp_dir(spec: &ScenarioSpec, label: &str) -> Result<TempDirGuard> {
    let output_dir = unique_temp_dir("configflux-loop10-runtime", label)?;

    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto(spec.source_defs, spec.chunk_defs)
        .with_context(|| format!("Failed to add chunk '{}'", spec.source_defs))?;
    compiler
        .add_chunk_auto(spec.source_components, spec.chunk_components)
        .with_context(|| format!("Failed to add chunk '{}'", spec.source_components))?;
    compiler.emit_ir(&output_dir.path)?;

    Ok(output_dir)
}

fn open_handle(cmp_dir: &Path) -> Result<ModelHandle> {
    let manifest_path = cmp_dir.join(crate::ir::CMP_DEFAULT_MANIFEST_FILENAME);
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });
    if result.status != OperationStatus::Ok {
        bail!("open_model failed: {:?}", result.diagnostics.diagnostics);
    }
    result.model_handle.context("missing model_handle")
}

fn resolve_result_for_spec(spec: &ScenarioSpec, label: &str) -> Result<crate::loader_api::ResolveResult> {
    let cmp_dir = emitted_cmp_dir(spec, label)?;
    let handle = open_handle(&cmp_dir.path)?;
    let state = canonical_selection_state(
        handle.model_hash.clone(),
        spec.scope.to_string(),
        context_tags_for_spec(spec),
        choices_for_spec(spec),
    )?;
    let result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: spec.scope.to_string(),
        selection_state: state,
    });
    if result.status != OperationStatus::Ok {
        bail!(
            "resolve_from_selection failed: {:?}",
            result.diagnostics.diagnostics
        );
    }
    Ok(result)
}

fn runtime_open_request_from_resolve(result: &crate::loader_api::ResolveResult) -> RuntimeOpenRequest {
    RuntimeOpenRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash: result.model_hash.clone(),
        ccm_ref: String::new(),
        resolve_hash: result.resolve_hash.clone().unwrap_or_default(),
        scope: result.scope.clone(),
        resolved_output: result
            .resolved_output
            .clone()
            .unwrap_or(serde_json::Value::Null),
        resolved_component_dependencies: result.resolved_component_dependencies.clone(),
        resolved_artifacts: result.resolved_artifacts.clone(),
        context_tags: result.context_tags.clone(),
        choices: result.choices.clone(),
        committed_overlay: std::collections::BTreeMap::new(),
        dirty_overlay: std::collections::BTreeMap::new(),
        dirty_generations: std::collections::BTreeMap::new(),
        dirty_metadata: std::collections::BTreeMap::new(),
        auto_reset_policy: AutoResetPolicy::default(),
        auto_reset_scheduler: AutoResetSchedulerState::default(),
        event_bus: RuntimeEventBusState::default(),
        sync_status: crate::runtime_api::RuntimeSyncStatus::default(),
        audit_events: Vec::new(),
        audit_next_sequence: 1,
        audit_uploaded_sequence: 0,
        persistence_format_version: 1,
        persistence_journal_sequence: 0,
    }
}

fn runtime_snapshot_for_spec(spec: &ScenarioSpec, label: &str) -> Result<RuntimeSnapshot> {
    let resolved = resolve_result_for_spec(spec, label)?;
    let open_result = runtime_open(runtime_open_request_from_resolve(&resolved));
    if open_result.status != OperationStatus::Ok {
        bail!(
            "runtime_open failed: {:?}",
            open_result.diagnostics.diagnostics
        );
    }
    open_result
        .runtime_snapshot
        .context("missing runtime_snapshot")
}

fn canonical_runtime_path(spec: &ScenarioSpec, path: &str) -> String {
    format!("{}/{}", spec.scope_root, path)
}

#[test]
fn loop10_contract_runtime_open_and_read_envelopes_have_required_fields() -> Result<()> {
    let resolved = resolve_result_for_spec(&S1_SPEC, "s1-contract")?;
    let open_result = runtime_open(runtime_open_request_from_resolve(&resolved));
    assert_eq!(open_result.status, OperationStatus::Ok);
    assert_eq!(open_result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(open_result.error_count, 0);
    assert_eq!(open_result.warning_count, 0);
    assert!(!open_result.model_hash.is_empty());
    assert!(!open_result.resolve_hash.is_empty());

    let snapshot = open_result
        .runtime_snapshot
        .clone()
        .context("missing runtime_snapshot")?;

    let metadata = get_scope_metadata(GetScopeMetadataRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: S1_SPEC.scope_root.to_string(),
    });
    assert_eq!(metadata.status, OperationStatus::Ok);
    assert_eq!(metadata.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(metadata.error_count, 0);
    assert_eq!(metadata.warning_count, 0);
    let scope_stats = metadata.metadata.context("missing metadata payload")?;
    assert_eq!(
        scope_stats.component_count,
        S1_SPEC.expected_component_count
    );
    assert_eq!(
        scope_stats.parameter_count,
        S1_SPEC.expected_parameter_count
    );
    assert_eq!(scope_stats.artifact_count, S1_SPEC.expected_artifact_count);

    let listed = list_parameters(ListParametersRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: S1_SPEC.scope_root.to_string(),
    });
    assert_eq!(listed.status, OperationStatus::Ok);
    assert_eq!(
        listed.parameter_paths.len(),
        S1_SPEC.expected_parameter_count as usize
    );
    assert!(listed
        .parameter_paths
        .windows(2)
        .all(|window| window[0] < window[1]));

    let parameter = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: S1_SPEC.probe_path.to_string(),
    });
    assert_eq!(parameter.status, OperationStatus::Ok);
    assert_eq!(parameter.path, S1_SPEC.probe_path.to_string());
    assert!(parameter.parameter.is_some());

    Ok(())
}

#[test]
fn loop10_smoke_s1_and_s3_runtime_read_paths_validate() -> Result<()> {
    for (idx, spec) in [S1_SPEC, S3_SPEC].iter().enumerate() {
        let snapshot = runtime_snapshot_for_spec(spec, &format!("{}-smoke", idx))?;

        let metadata = get_scope_metadata(GetScopeMetadataRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot.clone(),
            scope_root: spec.scope_root.to_string(),
        });
        assert_eq!(metadata.status, OperationStatus::Ok);
        let stats = metadata.metadata.context("missing metadata payload")?;
        assert_eq!(stats.component_count, spec.expected_component_count);
        assert_eq!(stats.parameter_count, spec.expected_parameter_count);
        assert_eq!(stats.artifact_count, spec.expected_artifact_count);

        let listed = list_parameters(ListParametersRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot.clone(),
            scope_root: spec.scope_root.to_string(),
        });
        assert_eq!(listed.status, OperationStatus::Ok);
        assert_eq!(
            listed.parameter_paths.len(),
            spec.expected_parameter_count as usize
        );
        assert!(listed
            .parameter_paths
            .windows(2)
            .all(|window| window[0] < window[1]));
        assert!(listed
            .parameter_paths
            .contains(&spec.probe_path.to_string()));

        let parameter = get_parameter(GetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: spec.probe_path.to_string(),
        });
        assert_eq!(parameter.status, OperationStatus::Ok);
        assert_eq!(
            parameter
                .parameter
                .context("missing parameter payload")?
                .path,
            spec.probe_path.to_string()
        );
    }
    Ok(())
}

#[test]
fn loop10_mutation_hash_mismatch_and_unknown_scope_path_emit_stable_codes() -> Result<()> {
    let resolved = resolve_result_for_spec(&S1_SPEC, "s1-mutation-hash")?;
    let mut mismatched_request = runtime_open_request_from_resolve(&resolved);
    mismatched_request.resolve_hash = "00".repeat(32);
    let hash_mismatch = runtime_open(mismatched_request);
    assert_eq!(hash_mismatch.status, OperationStatus::Error);
    assert_eq!(
        hash_mismatch.diagnostics.diagnostics[0].code,
        E_RUNTIME_HASH_MISMATCH.to_string()
    );

    let snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-mutation-reads")?;

    let unknown_scope = get_scope_metadata(GetScopeMetadataRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: "missing_scope".to_string(),
    });
    assert_eq!(unknown_scope.status, OperationStatus::Error);
    assert_eq!(
        unknown_scope.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNKNOWN_SCOPE.to_string()
    );

    let unknown_scope_list = list_parameters(ListParametersRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: "missing_scope".to_string(),
    });
    assert_eq!(unknown_scope_list.status, OperationStatus::Error);
    assert_eq!(
        unknown_scope_list.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNKNOWN_SCOPE.to_string()
    );

    let unknown_path = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: "component.thermal_control.param.missing".to_string(),
    });
    assert_eq!(unknown_path.status, OperationStatus::Error);
    assert_eq!(
        unknown_path.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNKNOWN_PATH.to_string()
    );

    let unknown_write_path = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: "component.thermal_control.param.missing".to_string(),
        value: crate::schema::Value::Float(0.2),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(unknown_write_path.status, OperationStatus::Error);
    assert_eq!(
        unknown_write_path.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNKNOWN_PATH.to_string()
    );

    Ok(())
}

#[test]
fn loop10_mutation_runtime_write_policy_rejections_are_enforced() -> Result<()> {
    let base_snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-mutation-writes")?;

    let type_mismatch = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot.clone(),
        path: "component.thermal_control.param.runtime_trim_gain".to_string(),
        value: crate::schema::Value::String("bad".to_string()),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(type_mismatch.status, OperationStatus::Error);
    assert_eq!(
        type_mismatch.diagnostics.diagnostics[0].code,
        E_RUNTIME_TYPE_MISMATCH.to_string()
    );

    let mut bounded_snapshot = base_snapshot.clone();
    {
        let parameter = bounded_snapshot
            .resolved_output
            .get_mut(S1_SPEC.scope_root)
            .and_then(|scope| scope.components.get_mut("thermal_control"))
            .and_then(|component| component.params.get_mut("runtime_trim_gain"))
            .context("missing runtime_trim_gain parameter")?;
        parameter.limits = Some(crate::schema::Limits {
            min: Some(crate::schema::Value::Float(0.0)),
            max: Some(crate::schema::Value::Float(0.2)),
            min_len: None,
            max_len: None,
        });
    }
    let out_of_bounds = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: bounded_snapshot,
        path: "component.thermal_control.param.runtime_trim_gain".to_string(),
        value: crate::schema::Value::Float(0.9),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(out_of_bounds.status, OperationStatus::Error);
    assert_eq!(
        out_of_bounds.diagnostics.diagnostics[0].code,
        E_RUNTIME_LIMIT_VIOLATION.to_string()
    );

    let lifecycle_rejected = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot.clone(),
        path: "component.thermal_control.param.max_flow_at_commissioning".to_string(),
        value: crate::schema::Value::Float(44.0),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(lifecycle_rejected.status, OperationStatus::Error);
    assert_eq!(
        lifecycle_rejected.diagnostics.diagnostics[0].code,
        E_RUNTIME_LIFECYCLE_IMMUTABLE.to_string()
    );

    let mut writable_artifact_snapshot = base_snapshot.clone();
    {
        let parameter = writable_artifact_snapshot
            .resolved_output
            .get_mut(S1_SPEC.scope_root)
            .and_then(|scope| scope.components.get_mut("thermal_control"))
            .and_then(|component| component.params.get_mut("control_driver"))
            .context("missing control_driver parameter")?;
        parameter.lifecycle = crate::schema::Lifecycle::Runtime;
    }
    let unknown_artifact = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: writable_artifact_snapshot,
        path: "component.thermal_control.param.control_driver".to_string(),
        value: crate::schema::Value::String("missing_artifact".to_string()),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(unknown_artifact.status, OperationStatus::Error);
    assert_eq!(
        unknown_artifact.diagnostics.diagnostics[0].code,
        E_RUNTIME_ARTIFACT_UNKNOWN.to_string()
    );

    let successful_write = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot,
        path: "component.thermal_control.param.runtime_trim_gain".to_string(),
        value: crate::schema::Value::Float(0.18),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(successful_write.status, OperationStatus::Ok);
    let updated_snapshot = successful_write
        .runtime_snapshot
        .clone()
        .context("missing updated runtime_snapshot")?;
    let updated = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: updated_snapshot,
        path: "component.thermal_control.param.runtime_trim_gain".to_string(),
    });
    assert_eq!(updated.status, OperationStatus::Ok);
    assert_eq!(
        updated
            .parameter
            .context("missing updated parameter")?
            .value,
        crate::schema::Value::Float(0.18)
    );

    Ok(())
}

#[test]
fn loop10_runtime_layered_overlay_precedence_and_dirty_write_path() -> Result<()> {
    let base_snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-layered-store")?;
    let path = "component.thermal_control.param.runtime_trim_gain";
    let scope_root = S1_SPEC.scope_root.to_string();

    let baseline_value = base_snapshot
        .resolved_output
        .get(&scope_root)
        .and_then(|scope| scope.components.get("thermal_control"))
        .and_then(|component| component.params.get("runtime_trim_gain"))
        .map(|parameter| parameter.value.clone())
        .context("missing baseline runtime_trim_gain")?;

    let mut committed_snapshot = base_snapshot.clone();
    committed_snapshot
        .committed_overlay
        .entry(scope_root.clone())
        .or_default()
        .insert(path.to_string(), crate::schema::Value::Float(0.11));
    let committed_read = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: committed_snapshot.clone(),
        path: path.to_string(),
    });
    assert_eq!(committed_read.status, OperationStatus::Ok);
    assert_eq!(
        committed_read.parameter.context("missing committed read")?.value,
        crate::schema::Value::Float(0.11)
    );

    let mut dirty_snapshot = committed_snapshot;
    dirty_snapshot
        .dirty_overlay
        .entry(scope_root.clone())
        .or_default()
        .insert(path.to_string(), crate::schema::Value::Float(0.22));
    let dirty_read = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_snapshot,
        path: path.to_string(),
    });
    assert_eq!(dirty_read.status, OperationStatus::Ok);
    assert_eq!(
        dirty_read.parameter.context("missing dirty read")?.value,
        crate::schema::Value::Float(0.22)
    );

    let write_result = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot,
        path: path.to_string(),
        value: crate::schema::Value::Float(0.33),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(write_result.status, OperationStatus::Ok);
    let updated_snapshot = write_result
        .runtime_snapshot
        .context("missing updated snapshot after set")?;
    let dirty_value = updated_snapshot
        .dirty_overlay
        .get(&scope_root)
        .and_then(|values| values.get(path))
        .cloned()
        .context("missing dirty overlay value after write")?;
    assert_eq!(dirty_value, crate::schema::Value::Float(0.33));

    let baseline_after_write = updated_snapshot
        .resolved_output
        .get(&scope_root)
        .and_then(|scope| scope.components.get("thermal_control"))
        .and_then(|component| component.params.get("runtime_trim_gain"))
        .map(|parameter| parameter.value.clone())
        .context("missing baseline value after write")?;
    assert_eq!(baseline_after_write, baseline_value);
    assert!(
        updated_snapshot.persistence_journal_sequence > 0,
        "set_parameter should advance persistence journal sequence"
    );

    Ok(())
}

#[test]
fn loop10_configuration_identity_hashes_are_stable_and_type_sensitive() -> Result<()> {
    let base_snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-identity-hash")?;
    let scope_root = S1_SPEC.scope_root.to_string();
    let path_a = "component.thermal_control.param.runtime_trim_gain".to_string();
    let path_b = "component.thermal_control.param.max_flow_at_commissioning".to_string();

    let mut ordered_snapshot = base_snapshot.clone();
    {
        let overlay = ordered_snapshot
            .committed_overlay
            .entry(scope_root.clone())
            .or_default();
        overlay.insert(path_a.clone(), crate::schema::Value::Float(0.15));
        overlay.insert(path_b.clone(), crate::schema::Value::Float(31.0));
    }
    let mut reversed_snapshot = base_snapshot.clone();
    {
        let overlay = reversed_snapshot
            .committed_overlay
            .entry(scope_root.clone())
            .or_default();
        overlay.insert(path_b.clone(), crate::schema::Value::Float(31.0));
        overlay.insert(path_a.clone(), crate::schema::Value::Float(0.15));
    }

    let ordered_identity = get_configuration_identity(GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: ordered_snapshot,
    });
    let reversed_identity = get_configuration_identity(GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: reversed_snapshot,
    });
    assert_eq!(ordered_identity.status, OperationStatus::Ok);
    assert_eq!(reversed_identity.status, OperationStatus::Ok);
    let ordered_identity = ordered_identity.identity.context("missing ordered identity")?;
    let reversed_identity = reversed_identity
        .identity
        .context("missing reversed identity")?;
    assert_eq!(
        ordered_identity.committed_configuration_id,
        reversed_identity.committed_configuration_id
    );
    assert_eq!(ordered_identity.diff_hash, reversed_identity.diff_hash);

    let mut integer_snapshot = base_snapshot.clone();
    integer_snapshot
        .committed_overlay
        .entry(scope_root.clone())
        .or_default()
        .insert(path_a.clone(), crate::schema::Value::Integer(1));
    let mut float_snapshot = base_snapshot;
    float_snapshot
        .committed_overlay
        .entry(scope_root)
        .or_default()
        .insert(path_a, crate::schema::Value::Float(1.0));

    let integer_identity = get_configuration_identity(GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: integer_snapshot,
    });
    let float_identity = get_configuration_identity(GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: float_snapshot,
    });
    assert_eq!(integer_identity.status, OperationStatus::Ok);
    assert_eq!(float_identity.status, OperationStatus::Ok);
    let integer_identity = integer_identity.identity.context("missing integer identity")?;
    let float_identity = float_identity.identity.context("missing float identity")?;
    assert_ne!(
        integer_identity.committed_configuration_id,
        float_identity.committed_configuration_id
    );
    assert_ne!(integer_identity.diff_hash, float_identity.diff_hash);

    Ok(())
}

#[test]
fn loop10_commit_configuration_moves_dirty_into_committed_overlay_and_emits_delta_manifest(
) -> Result<()> {
    let path = "component.thermal_control.param.runtime_trim_gain";
    let base_snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-commit-happy")?;
    let pre_identity = get_configuration_identity(GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot.clone(),
    });
    assert_eq!(pre_identity.status, OperationStatus::Ok);
    let pre_identity = pre_identity.identity.context("missing pre-commit identity")?;

    let write = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot,
        path: path.to_string(),
        value: crate::schema::Value::Float(0.27),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(write.status, OperationStatus::Ok);
    let dirty_snapshot = write.runtime_snapshot.context("missing dirty snapshot")?;

    let commit = commit_configuration(CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_snapshot,
        actor: "loop10.commit".to_string(),
        reason: Some("accept update".to_string()),
        expected_base_configuration_id: Some(pre_identity.committed_configuration_id.clone()),
        changed_paths_hint: vec![canonical_runtime_path(&S1_SPEC, path)],
    });
    assert_eq!(commit.status, OperationStatus::Ok);
    let commit_snapshot = commit.runtime_snapshot.clone().context("missing commit snapshot")?;
    assert!(commit_snapshot
        .dirty_overlay
        .get(S1_SPEC.scope_root)
        .and_then(|entries| entries.get(path))
        .is_none());
    assert_eq!(
        commit_snapshot
            .committed_overlay
            .get(S1_SPEC.scope_root)
            .and_then(|entries| entries.get(path))
            .cloned()
            .context("missing committed overlay value after commit")?,
        crate::schema::Value::Float(0.27)
    );

    assert_eq!(
        commit.base_configuration_id,
        Some(pre_identity.committed_configuration_id.clone())
    );
    let post_identity = get_configuration_identity(GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: commit_snapshot.clone(),
    });
    assert_eq!(post_identity.status, OperationStatus::Ok);
    let post_identity = post_identity.identity.context("missing post identity")?;
    assert_eq!(
        commit.target_configuration_id,
        Some(post_identity.committed_configuration_id.clone())
    );
    assert_eq!(
        post_identity.committed_configuration_id,
        post_identity.working_configuration_id
    );

    assert_eq!(commit.changed_paths.len(), 1);
    let delta = &commit.changed_paths[0];
    assert_eq!(delta.path, canonical_runtime_path(&S1_SPEC, path));
    assert!(delta.before_leaf_hash.is_some());
    assert!(delta.after_leaf_hash.is_some());
    assert_ne!(delta.before_leaf_hash, delta.after_leaf_hash);
    assert!(commit.delta_manifest.is_some());
    assert!(commit_snapshot.event_bus.events.iter().any(|event| {
        event.event_kind == RuntimeEventKind::CommitApplied
    }));
    assert!(commit_snapshot.event_bus.events.iter().any(|event| {
        event.event_kind == RuntimeEventKind::DirtyStateChanged
            && matches!(
                event.payload,
                crate::runtime_api::RuntimeEventPayload::DirtyStateChanged {
                    ref path,
                    dirty: false,
                    ..
                } if path == "component.thermal_control.param.runtime_trim_gain"
            )
    }));
    Ok(())
}

#[test]
fn loop10_commit_configuration_rejects_base_configuration_mismatch() -> Result<()> {
    let path = "component.thermal_control.param.runtime_trim_gain";
    let base_snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-commit-base-mismatch")?;
    let write = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot,
        path: path.to_string(),
        value: crate::schema::Value::Float(0.31),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(write.status, OperationStatus::Ok);

    let commit = commit_configuration(CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: write.runtime_snapshot.context("missing dirty snapshot")?,
        actor: "loop10.commit".to_string(),
        reason: None,
        expected_base_configuration_id: Some("00".repeat(32)),
        changed_paths_hint: Vec::new(),
    });
    assert_eq!(commit.status, OperationStatus::Error);
    assert_eq!(
        commit.diagnostics.diagnostics[0].code,
        E_RUNTIME_COMMIT_BASE_MISMATCH.to_string()
    );
    Ok(())
}

#[test]
fn loop10_rollback_dirty_subset_clears_requested_paths_and_emits_event() -> Result<()> {
    let path = "component.thermal_control.param.runtime_trim_gain";
    let base_snapshot = runtime_snapshot_for_spec(&S1_SPEC, "s1-rollback-subset")?;
    let baseline_value = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot.clone(),
        path: path.to_string(),
    })
    .parameter
    .context("missing baseline parameter")?
    .value;

    let write = set_parameter(SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: base_snapshot,
        path: path.to_string(),
        value: crate::schema::Value::Float(0.41),
        intent: crate::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    });
    assert_eq!(write.status, OperationStatus::Ok);
    let dirty_snapshot = write.runtime_snapshot.context("missing dirty snapshot")?;

    let rollback = rollback_dirty(RollbackDirtyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_snapshot,
        mode: RollbackMode::Subset,
        paths: vec![canonical_runtime_path(&S1_SPEC, path)],
        actor: "loop10.rollback".to_string(),
        reason: Some("operator cancel".to_string()),
    });
    assert_eq!(rollback.status, OperationStatus::Ok);
    assert_eq!(
        rollback.rolled_back_paths,
        vec![canonical_runtime_path(&S1_SPEC, path)]
    );
    assert!(rollback.remaining_dirty_paths.is_empty());
    assert!(rollback.rollback_event_id.is_some());

    let rolled_snapshot = rollback
        .runtime_snapshot
        .clone()
        .context("missing rollback snapshot")?;
    assert!(rolled_snapshot
        .dirty_overlay
        .get(S1_SPEC.scope_root)
        .and_then(|entries| entries.get(path))
        .is_none());

    let readback = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: rolled_snapshot.clone(),
        path: path.to_string(),
    });
    assert_eq!(readback.status, OperationStatus::Ok);
    assert_eq!(
        readback.parameter.context("missing rollback readback")?.value,
        baseline_value
    );
    assert!(rolled_snapshot
        .event_bus
        .events
        .iter()
        .any(|event| event.event_kind == RuntimeEventKind::RollbackApplied));
    Ok(())
}

#[test]
fn loop10_determinism_runtime_read_envelopes_are_byte_stable() -> Result<()> {
    let snapshot = runtime_snapshot_for_spec(&S3_SPEC, "s3-determinism")?;

    let metadata_a = get_scope_metadata(GetScopeMetadataRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: S3_SPEC.scope_root.to_string(),
    });
    let metadata_b = get_scope_metadata(GetScopeMetadataRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: S3_SPEC.scope_root.to_string(),
    });
    assert_eq!(
        serde_json::to_string(&metadata_a).context("Failed to serialize metadata A")?,
        serde_json::to_string(&metadata_b).context("Failed to serialize metadata B")?
    );

    let list_a = list_parameters(ListParametersRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: S3_SPEC.scope_root.to_string(),
    });
    let list_b = list_parameters(ListParametersRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        scope_root: S3_SPEC.scope_root.to_string(),
    });
    assert_eq!(
        serde_json::to_string(&list_a).context("Failed to serialize list A")?,
        serde_json::to_string(&list_b).context("Failed to serialize list B")?
    );

    let parameter_a = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: S3_SPEC.probe_path.to_string(),
    });
    let parameter_b = get_parameter(GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: S3_SPEC.probe_path.to_string(),
    });
    assert_eq!(
        serde_json::to_string(&parameter_a).context("Failed to serialize parameter A")?,
        serde_json::to_string(&parameter_b).context("Failed to serialize parameter B")?
    );

    Ok(())
}
