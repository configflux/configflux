// SPDX-License-Identifier: BUSL-1.1

use crate::product_api::{E_COMPONENT_DEP_CYCLE, E_COMPONENT_DEP_DIAMOND};
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};
use crate::loader_api::{
    apply_selection, canonical_selection_state, export_resolved, export_software_bom,
    get_selection_options, open_model, resolve_from_selection, ApplySelectionRequest,
    ExportResolvedRequest, ExportSoftwareBomRequest, GetSelectionOptionsRequest, ModelHandle,
    OpenModelRequest, ResolveFromSelectionRequest, SelectionDelta, SelectionState,
    EXPORT_PROFILE_CPP_EARLY_BINDING_V1, EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT,
    E_RESOLVE_CONTEXT_UNSATISFIED, E_SELECTION_UNSATISFIABLE,
};
use crate::product_api::{
    compile_model, verify_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    VerifyModelRequest, PRODUCT_SCHEMA_VERSION,
};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

const S1_MEDIUM_SOURCE_DEFS: &str = "scenarios/s1_water_pump/medium/chunks/00_definitions.toml";
const S1_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s1_water_pump/medium/chunks/10_components.toml";
const S1_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../scenarios/s1_water_pump/medium/cue/00_definitions.json");
const S1_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s1_water_pump/medium/cue/10_components.json");

const S2_MEDIUM_SOURCE_DEFS: &str = "scenarios/s2_wind_turbine/medium/chunks/00_definitions.toml";
const S2_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s2_wind_turbine/medium/chunks/10_components.toml";
const S2_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../scenarios/s2_wind_turbine/medium/cue/00_definitions.json");
const S2_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s2_wind_turbine/medium/cue/10_components.json");

const S3_MEDIUM_SOURCE_DEFS: &str =
    "scenarios/s3_automation_cell/medium/chunks/00_definitions.toml";
const S3_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s3_automation_cell/medium/chunks/10_components.toml";
const S3_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/medium/cue/00_definitions.json");
const S3_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/medium/cue/10_components.json");

const S4_SMOKE_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/smoke/chunks/00_definitions.toml";
const S4_SMOKE_SOURCE_COMPONENTS: &str =
    "scenarios/s4_mobile_robot/smoke/chunks/10_components.toml";
const S4_SMOKE_CHUNK_DEFS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/cue/00_definitions.json");
const S4_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s4_mobile_robot/smoke/cue/10_components.json");

const S4_MEDIUM_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/medium/chunks/00_definitions.toml";
const S4_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s4_mobile_robot/medium/chunks/10_components.toml";
const S4_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../scenarios/s4_mobile_robot/medium/cue/00_definitions.json");
const S4_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s4_mobile_robot/medium/cue/10_components.json");

const S3_LARGE_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/large/chunks/00_definitions.toml";
const S3_LARGE_SOURCE_COMPONENTS: &str =
    "scenarios/s3_automation_cell/large/chunks/10_components.toml";
const S3_LARGE_CHUNK_DEFS: &str =
    include_str!("../scenarios/s3_automation_cell/large/cue/00_definitions.json");
const S3_LARGE_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s3_automation_cell/large/cue/10_components.json");

const S4_LARGE_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/large/chunks/00_definitions.toml";
const S4_LARGE_SOURCE_COMPONENTS: &str =
    "scenarios/s4_mobile_robot/large/chunks/10_components.toml";
const S4_LARGE_CHUNK_DEFS: &str =
    include_str!("../scenarios/s4_mobile_robot/large/cue/00_definitions.json");
const S4_LARGE_CHUNK_COMPONENTS: &str =
    include_str!("../scenarios/s4_mobile_robot/large/cue/10_components.json");

const S1_SELECTION_STEPS: &[(&str, &str)] = &[
    ("cooling_brand", "hydra"),
    ("cooling_model", "x200"),
    ("pump_type", "dual"),
];
const S2_SELECTION_STEPS: &[(&str, &str)] =
    &[("gearbox_type", "direct_drive"), ("grid_code", "iec_61400")];
const S3_SELECTION_STEPS: &[(&str, &str)] = &[
    ("conveyor_brand", "swiftmove"),
    ("vision_stack", "opticore"),
    ("safety_mode", "pl_d"),
    ("network_topology", "ring"),
];
const S4_SELECTION_STEPS: &[(&str, &str)] = &[
    ("drive_type", "mecanum"),
    ("localization_stack", "lidar"),
    ("battery_pack", "high_density"),
    ("payload_module", "heavy_lift"),
];

const CYCLE_MUTATION_SOURCE: &str = "scenarios/loop7/mutations/dependency_cycle.toml";
const CYCLE_MUTATION_CHUNK: &str = r#"
package = "loop7_cycle_mutation"
version = "1.0.0"

[components.loop7_cycle_a]
type = "module"
depends_on = ["loop7_cycle_b"]

[components.loop7_cycle_b]
type = "module"
depends_on = ["loop7_cycle_a"]
"#;

const DIAMOND_MUTATION_SOURCE: &str = "scenarios/loop7/mutations/dependency_diamond.toml";
const DIAMOND_MUTATION_CHUNK: &str = r#"
package = "loop7_diamond_mutation"
version = "1.0.0"

[components.loop7_diamond_root]
type = "module"
depends_on = ["loop7_diamond_left", "loop7_diamond_right"]

[components.loop7_diamond_left]
type = "module"
depends_on = ["loop7_diamond_shared"]

[components.loop7_diamond_right]
type = "module"
depends_on = ["loop7_diamond_shared"]

[components.loop7_diamond_shared]
type = "module"
"#;

const MEDIUM_VERIFY_THRESHOLD_US: u128 = 750_000;
const MEDIUM_COMPILE_THRESHOLD_US: u128 = 1_200_000;
const MEDIUM_SELECTION_THRESHOLD_US: u128 = 500_000;
const MEDIUM_RESOLVE_THRESHOLD_US: u128 = 600_000;
const MEDIUM_EXPORT_THRESHOLD_US: u128 = 600_000;
const MEDIUM_BOM_THRESHOLD_US: u128 = 900_000;

const LARGE_VERIFY_THRESHOLD_US: u128 = 2_500_000;
const LARGE_COMPILE_THRESHOLD_US: u128 = 3_500_000;
const LARGE_SELECTION_THRESHOLD_US: u128 = 1_200_000;
const LARGE_RESOLVE_THRESHOLD_US: u128 = 2_000_000;
const LARGE_EXPORT_THRESHOLD_US: u128 = 2_000_000;
const LARGE_BOM_THRESHOLD_US: u128 = 2_500_000;
const RSS_THRESHOLD_KIB: u64 = 524_288;

#[derive(Clone, Copy)]
struct ScenarioSpec {
    name: &'static str,
    source_defs: &'static str,
    source_components: &'static str,
    chunk_defs: &'static str,
    chunk_components: &'static str,
    scope: &'static str,
    selection_steps: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClosedLoopHashes {
    model_hash: String,
    selection_state_hash: String,
    resolve_hash: String,
    generator_hash: String,
    bom_hash: String,
}

#[derive(Debug, Clone, Copy)]
struct StageMetrics {
    verify_us: u128,
    compile_us: u128,
    selection_us: u128,
    resolve_us: u128,
    export_us: u128,
    bom_us: u128,
    rss_kib: u64,
}

#[derive(Debug, Clone)]
struct ClosedLoopResult {
    hashes: ClosedLoopHashes,
    metrics: StageMetrics,
}

const S1_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s1-medium",
    source_defs: S1_MEDIUM_SOURCE_DEFS,
    source_components: S1_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S1_MEDIUM_CHUNK_DEFS,
    chunk_components: S1_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:thermal_control",
    selection_steps: S1_SELECTION_STEPS,
};

const S2_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s2-medium",
    source_defs: S2_MEDIUM_SOURCE_DEFS,
    source_components: S2_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S2_MEDIUM_CHUNK_DEFS,
    chunk_components: S2_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:turbine_controller",
    selection_steps: S2_SELECTION_STEPS,
};

const S3_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s3-medium",
    source_defs: S3_MEDIUM_SOURCE_DEFS,
    source_components: S3_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S3_MEDIUM_CHUNK_DEFS,
    chunk_components: S3_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:swift_ring_standard",
    selection_steps: S3_SELECTION_STEPS,
};

const S4_SMOKE_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s4-smoke",
    source_defs: S4_SMOKE_SOURCE_DEFS,
    source_components: S4_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S4_SMOKE_CHUNK_DEFS,
    chunk_components: S4_SMOKE_CHUNK_COMPONENTS,
    scope: "component:robot_platform",
    selection_steps: S4_SELECTION_STEPS,
};

const S4_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s4-medium",
    source_defs: S4_MEDIUM_SOURCE_DEFS,
    source_components: S4_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S4_MEDIUM_CHUNK_DEFS,
    chunk_components: S4_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:robot_platform",
    selection_steps: S4_SELECTION_STEPS,
};

const S3_LARGE_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s3-large",
    source_defs: S3_LARGE_SOURCE_DEFS,
    source_components: S3_LARGE_SOURCE_COMPONENTS,
    chunk_defs: S3_LARGE_CHUNK_DEFS,
    chunk_components: S3_LARGE_CHUNK_COMPONENTS,
    scope: "component:large_cell_orchestrator",
    selection_steps: S3_SELECTION_STEPS,
};

const S4_LARGE_SPEC: ScenarioSpec = ScenarioSpec {
    name: "s4-large",
    source_defs: S4_LARGE_SOURCE_DEFS,
    source_components: S4_LARGE_SOURCE_COMPONENTS,
    chunk_defs: S4_LARGE_CHUNK_DEFS,
    chunk_components: S4_LARGE_CHUNK_COMPONENTS,
    scope: "component:robot_platform",
    selection_steps: S4_SELECTION_STEPS,
};

fn manifest_from_chunks(chunks: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    chunks
        .iter()
        .map(|(source_id, inline_content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*inline_content).to_string(),
        })
        .collect()
}

fn manifest_for_spec(
    spec: &ScenarioSpec,
    extra_chunks: &[(&str, &str)],
) -> Vec<SourceManifestEntry> {
    let mut chunks = vec![
        (spec.source_defs, spec.chunk_defs),
        (spec.source_components, spec.chunk_components),
    ];
    chunks.extend_from_slice(extra_chunks);
    manifest_from_chunks(&chunks)
}

fn temp_output_dir(label: &str) -> Result<TempDirGuard> {
    unique_temp_dir("configflux-loop7-scale", label)
}

fn open_handle(cmp_manifest: &Path) -> Result<ModelHandle> {
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_manifest.to_string_lossy().into_owned(),
    });
    if result.status != OperationStatus::Ok {
        anyhow::bail!("open_model failed: {:?}", result.diagnostics.diagnostics);
    }
    result.model_handle.context("missing model_handle")
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

fn apply_selection_steps(
    handle: &ModelHandle,
    scope: &str,
    steps: &[(&str, &str)],
) -> Result<(SelectionState, u128)> {
    let selection_start = Instant::now();
    let mut state = canonical_selection_state(
        handle.model_hash.clone(),
        scope.to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )?;

    for (facet, option) in steps {
        let options = get_selection_options(GetSelectionOptionsRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: scope.to_string(),
            selection_state: state.clone(),
            facet: (*facet).to_string(),
            include_pruned_reasons: false,
        });
        assert_eq!(options.status, OperationStatus::Ok);
        assert!(
            options
                .valid_options
                .iter()
                .any(|candidate| candidate == option),
            "Facet '{}' missing option '{}' (valid: {:?})",
            facet,
            option,
            options.valid_options
        );

        let applied = apply_selection(ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: scope.to_string(),
            selection_state: state.clone(),
            selection_delta: SelectionDelta {
                facet: (*facet).to_string(),
                option: (*option).to_string(),
            },
        });
        assert_eq!(applied.status, OperationStatus::Ok);
        state = applied
            .selection_state
            .context("missing selection_state after apply_selection")?;
    }

    Ok((state, selection_start.elapsed().as_micros()))
}

fn run_closed_loop(spec: &ScenarioSpec, label: &str) -> Result<ClosedLoopResult> {
    let manifest = manifest_for_spec(spec, &[]);

    let verify_start = Instant::now();
    let verify_report = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest.clone(),
    });
    let verify_us = verify_start.elapsed().as_micros();
    assert_eq!(verify_report.status, OperationStatus::Ok);
    assert_eq!(verify_report.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(verify_report.error_count, 0);
    assert_eq!(verify_report.warning_count, 0);
    assert_eq!(verify_report.diagnostics.error_count, 0);
    assert_eq!(verify_report.diagnostics.warning_count, 0);

    let output_dir = temp_output_dir(label)?;
    let compile_start = Instant::now();
    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest,
        output_dir: Some(output_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    let compile_us = compile_start.elapsed().as_micros();
    assert_eq!(compile_result.status, OperationStatus::Ok);
    assert_eq!(compile_result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(compile_result.verify_report.status, OperationStatus::Ok);
    assert_eq!(compile_result.verify_report.error_count, 0);
    assert!(compile_result.compiled_model_package_ref.is_some());
    let cmp_manifest_ref = compile_result
        .compiled_model_package_ref
        .as_deref()
        .context("missing compiled_model_package_ref")?;
    let handle = open_handle(Path::new(cmp_manifest_ref))?;
    assert_eq!(handle.model_hash, compile_result.model_hash);

    let (selection_state, selection_us) =
        apply_selection_steps(&handle, spec.scope, spec.selection_steps)?;

    let resolve_start = Instant::now();
    let resolve_result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: spec.scope.to_string(),
        selection_state: selection_state.clone(),
    });
    let resolve_us = resolve_start.elapsed().as_micros();
    assert_eq!(resolve_result.status, OperationStatus::Ok);
    assert_eq!(resolve_result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(resolve_result.model_hash, handle.model_hash);
    assert_eq!(resolve_result.scope, spec.scope);
    assert_eq!(
        resolve_result.selection_state_hash,
        selection_state.selection_state_hash
    );
    assert!(resolve_result.resolve_hash.is_some());
    assert!(resolve_result.resolved_output.is_some());
    assert_eq!(resolve_result.error_count, 0);
    assert_eq!(resolve_result.warning_count, 0);
    assert_eq!(resolve_result.diagnostics.error_count, 0);
    assert_eq!(resolve_result.diagnostics.warning_count, 0);

    let export_start = Instant::now();
    let export_result = export_resolved(ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_result.clone(),
        profile: EXPORT_PROFILE_CPP_EARLY_BINDING_V1.to_string(),
    });
    let export_us = export_start.elapsed().as_micros();
    assert_eq!(export_result.status, OperationStatus::Ok);
    assert_eq!(export_result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(export_result.model_hash, handle.model_hash);
    assert_eq!(export_result.scope, spec.scope);
    assert_eq!(export_result.resolve_hash, resolve_result.resolve_hash);
    assert_eq!(export_result.error_count, 0);
    assert_eq!(export_result.warning_count, 0);
    assert_eq!(export_result.diagnostics.error_count, 0);
    assert_eq!(export_result.diagnostics.warning_count, 0);
    let generated = export_result
        .generated_artifacts
        .as_ref()
        .context("missing generated_artifacts")?;
    assert_eq!(generated.profile, EXPORT_PROFILE_CPP_EARLY_BINDING_V1);
    assert!(!generated.generator_hash.is_empty());

    let bom_start = Instant::now();
    let bom_result = export_software_bom(ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT.to_string(),
    });
    let bom_us = bom_start.elapsed().as_micros();
    assert_eq!(bom_result.status, OperationStatus::Ok);
    assert_eq!(bom_result.schema_version, PRODUCT_SCHEMA_VERSION);
    assert_eq!(bom_result.model_hash, handle.model_hash);
    assert_eq!(bom_result.scope, spec.scope);
    assert!(bom_result.resolve_hash.is_some());
    assert!(bom_result.bom_hash.is_some());
    assert!(bom_result.software_bom.is_some());
    assert_eq!(bom_result.error_count, 0);
    assert_eq!(bom_result.warning_count, 0);
    assert_eq!(bom_result.diagnostics.error_count, 0);
    assert_eq!(bom_result.diagnostics.warning_count, 0);

    Ok(ClosedLoopResult {
        hashes: ClosedLoopHashes {
            model_hash: compile_result.model_hash,
            selection_state_hash: selection_state.selection_state_hash,
            resolve_hash: export_result
                .resolve_hash
                .context("missing resolve_hash in export_result")?,
            generator_hash: generated.generator_hash.clone(),
            bom_hash: bom_result.bom_hash.context("missing bom_hash")?,
        },
        metrics: StageMetrics {
            verify_us,
            compile_us,
            selection_us,
            resolve_us,
            export_us,
            bom_us,
            rss_kib: read_vm_rss_kib().unwrap_or(0),
        },
    })
}

fn assert_hash_stability(label: &str, first: &ClosedLoopResult, second: &ClosedLoopResult) {
    assert_eq!(
        first.hashes, second.hashes,
        "hash tuple mismatch for {label}"
    );
}

// configflux-pz81 precedent (compiler/tests/ccm_emitter_scale.rs): wall-clock
// and RSS budgets are a flaky gate when this non-exclusive target is
// co-scheduled with the parallel suite. The metrics eprintln! snapshots stay
// always-on; the budget *assertions* fire only when opted in via
// CONFIGFLUX_SCALE_WALLTIME_ASSERT (configflux-xowl.2).
fn perf_asserts_enabled() -> bool {
    std::env::var_os("CONFIGFLUX_SCALE_WALLTIME_ASSERT").is_some()
}

fn assert_within_threshold(name: &str, value: u128, threshold: u128) {
    if value > threshold {
        eprintln!("[loop7-perf] {name} exceeded threshold: {value} > {threshold}");
    }
    if perf_asserts_enabled() {
        assert!(
            value <= threshold,
            "{} exceeded threshold: {} > {}",
            name,
            value,
            threshold
        );
    }
}

fn assert_medium_thresholds(label: &str, metrics: &StageMetrics) {
    assert_within_threshold(
        &format!("{label}.verify_us"),
        metrics.verify_us,
        MEDIUM_VERIFY_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.compile_us"),
        metrics.compile_us,
        MEDIUM_COMPILE_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.selection_us"),
        metrics.selection_us,
        MEDIUM_SELECTION_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.resolve_us"),
        metrics.resolve_us,
        MEDIUM_RESOLVE_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.export_us"),
        metrics.export_us,
        MEDIUM_EXPORT_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.bom_us"),
        metrics.bom_us,
        MEDIUM_BOM_THRESHOLD_US,
    );
}

fn assert_large_thresholds(label: &str, metrics: &StageMetrics) {
    assert_within_threshold(
        &format!("{label}.verify_us"),
        metrics.verify_us,
        LARGE_VERIFY_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.compile_us"),
        metrics.compile_us,
        LARGE_COMPILE_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.selection_us"),
        metrics.selection_us,
        LARGE_SELECTION_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.resolve_us"),
        metrics.resolve_us,
        LARGE_RESOLVE_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.export_us"),
        metrics.export_us,
        LARGE_EXPORT_THRESHOLD_US,
    );
    assert_within_threshold(
        &format!("{label}.bom_us"),
        metrics.bom_us,
        LARGE_BOM_THRESHOLD_US,
    );
}

#[test]
fn loop7_medium_closed_loop_s1_s4_are_stable_across_repeated_runs() -> Result<()> {
    let medium_specs = [
        S1_MEDIUM_SPEC,
        S2_MEDIUM_SPEC,
        S3_MEDIUM_SPEC,
        S4_MEDIUM_SPEC,
    ];
    for spec in medium_specs {
        let first = run_closed_loop(&spec, &format!("{}-run-a", spec.name))?;
        let second = run_closed_loop(&spec, &format!("{}-run-b", spec.name))?;
        assert_hash_stability(spec.name, &first, &second);
    }
    Ok(())
}

#[test]
fn loop7_large_closed_loop_s3_s4_are_stable_across_repeated_runs() -> Result<()> {
    let large_specs = [S3_LARGE_SPEC, S4_LARGE_SPEC];
    for spec in large_specs {
        let first = run_closed_loop(&spec, &format!("{}-run-a", spec.name))?;
        let second = run_closed_loop(&spec, &format!("{}-run-b", spec.name))?;
        assert_hash_stability(spec.name, &first, &second);
    }
    Ok(())
}

#[test]
fn loop7_hash_determinism_representative_selection_across_smoke_medium_large() -> Result<()> {
    let representative = [S4_SMOKE_SPEC, S4_MEDIUM_SPEC, S4_LARGE_SPEC];
    for spec in representative {
        let first = run_closed_loop(&spec, &format!("{}-representative-a", spec.name))?;
        let second = run_closed_loop(&spec, &format!("{}-representative-b", spec.name))?;
        assert_hash_stability(spec.name, &first, &second);
    }
    Ok(())
}

#[test]
fn loop7_medium_mutations_cycle_and_unsatisfied_emit_stable_codes() -> Result<()> {
    let cycle_manifest = manifest_for_spec(
        &S2_MEDIUM_SPEC,
        &[(CYCLE_MUTATION_SOURCE, CYCLE_MUTATION_CHUNK)],
    );
    let cycle_verify = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: cycle_manifest.clone(),
    });
    assert_eq!(cycle_verify.status, OperationStatus::Error);
    assert_eq!(
        cycle_verify.diagnostics.diagnostics[0].code,
        E_COMPONENT_DEP_CYCLE.to_string()
    );

    let cycle_compile_dir = temp_output_dir("cycle-mutation-compile")?;
    let cycle_compile = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: cycle_manifest,
        output_dir: Some(cycle_compile_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(cycle_compile.status, OperationStatus::Error);
    assert_eq!(
        cycle_compile.verify_report.diagnostics.diagnostics[0].code,
        E_COMPONENT_DEP_CYCLE.to_string()
    );

    let unsat_manifest = manifest_for_spec(&S1_MEDIUM_SPEC, &[]);
    let unsat_compile_dir = temp_output_dir("unsat-selection-medium")?;
    let unsat_compile = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: unsat_manifest,
        output_dir: Some(unsat_compile_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(unsat_compile.status, OperationStatus::Ok);
    let unsat_cmp_manifest = unsat_compile
        .compiled_model_package_ref
        .as_deref()
        .context("missing compiled_model_package_ref for unsat selection test")?;
    let unsat_handle = open_handle(Path::new(unsat_cmp_manifest))?;
    let scope = S1_MEDIUM_SPEC.scope;
    let empty = canonical_selection_state(
        unsat_handle.model_hash.clone(),
        scope.to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )?;
    let pick_brand = apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: unsat_handle.clone(),
        scope: scope.to_string(),
        selection_state: empty,
        selection_delta: SelectionDelta {
            facet: "cooling_brand".to_string(),
            option: "hydra".to_string(),
        },
    });
    assert_eq!(pick_brand.status, OperationStatus::Ok);
    let conflict = apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: unsat_handle.clone(),
        scope: scope.to_string(),
        selection_state: pick_brand
            .selection_state
            .context("missing state after cooling_brand apply")?,
        selection_delta: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "a9".to_string(),
        },
    });
    assert_eq!(conflict.status, OperationStatus::Error);
    assert_eq!(
        conflict.diagnostics.diagnostics[0].code,
        E_SELECTION_UNSATISFIABLE.to_string()
    );

    Ok(())
}

#[test]
fn loop7_large_mutations_diamond_and_unsatisfied_emit_stable_codes() -> Result<()> {
    let diamond_manifest = manifest_for_spec(
        &S4_LARGE_SPEC,
        &[(DIAMOND_MUTATION_SOURCE, DIAMOND_MUTATION_CHUNK)],
    );
    let diamond_verify = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: diamond_manifest,
    });
    assert_eq!(diamond_verify.status, OperationStatus::Error);
    assert_eq!(
        diamond_verify.diagnostics.diagnostics[0].code,
        E_COMPONENT_DEP_DIAMOND.to_string()
    );

    let large_unsat_manifest = manifest_for_spec(&S3_LARGE_SPEC, &[]);
    let large_unsat_compile_dir = temp_output_dir("unsat-resolve-large")?;
    let large_unsat_compile = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: large_unsat_manifest,
        output_dir: Some(large_unsat_compile_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(large_unsat_compile.status, OperationStatus::Ok);
    let large_unsat_cmp_manifest = large_unsat_compile
        .compiled_model_package_ref
        .as_deref()
        .context("missing compiled_model_package_ref for large resolve unsat test")?;
    let large_handle = open_handle(Path::new(large_unsat_cmp_manifest))?;

    let mut sparse_context = BTreeMap::new();
    sparse_context.insert("conveyor_brand".to_string(), "swiftmove".to_string());
    let sparse_state = canonical_selection_state(
        large_handle.model_hash.clone(),
        S3_LARGE_SPEC.scope.to_string(),
        sparse_context,
        BTreeMap::new(),
    )?;
    let unsat_resolve = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: large_handle,
        scope: S3_LARGE_SPEC.scope.to_string(),
        selection_state: sparse_state,
    });
    assert_eq!(unsat_resolve.status, OperationStatus::Error);
    assert_eq!(
        unsat_resolve.diagnostics.diagnostics[0].code,
        E_RESOLVE_CONTEXT_UNSATISFIED.to_string()
    );

    Ok(())
}

#[test]
fn loop7_medium_performance_thresholds_and_metrics_snapshot() -> Result<()> {
    let medium_s2 = run_closed_loop(&S2_MEDIUM_SPEC, "metrics-s2-medium")?;
    let medium_s4 = run_closed_loop(&S4_MEDIUM_SPEC, "metrics-s4-medium")?;

    assert_medium_thresholds(S2_MEDIUM_SPEC.name, &medium_s2.metrics);
    assert_medium_thresholds(S4_MEDIUM_SPEC.name, &medium_s4.metrics);

    let rss_kib = [medium_s2.metrics.rss_kib, medium_s4.metrics.rss_kib]
        .into_iter()
        .max()
        .unwrap_or(0);
    if rss_kib > RSS_THRESHOLD_KIB {
        eprintln!("[loop7-perf] rss_kib exceeded threshold: {rss_kib} > {RSS_THRESHOLD_KIB}");
    }
    if perf_asserts_enabled() {
        assert!(
            rss_kib <= RSS_THRESHOLD_KIB,
            "rss_kib exceeded threshold: {} > {}",
            rss_kib,
            RSS_THRESHOLD_KIB
        );
    }

    eprintln!(
        "loop7_scale_metrics_medium medium_s2_verify_us={} medium_s2_compile_us={} medium_s2_selection_us={} medium_s2_resolve_us={} medium_s2_export_us={} medium_s2_bom_us={} medium_s4_verify_us={} medium_s4_compile_us={} medium_s4_selection_us={} medium_s4_resolve_us={} medium_s4_export_us={} medium_s4_bom_us={} rss_kib={}",
        medium_s2.metrics.verify_us,
        medium_s2.metrics.compile_us,
        medium_s2.metrics.selection_us,
        medium_s2.metrics.resolve_us,
        medium_s2.metrics.export_us,
        medium_s2.metrics.bom_us,
        medium_s4.metrics.verify_us,
        medium_s4.metrics.compile_us,
        medium_s4.metrics.selection_us,
        medium_s4.metrics.resolve_us,
        medium_s4.metrics.export_us,
        medium_s4.metrics.bom_us,
        rss_kib
    );

    Ok(())
}

#[test]
fn loop7_large_performance_thresholds_and_metrics_snapshot() -> Result<()> {
    let large_s3 = run_closed_loop(&S3_LARGE_SPEC, "metrics-s3-large")?;
    let large_s4 = run_closed_loop(&S4_LARGE_SPEC, "metrics-s4-large")?;

    assert_large_thresholds(S3_LARGE_SPEC.name, &large_s3.metrics);
    assert_large_thresholds(S4_LARGE_SPEC.name, &large_s4.metrics);

    let rss_kib = [large_s3.metrics.rss_kib, large_s4.metrics.rss_kib]
        .into_iter()
        .max()
        .unwrap_or(0);
    if rss_kib > RSS_THRESHOLD_KIB {
        eprintln!("[loop7-perf] rss_kib exceeded threshold: {rss_kib} > {RSS_THRESHOLD_KIB}");
    }
    if perf_asserts_enabled() {
        assert!(
            rss_kib <= RSS_THRESHOLD_KIB,
            "rss_kib exceeded threshold: {} > {}",
            rss_kib,
            RSS_THRESHOLD_KIB
        );
    }

    eprintln!(
        "loop7_scale_metrics_large large_s3_verify_us={} large_s3_compile_us={} large_s3_selection_us={} large_s3_resolve_us={} large_s3_export_us={} large_s3_bom_us={} large_s4_verify_us={} large_s4_compile_us={} large_s4_selection_us={} large_s4_resolve_us={} large_s4_export_us={} large_s4_bom_us={} rss_kib={}",
        large_s3.metrics.verify_us,
        large_s3.metrics.compile_us,
        large_s3.metrics.selection_us,
        large_s3.metrics.resolve_us,
        large_s3.metrics.export_us,
        large_s3.metrics.bom_us,
        large_s4.metrics.verify_us,
        large_s4.metrics.compile_us,
        large_s4.metrics.selection_us,
        large_s4.metrics.resolve_us,
        large_s4.metrics.export_us,
        large_s4.metrics.bom_us,
        rss_kib
    );

    Ok(())
}
