// SPDX-License-Identifier: BUSL-1.1

use super::*;
use compiler::loader_api::{
    apply_selection, canonical_selection_state, open_model, resolve_from_selection,
    ApplySelectionRequest, ConstraintKind, ExplainRejectionRequest, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, SelectionDelta, SelectionState, E_SELECTION_CONFLICT,
    E_SELECTION_ENGINE_DIVERGENCE, E_SELECTION_UNKNOWN_FACET, MODEL_OVER_CONSTRAINED_SUMMARY,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, SourceManifestEntry, PRODUCT_SCHEMA_VERSION,
};
use compiler::runtime_api::{
    AutoResetPolicy, AutoResetSchedulerState, RuntimeEventBusState, RuntimeEventKind,
    RuntimeEventPayload, RuntimeExplainRejectionRequest, RuntimeExplainRejectionResult,
    RuntimeSnapshot, E_RUNTIME_ARTIFACT_UNKNOWN, E_RUNTIME_HASH_MISMATCH,
    E_RUNTIME_LIFECYCLE_IMMUTABLE, E_RUNTIME_LIMIT_VIOLATION, E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE,
    E_RUNTIME_TYPE_MISMATCH, E_RUNTIME_UNKNOWN_PATH, E_RUNTIME_UNKNOWN_SCOPE,
    E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION,
};
use compiler::schema::Value;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const S1_SMOKE_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SMOKE_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_SMOKE_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/smoke/cue/10_components.json");

const S1_MEDIUM_SOURCE_DEFS: &str = "scenarios/s1_water_pump/medium/chunks/00_definitions.toml";
const S1_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s1_water_pump/medium/chunks/10_components.toml";
const S1_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/medium/cue/00_definitions.json");
const S1_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/medium/cue/10_components.json");

const S2_SMOKE_SOURCE_DEFS: &str = "scenarios/s2_wind_turbine/smoke/chunks/00_definitions.toml";
const S2_SMOKE_SOURCE_COMPONENTS: &str =
    "scenarios/s2_wind_turbine/smoke/chunks/10_components.toml";
const S2_SMOKE_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s2_wind_turbine/smoke/cue/00_definitions.json");
const S2_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s2_wind_turbine/smoke/cue/10_components.json");

const S2_MEDIUM_SOURCE_DEFS: &str = "scenarios/s2_wind_turbine/medium/chunks/00_definitions.toml";
const S2_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s2_wind_turbine/medium/chunks/10_components.toml";
const S2_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s2_wind_turbine/medium/cue/00_definitions.json");
const S2_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s2_wind_turbine/medium/cue/10_components.json");

const S3_SMOKE_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/smoke/chunks/00_definitions.toml";
const S3_SMOKE_SOURCE_COMPONENTS: &str =
    "scenarios/s3_automation_cell/smoke/chunks/10_components.toml";
const S3_SMOKE_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s3_automation_cell/smoke/cue/00_definitions.json");
const S3_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s3_automation_cell/smoke/cue/10_components.json");

const S3_MEDIUM_SOURCE_DEFS: &str =
    "scenarios/s3_automation_cell/medium/chunks/00_definitions.toml";
const S3_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s3_automation_cell/medium/chunks/10_components.toml";
const S3_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s3_automation_cell/medium/cue/00_definitions.json");
const S3_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s3_automation_cell/medium/cue/10_components.json");

const S4_SMOKE_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/smoke/chunks/00_definitions.toml";
const S4_SMOKE_SOURCE_COMPONENTS: &str =
    "scenarios/s4_mobile_robot/smoke/chunks/10_components.toml";
const S4_SMOKE_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s4_mobile_robot/smoke/cue/00_definitions.json");
const S4_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s4_mobile_robot/smoke/cue/10_components.json");

const S4_MEDIUM_SOURCE_DEFS: &str = "scenarios/s4_mobile_robot/medium/chunks/00_definitions.toml";
const S4_MEDIUM_SOURCE_COMPONENTS: &str =
    "scenarios/s4_mobile_robot/medium/chunks/10_components.toml";
const S4_MEDIUM_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s4_mobile_robot/medium/cue/00_definitions.json");
const S4_MEDIUM_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s4_mobile_robot/medium/cue/10_components.json");

const S5_SMOKE_SOURCE_DEFS: &str = "scenarios/s5_building_hvac/smoke/chunks/00_definitions.toml";
const S5_SMOKE_SOURCE_COMPONENTS: &str =
    "scenarios/s5_building_hvac/smoke/chunks/10_components.toml";
const S5_SMOKE_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s5_building_hvac/smoke/cue/00_definitions.json");
const S5_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s5_building_hvac/smoke/cue/10_components.json");

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
const S5_SELECTION_STEPS: &[(&str, &str)] = &[
    ("occupancy_class", "hospital"),
    ("filtration_grade", "hepa"),
    ("region", "us"),
];

#[derive(Clone, Copy)]
struct ScenarioSpec {
    id: &'static str,
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
    mutable_path: Option<&'static str>,
    immutable_path: &'static str,
    artifact_path: &'static str,
    selection_steps: &'static [(&'static str, &'static str)],
}

const S1_SMOKE_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s1-smoke",
    source_defs: S1_SMOKE_SOURCE_DEFS,
    source_components: S1_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S1_SMOKE_CHUNK_DEFS,
    chunk_components: S1_SMOKE_CHUNK_COMPONENTS,
    scope: "component:thermal_control",
    scope_root: "thermal_control",
    expected_component_count: 2,
    expected_parameter_count: 3,
    expected_artifact_count: 1,
    probe_path: "component.thermal_control.param.runtime_trim_gain",
    mutable_path: Some("component.thermal_control.param.runtime_trim_gain"),
    immutable_path: "component.thermal_control.param.max_flow_at_commissioning",
    artifact_path: "component.thermal_control.param.control_driver",
    selection_steps: S1_SELECTION_STEPS,
};

const S1_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s1-medium",
    source_defs: S1_MEDIUM_SOURCE_DEFS,
    source_components: S1_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S1_MEDIUM_CHUNK_DEFS,
    chunk_components: S1_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:thermal_control",
    scope_root: "thermal_control",
    expected_component_count: 2,
    expected_parameter_count: 3,
    expected_artifact_count: 1,
    probe_path: "component.thermal_control.param.runtime_trim_gain",
    mutable_path: Some("component.thermal_control.param.runtime_trim_gain"),
    immutable_path: "component.thermal_control.param.max_flow_at_commissioning",
    artifact_path: "component.thermal_control.param.control_driver",
    selection_steps: S1_SELECTION_STEPS,
};

const S2_SMOKE_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s2-smoke",
    source_defs: S2_SMOKE_SOURCE_DEFS,
    source_components: S2_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S2_SMOKE_CHUNK_DEFS,
    chunk_components: S2_SMOKE_CHUNK_COMPONENTS,
    scope: "component:turbine_controller",
    scope_root: "turbine_controller",
    expected_component_count: 3,
    expected_parameter_count: 3,
    expected_artifact_count: 1,
    probe_path: "component.turbine_controller.param.pitch_trim_gain",
    mutable_path: Some("component.turbine_controller.param.pitch_trim_gain"),
    immutable_path: "component.turbine_controller.param.grid_profile",
    artifact_path: "component.turbine_controller.param.control_driver",
    selection_steps: S2_SELECTION_STEPS,
};

const S2_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s2-medium",
    source_defs: S2_MEDIUM_SOURCE_DEFS,
    source_components: S2_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S2_MEDIUM_CHUNK_DEFS,
    chunk_components: S2_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:turbine_controller",
    scope_root: "turbine_controller",
    expected_component_count: 3,
    expected_parameter_count: 3,
    expected_artifact_count: 1,
    probe_path: "component.turbine_controller.param.pitch_trim_gain",
    mutable_path: Some("component.turbine_controller.param.pitch_trim_gain"),
    immutable_path: "component.turbine_controller.param.grid_profile",
    artifact_path: "component.turbine_controller.param.control_driver",
    selection_steps: S2_SELECTION_STEPS,
};

const S3_SMOKE_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s3-smoke",
    source_defs: S3_SMOKE_SOURCE_DEFS,
    source_components: S3_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S3_SMOKE_CHUNK_DEFS,
    chunk_components: S3_SMOKE_CHUNK_COMPONENTS,
    scope: "component:swift_ring_standard",
    scope_root: "swift_ring_standard",
    expected_component_count: 2,
    expected_parameter_count: 2,
    expected_artifact_count: 1,
    probe_path: "component.swift_ring_standard.param.profile",
    mutable_path: None,
    immutable_path: "component.swift_ring_standard.param.profile",
    artifact_path: "component.swift_ring_standard.param.driver",
    selection_steps: S3_SELECTION_STEPS,
};

const S3_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s3-medium",
    source_defs: S3_MEDIUM_SOURCE_DEFS,
    source_components: S3_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S3_MEDIUM_CHUNK_DEFS,
    chunk_components: S3_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:swift_ring_standard",
    scope_root: "swift_ring_standard",
    expected_component_count: 2,
    expected_parameter_count: 2,
    expected_artifact_count: 1,
    probe_path: "component.swift_ring_standard.param.profile",
    mutable_path: None,
    immutable_path: "component.swift_ring_standard.param.profile",
    artifact_path: "component.swift_ring_standard.param.driver",
    selection_steps: S3_SELECTION_STEPS,
};

const S4_SMOKE_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s4-smoke",
    source_defs: S4_SMOKE_SOURCE_DEFS,
    source_components: S4_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S4_SMOKE_CHUNK_DEFS,
    chunk_components: S4_SMOKE_CHUNK_COMPONENTS,
    scope: "component:robot_platform",
    scope_root: "robot_platform",
    expected_component_count: 5,
    expected_parameter_count: 7,
    expected_artifact_count: 3,
    probe_path: "component.drive_stack.param.runtime_trim_gain",
    mutable_path: Some("component.drive_stack.param.runtime_trim_gain"),
    immutable_path: "component.drive_stack.param.max_speed_at_startup",
    artifact_path: "component.drive_stack.param.drive_driver",
    selection_steps: S4_SELECTION_STEPS,
};

const S4_MEDIUM_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s4-medium",
    source_defs: S4_MEDIUM_SOURCE_DEFS,
    source_components: S4_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S4_MEDIUM_CHUNK_DEFS,
    chunk_components: S4_MEDIUM_CHUNK_COMPONENTS,
    scope: "component:robot_platform",
    scope_root: "robot_platform",
    expected_component_count: 5,
    expected_parameter_count: 7,
    expected_artifact_count: 3,
    probe_path: "component.drive_stack.param.runtime_trim_gain",
    mutable_path: Some("component.drive_stack.param.runtime_trim_gain"),
    immutable_path: "component.drive_stack.param.max_speed_at_startup",
    artifact_path: "component.drive_stack.param.drive_driver",
    selection_steps: S4_SELECTION_STEPS,
};

const S5_SMOKE_SPEC: ScenarioSpec = ScenarioSpec {
    id: "s5-smoke",
    source_defs: S5_SMOKE_SOURCE_DEFS,
    source_components: S5_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S5_SMOKE_CHUNK_DEFS,
    chunk_components: S5_SMOKE_CHUNK_COMPONENTS,
    scope: "component:climate_controller",
    scope_root: "climate_controller",
    expected_component_count: 4,
    expected_parameter_count: 3,
    expected_artifact_count: 1,
    probe_path: "component.climate_controller.param.airflow_trim_gain",
    mutable_path: Some("component.climate_controller.param.airflow_trim_gain"),
    immutable_path: "component.climate_controller.param.ventilation_profile",
    artifact_path: "component.climate_controller.param.controller_package",
    selection_steps: S5_SELECTION_STEPS,
};

const ALL_SPECS: [ScenarioSpec; 9] = [
    S1_SMOKE_SPEC,
    S1_MEDIUM_SPEC,
    S2_SMOKE_SPEC,
    S2_MEDIUM_SPEC,
    S3_SMOKE_SPEC,
    S3_MEDIUM_SPEC,
    S4_SMOKE_SPEC,
    S4_MEDIUM_SPEC,
    S5_SMOKE_SPEC,
];

static RESOLVED_FIXTURES: OnceLock<BTreeMap<&'static str, compiler::loader_api::ResolveResult>> =
    OnceLock::new();

/// Process-wide registry mapping a fixture's `model_hash` to the on-disk path
/// of its emitted `.ccm` sibling. ADR-0030 D2 makes a usable `.ccm` a hard
/// precondition for `runtime-open`, so every fixture that opens a snapshot must
/// carry one. The compiled CMP+`.ccm` directory is leaked (never removed) for
/// the lifetime of the test process so the `.ccm` outlives fixture
/// construction; `runtime_open_request_from_resolve` looks the path up by the
/// resolve result's `model_hash`.
static FIXTURE_CCM_REFS: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();

/// Record the `.ccm` path for a fixture keyed by its `model_hash`. Poison-
/// resilient: a panic in another test must not wedge the shared registry.
fn register_fixture_ccm_ref(model_hash: &str, ccm_ref: &str) {
    let registry = FIXTURE_CCM_REFS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(model_hash.to_string(), ccm_ref.to_string());
}

/// Look up the registered `.ccm` path for a fixture `model_hash`, or `""` if
/// none was registered. An empty result is intentional, not an error: tests
/// that build a bespoke fixture (e.g. the solver-validation fixture) register
/// nothing here and override `ccm_ref` on the open request themselves. The
/// cached scenario fixtures always register, so cached-fixture opens get a real
/// path. Poison-resilient for the same reason as `register_fixture_ccm_ref`.
fn fixture_ccm_ref(model_hash: &str) -> String {
    let Some(registry) = FIXTURE_CCM_REFS.get() else {
        return String::new();
    };
    let guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    guard.get(model_hash).cloned().unwrap_or_default()
}

struct TempDirGuard {
    path: PathBuf,
    manifest_ref: String,
}

impl TempDirGuard {
    /// Leak this directory: skip the `Drop` cleanup so the emitted CMP and its
    /// `.ccm` sibling persist for the lifetime of the test process. Used for
    /// fixtures whose `.ccm` must remain loadable by later `runtime-open`
    /// calls (ADR-0030 D2). Returns the directory path.
    fn persist(self) -> PathBuf {
        let path = self.path.clone();
        std::mem::forget(self);
        path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// Per-process monotonic counter for fixture dir names — unique across threads
/// regardless of clock resolution (configflux-6gzn).
static FIXTURE_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build and create a guaranteed-unique temp dir for a compiled fixture.
///
/// Determinism (configflux-6gzn): many `#[test]` threads compile the
/// byte-identical solver fixture (same emitted file names). The old name
/// `solver-<pid>-<nanos>` collided when `<nanos>` repeated under a coarse clock
/// on a loaded gate host — two builds truncated each other's files and a reader
/// saw a half-written index/chunk, so `open_model` returned `status=Error` (the
/// flaky `open_handle` panic in `run_037`/`run_038`). The atomic `seq` makes the
/// path collision-proof by construction; `nanos` is kept only for triage.
fn unique_fixture_dir(label: &str) -> PathBuf {
    let seq = FIXTURE_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("duration")
        .as_nanos();
    let path = std::env::temp_dir().join(fixture_dir_name(label, seq, &thread_token(), nanos));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Stable, filesystem-safe, unique-per-live-thread token.
fn thread_token() -> String {
    format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Pure fixture-dir-name construction. Factored out so the collision-resistance
/// invariant (the monotonic `seq` alone keeps names distinct) can be asserted
/// deterministically, without depending on wall-clock resolution (configflux-6gzn).
fn fixture_dir_name(label: &str, seq: u64, thread_token: &str, nanos: u128) -> String {
    format!(
        "configflux-runtime-{label}-{}-{thread_token}-{seq}-{nanos}",
        std::process::id()
    )
}

struct RunOutput {
    exit_code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

fn run_cli(args: &[&str], stdin_payload: &[u8]) -> RunOutput {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("configflux-runtime");
    argv.extend_from_slice(args);

    let mut stdin = Cursor::new(stdin_payload.to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run(argv, &mut stdin, &mut stdout, &mut stderr);

    RunOutput {
        exit_code,
        stdout,
        stderr: String::from_utf8(stderr).expect("stderr utf8"),
    }
}

fn run_json_command<Req: Serialize, Res: DeserializeOwned>(
    args: &[&str],
    request: &Req,
) -> (RunOutput, Res) {
    let payload = serde_json::to_vec(request).expect("serialize request");
    let output = run_cli(args, &payload);
    let response = serde_json::from_slice::<Res>(&output.stdout).expect("parse response");
    (output, response)
}

fn all_specs() -> &'static [ScenarioSpec] {
    &ALL_SPECS
}

fn mutable_specs() -> Vec<&'static ScenarioSpec> {
    all_specs()
        .iter()
        .filter(|spec| spec.mutable_path.is_some())
        .collect()
}

fn resolved_fixture(spec: &'static ScenarioSpec) -> compiler::loader_api::ResolveResult {
    let fixtures = RESOLVED_FIXTURES.get_or_init(build_resolved_fixtures);
    fixtures.get(spec.id).expect("resolved fixture").clone()
}

fn build_resolved_fixtures() -> BTreeMap<&'static str, compiler::loader_api::ResolveResult> {
    let mut fixtures = BTreeMap::new();
    for spec in all_specs() {
        fixtures.insert(
            spec.id,
            resolve_result_for_spec(spec, &format!("fixture-{}", spec.id)),
        );
    }
    fixtures
}

fn context_tags_for_spec(spec: &ScenarioSpec) -> BTreeMap<String, String> {
    if spec.id.starts_with("s1-") {
        BTreeMap::from([("region".to_string(), "us".to_string())])
    } else {
        BTreeMap::new()
    }
}

fn emitted_cmp_dir(spec: &ScenarioSpec, label: &str) -> TempDirGuard {
    let path = unique_fixture_dir(label);

    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![
            SourceManifestEntry {
                source_id: spec.source_defs.to_string(),
                inline_content: spec.chunk_defs.to_string(),
            },
            SourceManifestEntry {
                source_id: spec.source_components.to_string(),
                inline_content: spec.chunk_components.to_string(),
            },
        ],
        output_dir: Some(path_display(&path)),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        compile_result.status,
        OperationStatus::Ok,
        "fixture compilation failed: {:?}",
        compile_result.verify_report.diagnostics.diagnostics
    );

    TempDirGuard {
        path,
        manifest_ref: compile_result
            .compiled_model_package_ref
            .expect("compiled_model_package_ref"),
    }
}

fn open_handle(cmp_dir: &TempDirGuard) -> ModelHandle {
    let request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_dir.manifest_ref.clone(),
    };
    let result = open_model(request);
    // configflux-6gzn: surface the loader diagnostics on failure so a future
    // open Error is self-describing instead of a bare `left: Error` panic.
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "open_model failed for cmp_manifest_ref {:?}: {:?}",
        cmp_dir.manifest_ref,
        result.diagnostics.diagnostics
    );
    result.model_handle.expect("model_handle")
}

fn apply_selection_steps(handle: &ModelHandle, spec: &ScenarioSpec) -> SelectionState {
    let mut state = canonical_selection_state(
        handle.model_hash.clone(),
        spec.scope.to_string(),
        context_tags_for_spec(spec),
        BTreeMap::new(),
    )
    .expect("selection state");

    for (facet, option) in spec.selection_steps {
        let request = ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: spec.scope.to_string(),
            selection_state: state,
            selection_delta: SelectionDelta {
                facet: (*facet).to_string(),
                option: (*option).to_string(),
            },
        };
        let result = apply_selection(request);
        assert_eq!(result.status, OperationStatus::Ok);
        state = result.selection_state.expect("selection state");
    }

    state
}

fn resolve_result_for_spec(
    spec: &ScenarioSpec,
    label: &str,
) -> compiler::loader_api::ResolveResult {
    let cmp_dir = emitted_cmp_dir(spec, label);
    let handle = open_handle(&cmp_dir);
    let selection_state = apply_selection_steps(&handle, spec);

    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: spec.scope.to_string(),
        selection_state,
    };
    let result = resolve_from_selection(request);
    assert_eq!(result.status, OperationStatus::Ok);

    // ADR-0030 D2: register and persist the emitted `.ccm` sibling so a later
    // `runtime-open` can load it. `open` advertises it at `<cmp>/ccm`
    // (configflux-9hi2); leak the CMP dir so it outlives this fixture build.
    let ccm_ref = handle.ccm_ref.clone();
    assert!(
        Path::new(ccm_ref.trim())
            .join("ccm.manifest.json")
            .is_file(),
        "fixture compile must emit a usable .ccm sibling at {ccm_ref}"
    );
    register_fixture_ccm_ref(&result.model_hash, &ccm_ref);
    cmp_dir.persist();

    result
}

fn runtime_open_request_from_resolve(
    result: &compiler::loader_api::ResolveResult,
) -> RuntimeOpenRequest {
    RuntimeOpenRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash: result.model_hash.clone(),
        // ADR-0030 D2: every opened fixture carries its emitted `.ccm` so the
        // open-time precondition is satisfied (the registry was populated when
        // the fixture was resolved).
        ccm_ref: fixture_ccm_ref(&result.model_hash),
        resolve_hash: result.resolve_hash.clone().expect("resolve hash"),
        scope: result.scope.clone(),
        resolved_output: result.resolved_output.clone().expect("resolved output"),
        resolved_component_dependencies: result.resolved_component_dependencies.clone(),
        resolved_artifacts: result.resolved_artifacts.clone(),
        context_tags: result.context_tags.clone(),
        choices: result.choices.clone(),
        // ADR-0047 §5 lockstep: copy the resolve's auto-bound-default provenance
        // so runtime_open reproduces the same resolve_hash the loader emitted.
        defaulted_choices: result.defaulted_choices.clone(),
        committed_overlay: std::collections::BTreeMap::new(),
        dirty_overlay: std::collections::BTreeMap::new(),
        dirty_generations: std::collections::BTreeMap::new(),
        dirty_metadata: std::collections::BTreeMap::new(),
        auto_reset_policy: AutoResetPolicy::default(),
        auto_reset_scheduler: AutoResetSchedulerState::default(),
        event_bus: RuntimeEventBusState::default(),
        sync_status: compiler::runtime_api::RuntimeSyncStatus::default(),
        audit_events: Vec::new(),
        audit_next_sequence: 1,
        audit_uploaded_sequence: 0,
        persistence_format_version: 1,
        persistence_journal_sequence: 0,
    }
}

fn run_runtime_open(
    resolve_result: &compiler::loader_api::ResolveResult,
) -> (RunOutput, RuntimeOpenResult) {
    let request = runtime_open_request_from_resolve(resolve_result);
    run_json_command(&["runtime-open"], &request)
}

fn open_snapshot(spec: &'static ScenarioSpec) -> RuntimeSnapshot {
    let resolve_result = resolved_fixture(spec);
    let (output, response) = run_runtime_open(&resolve_result);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    response.runtime_snapshot.expect("runtime_snapshot")
}

fn reopen_snapshot(spec: &'static ScenarioSpec, snapshot: RuntimeSnapshot) -> RuntimeSnapshot {
    let resolve_result = resolved_fixture(spec);
    let mut request = runtime_open_request_from_resolve(&resolve_result);
    request.committed_overlay = snapshot.committed_overlay;
    request.dirty_overlay = snapshot.dirty_overlay;
    request.dirty_generations = snapshot.dirty_generations;
    request.dirty_metadata = snapshot.dirty_metadata;
    request.auto_reset_policy = snapshot.auto_reset_policy;
    request.auto_reset_scheduler = snapshot.auto_reset_scheduler;
    request.event_bus = snapshot.event_bus;
    request.sync_status = snapshot.sync_status;
    request.audit_events = snapshot.audit_events;
    request.audit_next_sequence = snapshot.audit_next_sequence;
    request.audit_uploaded_sequence = snapshot.audit_uploaded_sequence;
    request.persistence_format_version = snapshot.persistence_format_version;
    request.persistence_journal_sequence = snapshot.persistence_journal_sequence;

    let (output, response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    response
        .runtime_snapshot
        .expect("reopened runtime snapshot")
}

fn parse_parameter_path(path: &str) -> (&str, &str) {
    let mut parts = path.split('.');
    assert_eq!(parts.next(), Some("component"));
    let component_id = parts.next().expect("component_id");
    assert_eq!(parts.next(), Some("param"));
    let param_key = parts.next().expect("param_key");
    assert!(parts.next().is_none());
    (component_id, param_key)
}

fn mutate_snapshot_parameter<F>(snapshot: &mut RuntimeSnapshot, path: &str, mutator: F)
where
    F: FnOnce(&mut compiler::resolved_models::ResolvedParameter),
{
    let (component_id, param_key) = parse_parameter_path(path);

    let mut mutator = Some(mutator);
    for resolved_scope in snapshot.resolved_output.values_mut() {
        let Some(component) = resolved_scope.components.get_mut(component_id) else {
            continue;
        };
        let Some(parameter) = component.params.get_mut(param_key) else {
            continue;
        };
        mutator.take().expect("single mutator")(parameter);
        return;
    }

    panic!("parameter path not found in snapshot: {path}");
}

fn run_017_request_io_path(request_path: &Path) -> RunOutput {
    let request_path_str = request_path.to_string_lossy().into_owned();
    run_cli(&["runtime-open", "--request-file", &request_path_str], b"")
}

#[test]
fn run_001_open_snapshot_happy_path() {
    for spec in all_specs() {
        let resolve_result = resolved_fixture(spec);
        let (output, response) = run_runtime_open(&resolve_result);

        assert_eq!(output.exit_code, EXIT_OK);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Ok);
        assert_eq!(response.model_hash, resolve_result.model_hash);
        assert_eq!(
            response.resolve_hash,
            resolve_result.resolve_hash.clone().expect("resolve hash")
        );
        assert_eq!(response.scope, spec.scope);
        assert!(response.runtime_snapshot.is_some());
    }

    let temp = emitted_cmp_dir(&S1_SMOKE_SPEC, "run-001-file-mode");
    let request_path = temp.path.join("runtime-open.request.json");
    let response_path = temp.path.join("runtime-open.response.json");
    let resolve_result = resolved_fixture(&S1_SMOKE_SPEC);
    let request = runtime_open_request_from_resolve(&resolve_result);
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize request"),
    )
    .expect("write request");

    let request_path_str = request_path.to_string_lossy().into_owned();
    let response_path_str = response_path.to_string_lossy().into_owned();

    let output = run_cli(
        &[
            "runtime-open",
            "--request-file",
            &request_path_str,
            "--response-file",
            &response_path_str,
        ],
        b"",
    );

    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let response_bytes = std::fs::read(&response_path).expect("read response");
    let response: RuntimeOpenResult =
        serde_json::from_slice(&response_bytes).expect("parse response");
    assert_eq!(response.status, OperationStatus::Ok);
}

#[test]
fn run_002_open_snapshot_schema_version_invalid() {
    let resolve_result = resolved_fixture(&S1_SMOKE_SPEC);
    let mut request = runtime_open_request_from_resolve(&resolve_result);
    request.schema_version = PRODUCT_SCHEMA_VERSION + 1;

    let (output, response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &request);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION.to_string()
    );
}

#[test]
fn run_003_open_snapshot_hash_mismatch() {
    let resolve_result = resolved_fixture(&S1_SMOKE_SPEC);
    let mut request = runtime_open_request_from_resolve(&resolve_result);
    request.resolve_hash = "00".repeat(32);
    let payload = serde_json::to_vec(&request).expect("serialize request");

    let first = run_cli(&["runtime-open"], &payload);
    let second = run_cli(&["runtime-open"], &payload);

    assert_eq!(first.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(first.stderr, second.stderr);
    assert_eq!(first.stdout, second.stdout);

    let response: RuntimeOpenResult = serde_json::from_slice(&first.stdout).expect("response");
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_RUNTIME_HASH_MISMATCH.to_string()
    );
}

#[test]
fn run_004_open_snapshot_unknown_scope_root() {
    let resolve_result = resolved_fixture(&S1_SMOKE_SPEC);
    let mut request = runtime_open_request_from_resolve(&resolve_result);
    request.scope = "component:missing_scope".to_string();

    let (output, response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &request);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNKNOWN_SCOPE.to_string()
    );
}

#[test]
fn run_005_get_scope_metadata_happy_path() {
    for spec in all_specs() {
        let snapshot = open_snapshot(spec);
        let request = GetScopeMetadataRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            scope_root: spec.scope_root.to_string(),
        };
        let (output, response): (RunOutput, GetScopeMetadataResult) =
            run_json_command(&["get-scope-metadata"], &request);

        assert_eq!(output.exit_code, EXIT_OK);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Ok);
        let metadata = response.metadata.expect("metadata");
        assert_eq!(metadata.component_count, spec.expected_component_count);
        assert_eq!(metadata.parameter_count, spec.expected_parameter_count);
        assert_eq!(metadata.artifact_count, spec.expected_artifact_count);
    }
}

#[test]
fn run_006_get_scope_metadata_unknown_scope() {
    let snapshot = open_snapshot(&S2_SMOKE_SPEC);
    let request = GetScopeMetadataRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        scope_root: "missing_scope".to_string(),
    };
    let (output, response): (RunOutput, GetScopeMetadataResult) =
        run_json_command(&["get-scope-metadata"], &request);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_RUNTIME_UNKNOWN_SCOPE.to_string()
    );
}

#[test]
fn run_007_list_parameters_happy_path_sorted() {
    for spec in all_specs() {
        let snapshot = open_snapshot(spec);
        let request = ListParametersRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            scope_root: spec.scope_root.to_string(),
        };
        let (output, response): (RunOutput, ListParametersResult) =
            run_json_command(&["list-parameters"], &request);

        assert_eq!(output.exit_code, EXIT_OK);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Ok);
        assert_eq!(
            response.parameter_paths.len(),
            spec.expected_parameter_count as usize
        );
        assert!(response
            .parameter_paths
            .windows(2)
            .all(|window| window[0] < window[1]));
        assert!(response
            .parameter_paths
            .contains(&spec.probe_path.to_string()));
    }
}

#[test]
fn run_008_get_parameter_happy_path() {
    for spec in all_specs() {
        let snapshot = open_snapshot(spec);
        let request = GetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: spec.probe_path.to_string(),
        };
        let (output, response): (RunOutput, GetParameterResult) =
            run_json_command(&["get-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_OK);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Ok);
        let parameter = response.parameter.expect("parameter");
        assert_eq!(parameter.path, spec.probe_path.to_string());
    }
}

#[test]
fn run_009_get_parameter_unknown_path() {
    for spec in [&S1_SMOKE_SPEC, &S4_MEDIUM_SPEC] {
        let snapshot = open_snapshot(spec);
        let request = GetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: "component.missing.param.unknown".to_string(),
        };
        let (output, response): (RunOutput, GetParameterResult) =
            run_json_command(&["get-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Error);
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RUNTIME_UNKNOWN_PATH.to_string()
        );
    }
}

#[test]
fn run_010_set_parameter_runtime_mutable_happy_path() {
    for spec in mutable_specs() {
        let snapshot = open_snapshot(spec);
        let path = spec.mutable_path.expect("mutable path");
        let request = SetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: path.to_string(),
            value: Value::Float(0.123),
            intent: compiler::runtime_api::OverrideIntent::default(),
            actor: None,
            reason: None,
        };
        let (output, response): (RunOutput, SetParameterResult) =
            run_json_command(&["set-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_OK);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Ok);
        let updated_snapshot = response.runtime_snapshot.expect("updated snapshot");

        let get_request = GetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: updated_snapshot,
            path: path.to_string(),
        };
        let (get_output, get_response): (RunOutput, GetParameterResult) =
            run_json_command(&["get-parameter"], &get_request);

        assert_eq!(get_output.exit_code, EXIT_OK);
        assert!(get_output.stderr.is_empty());
        assert_eq!(get_response.status, OperationStatus::Ok);
        assert_eq!(
            get_response.parameter.expect("parameter").value,
            Value::Float(0.123)
        );
    }
}

#[test]
fn run_011_set_parameter_lifecycle_immutable_rejected() {
    for spec in all_specs() {
        let snapshot = open_snapshot(spec);
        let request = SetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: spec.immutable_path.to_string(),
            value: Value::String("blocked".to_string()),
            intent: compiler::runtime_api::OverrideIntent::default(),
            actor: None,
            reason: None,
        };
        let (output, response): (RunOutput, SetParameterResult) =
            run_json_command(&["set-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Error);
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RUNTIME_LIFECYCLE_IMMUTABLE.to_string()
        );
    }
}

#[test]
fn run_012_set_parameter_type_mismatch_rejected() {
    for spec in mutable_specs() {
        let snapshot = open_snapshot(spec);
        let request = SetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: spec.mutable_path.expect("mutable path").to_string(),
            value: Value::String("not-a-number".to_string()),
            intent: compiler::runtime_api::OverrideIntent::default(),
            actor: None,
            reason: None,
        };
        let (output, response): (RunOutput, SetParameterResult) =
            run_json_command(&["set-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Error);
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RUNTIME_TYPE_MISMATCH.to_string()
        );
    }
}

#[test]
fn run_013_set_parameter_limit_violation_rejected() {
    for spec in mutable_specs() {
        let mut snapshot = open_snapshot(spec);
        let path = spec.mutable_path.expect("mutable path");
        mutate_snapshot_parameter(&mut snapshot, path, |parameter| {
            parameter.limits = Some(compiler::schema::Limits {
                min: Some(Value::Float(0.0)),
                max: Some(Value::Float(0.1)),
                min_len: None,
                max_len: None,
            });
        });

        let request = SetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: path.to_string(),
            value: Value::Float(0.9),
            intent: compiler::runtime_api::OverrideIntent::default(),
            actor: None,
            reason: None,
        };
        let (output, response): (RunOutput, SetParameterResult) =
            run_json_command(&["set-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Error);
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RUNTIME_LIMIT_VIOLATION.to_string()
        );
    }
}

#[test]
fn run_014_set_parameter_artifact_unknown_rejected() {
    for spec in all_specs() {
        let mut snapshot = open_snapshot(spec);
        mutate_snapshot_parameter(&mut snapshot, spec.artifact_path, |parameter| {
            parameter.lifecycle = compiler::schema::Lifecycle::Runtime;
        });

        let request = SetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: spec.artifact_path.to_string(),
            value: Value::String("missing_artifact".to_string()),
            intent: compiler::runtime_api::OverrideIntent::default(),
            actor: None,
            reason: None,
        };
        let (output, response): (RunOutput, SetParameterResult) =
            run_json_command(&["set-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Error);
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RUNTIME_ARTIFACT_UNKNOWN.to_string()
        );
    }
}

#[test]
fn run_015_transport_malformed_json_fail_closed() {
    let payload = br#"{"schema_version":1,"secret":"sk_live_should_not_leak","scope":"oops""#;
    let first = run_cli(&["runtime-open"], payload);
    let second = run_cli(&["runtime-open"], payload);

    assert_eq!(first.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(first.stdout.is_empty());
    assert_eq!(first.stderr, second.stderr);
    assert!(first.stderr.contains(E_RUNTIME_CLI_REQUEST_INVALID));
    assert!(!first.stderr.contains("sk_live_should_not_leak"));
    assert!(!first.stderr.to_lowercase().contains("panic"));
    assert!(!first.stderr.to_lowercase().contains("stack backtrace"));
}

#[test]
fn run_016_transport_oversized_payload_fail_closed() {
    let oversized = vec![b'a'; REQUEST_SIZE_LIMIT_BYTES + 1];
    let output = run_cli(&["runtime-open"], &oversized);

    assert_eq!(output.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains(E_RUNTIME_CLI_REQUEST_TOO_LARGE));
}

#[test]
fn run_017_transport_request_file_io_failures() {
    let temp = emitted_cmp_dir(&S1_SMOKE_SPEC, "run-017");

    let missing_path = temp.path.join("missing.request.json");
    let missing = run_017_request_io_path(&missing_path);
    assert_eq!(missing.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(missing.stdout.is_empty());
    assert!(missing.stderr.contains(E_RUNTIME_CLI_REQUEST_IO));
    assert!(missing.stderr.contains("not_found"));

    let malformed_path = temp.path.join("malformed.request.json");
    std::fs::write(&malformed_path, b"{not-json").expect("write malformed");
    let malformed = run_017_request_io_path(&malformed_path);
    assert_eq!(malformed.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(malformed.stdout.is_empty());
    assert!(malformed.stderr.contains(E_RUNTIME_CLI_REQUEST_INVALID));
}

#[test]
fn run_018_transport_response_file_io_failures() {
    let temp = emitted_cmp_dir(&S1_SMOKE_SPEC, "run-018");
    let resolve_result = resolved_fixture(&S1_SMOKE_SPEC);
    let request = runtime_open_request_from_resolve(&resolve_result);
    let request_path = temp.path.join("request.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize request"),
    )
    .expect("write request file");

    let response_path = temp.path.join("missing-dir").join("response.json");
    let request_path_str = request_path.to_string_lossy().into_owned();
    let response_path_str = response_path.to_string_lossy().into_owned();

    let first = run_cli(
        &[
            "runtime-open",
            "--request-file",
            &request_path_str,
            "--response-file",
            &response_path_str,
        ],
        b"",
    );
    let second = run_cli(
        &[
            "runtime-open",
            "--request-file",
            &request_path_str,
            "--response-file",
            &response_path_str,
        ],
        b"",
    );

    assert_eq!(first.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(first.stdout.is_empty());
    assert_eq!(first.stderr, second.stderr);
    assert!(first.stderr.contains(E_RUNTIME_CLI_RESPONSE_IO));
    assert!(first.stderr.contains("not_found"));
}

#[test]
fn run_019_determinism_replay_read_paths_byte_stable() {
    for spec in all_specs() {
        let snapshot = open_snapshot(spec);

        let metadata_request = GetScopeMetadataRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot.clone(),
            scope_root: spec.scope_root.to_string(),
        };
        let metadata_payload = serde_json::to_vec(&metadata_request).expect("metadata request");
        let metadata_a = run_cli(&["get-scope-metadata"], &metadata_payload);
        let metadata_b = run_cli(&["get-scope-metadata"], &metadata_payload);
        assert_eq!(metadata_a.exit_code, EXIT_OK);
        assert_eq!(metadata_b.exit_code, EXIT_OK);
        assert_eq!(metadata_a.stderr, metadata_b.stderr);
        assert_eq!(metadata_a.stdout, metadata_b.stdout);

        let list_request = ListParametersRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot.clone(),
            scope_root: spec.scope_root.to_string(),
        };
        let list_payload = serde_json::to_vec(&list_request).expect("list request");
        let list_a = run_cli(&["list-parameters"], &list_payload);
        let list_b = run_cli(&["list-parameters"], &list_payload);
        assert_eq!(list_a.exit_code, EXIT_OK);
        assert_eq!(list_b.exit_code, EXIT_OK);
        assert_eq!(list_a.stderr, list_b.stderr);
        assert_eq!(list_a.stdout, list_b.stdout);

        let get_request = GetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: spec.probe_path.to_string(),
        };
        let get_payload = serde_json::to_vec(&get_request).expect("get request");
        let get_a = run_cli(&["get-parameter"], &get_payload);
        let get_b = run_cli(&["get-parameter"], &get_payload);
        assert_eq!(get_a.exit_code, EXIT_OK);
        assert_eq!(get_b.exit_code, EXIT_OK);
        assert_eq!(get_a.stderr, get_b.stderr);
        assert_eq!(get_a.stdout, get_b.stdout);
    }
}

#[test]
fn run_020_determinism_write_readback_consistency_and_hash_lineage() {
    for spec in all_specs() {
        let resolve_a = resolve_result_for_spec(spec, &format!("run-020-a-{}", spec.id));
        let resolve_b = resolve_result_for_spec(spec, &format!("run-020-b-{}", spec.id));

        assert_eq!(resolve_a.model_hash, resolve_b.model_hash);
        assert_eq!(
            resolve_a.selection_state_hash,
            resolve_b.selection_state_hash
        );
        assert_eq!(resolve_a.resolve_hash, resolve_b.resolve_hash);
        assert_eq!(resolve_a.resolved_output, resolve_b.resolved_output);

        let open_request = runtime_open_request_from_resolve(&resolve_a);
        let open_payload = serde_json::to_vec(&open_request).expect("open request");
        let open_a = run_cli(&["runtime-open"], &open_payload);
        let open_b = run_cli(&["runtime-open"], &open_payload);
        assert_eq!(open_a.exit_code, EXIT_OK);
        assert_eq!(open_b.exit_code, EXIT_OK);
        assert_eq!(open_a.stderr, open_b.stderr);

        let open_response: RuntimeOpenResult =
            serde_json::from_slice(&open_a.stdout).expect("open response");
        let open_response_replay: RuntimeOpenResult =
            serde_json::from_slice(&open_b.stdout).expect("open replay response");
        assert_eq!(open_response.status, open_response_replay.status);
        assert_eq!(open_response.model_hash, open_response_replay.model_hash);
        assert_eq!(
            open_response.resolve_hash,
            open_response_replay.resolve_hash
        );
        assert_eq!(open_response.scope, open_response_replay.scope);
        assert_eq!(open_response.status, OperationStatus::Ok);
        assert_eq!(open_response.model_hash, resolve_a.model_hash);
        assert_eq!(
            open_response.resolve_hash,
            resolve_a.resolve_hash.clone().expect("hash")
        );

        let snapshot = open_response.runtime_snapshot.expect("snapshot");
        assert_eq!(snapshot.model_hash, resolve_a.model_hash);
        assert_eq!(
            snapshot.resolve_hash,
            resolve_a.resolve_hash.clone().expect("hash")
        );
        assert_eq!(snapshot.choices, resolve_a.choices);
        assert_eq!(snapshot.context_tags, resolve_a.context_tags);

        let list_request = ListParametersRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot.clone(),
            scope_root: spec.scope_root.to_string(),
        };
        let (list_output, list_response): (RunOutput, ListParametersResult) =
            run_json_command(&["list-parameters"], &list_request);
        assert_eq!(list_output.exit_code, EXIT_OK);
        assert_eq!(list_response.model_hash, resolve_a.model_hash);
        assert_eq!(
            list_response.resolve_hash,
            resolve_a.resolve_hash.clone().expect("resolve hash")
        );

        let get_request = GetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot.clone(),
            path: spec.probe_path.to_string(),
        };
        let (get_output, get_response): (RunOutput, GetParameterResult) =
            run_json_command(&["get-parameter"], &get_request);
        assert_eq!(get_output.exit_code, EXIT_OK);
        assert_eq!(get_response.model_hash, resolve_a.model_hash);
        assert_eq!(
            get_response.resolve_hash,
            resolve_a.resolve_hash.clone().expect("resolve hash")
        );

        if let Some(mutable_path) = spec.mutable_path {
            let set_request = SetParameterRequest {
                schema_version: PRODUCT_SCHEMA_VERSION,
                runtime_snapshot: snapshot,
                path: mutable_path.to_string(),
                value: Value::Float(0.177),
                intent: compiler::runtime_api::OverrideIntent::default(),
                actor: None,
                reason: None,
            };
            let (set_output, set_response): (RunOutput, SetParameterResult) =
                run_json_command(&["set-parameter"], &set_request);
            assert_eq!(set_output.exit_code, EXIT_OK);
            assert_eq!(set_response.status, OperationStatus::Ok);
            assert_eq!(set_response.model_hash, resolve_a.model_hash);
            assert_eq!(
                set_response.resolve_hash,
                resolve_a.resolve_hash.clone().expect("resolve hash")
            );

            let updated_snapshot = set_response.runtime_snapshot.expect("updated snapshot");
            assert_eq!(updated_snapshot.model_hash, resolve_a.model_hash);
            assert_eq!(
                updated_snapshot.resolve_hash,
                resolve_a.resolve_hash.clone().expect("resolve hash")
            );

            let readback_request = GetParameterRequest {
                schema_version: PRODUCT_SCHEMA_VERSION,
                runtime_snapshot: updated_snapshot,
                path: mutable_path.to_string(),
            };
            let (_, readback_response): (RunOutput, GetParameterResult) =
                run_json_command(&["get-parameter"], &readback_request);
            assert_eq!(readback_response.status, OperationStatus::Ok);
            assert_eq!(
                readback_response.parameter.expect("parameter").value,
                Value::Float(0.177)
            );
        }
    }
}

#[test]
fn run_021_set_parameter_populates_dirty_metadata_and_scheduler_entries() {
    for spec in mutable_specs() {
        let snapshot = open_snapshot(spec);
        let path = spec.mutable_path.expect("mutable path");

        let request = SetParameterRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            runtime_snapshot: snapshot,
            path: path.to_string(),
            value: Value::Float(0.321),
            intent: compiler::runtime_api::OverrideIntent::default(),
            actor: None,
            reason: None,
        };
        let (output, response): (RunOutput, SetParameterResult) =
            run_json_command(&["set-parameter"], &request);

        assert_eq!(output.exit_code, EXIT_OK);
        assert_eq!(response.status, OperationStatus::Ok);

        let updated_snapshot = response.runtime_snapshot.expect("updated snapshot");
        let metadata = updated_snapshot
            .dirty_metadata
            .get(spec.scope_root)
            .and_then(|entries| entries.get(path))
            .expect("dirty metadata");
        assert_eq!(metadata.actor, "runtime_api.set_parameter");
        assert!(metadata.generation >= 1);
        assert!(metadata.reset_deadline_unix_ms.is_some());
        assert_eq!(
            updated_snapshot
                .dirty_generations
                .get(spec.scope_root)
                .and_then(|entries| entries.get(path))
                .copied()
                .expect("dirty generation"),
            metadata.generation
        );
        assert!(updated_snapshot
            .auto_reset_scheduler
            .pending
            .iter()
            .any(|entry| {
                entry.scope_root == spec.scope_root
                    && entry.path == path
                    && entry.generation == metadata.generation
                    && Some(entry.deadline_unix_ms) == metadata.reset_deadline_unix_ms
            }));
        assert!(updated_snapshot
            .event_bus
            .events
            .iter()
            .any(|event| event.event_kind == RuntimeEventKind::ParameterChanged));
        assert!(updated_snapshot
            .event_bus
            .events
            .iter()
            .any(|event| event.event_kind == RuntimeEventKind::DirtyStateChanged));
        assert!(updated_snapshot
            .event_bus
            .events
            .iter()
            .zip(updated_snapshot.event_bus.events.iter().skip(1))
            .all(|(left, right)| left.sequence < right.sequence));
    }
}

#[test]
fn run_022_runtime_open_scheduler_is_generation_safe_and_restart_persistent() {
    let resolve_result = resolved_fixture(&S1_SMOKE_SPEC);
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");

    let write_one = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: open_snapshot(&S1_SMOKE_SPEC),
        path: path.to_string(),
        value: Value::Float(0.11),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, write_one_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &write_one);
    let write_one_snapshot = write_one_response
        .runtime_snapshot
        .expect("write one snapshot");

    let write_two = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: write_one_snapshot,
        path: path.to_string(),
        value: Value::Float(0.22),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, write_two_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &write_two);
    let write_two_snapshot = write_two_response
        .runtime_snapshot
        .expect("write two snapshot");

    let mut stale_gen_request = runtime_open_request_from_resolve(&resolve_result);
    stale_gen_request.committed_overlay = write_two_snapshot.committed_overlay.clone();
    stale_gen_request.dirty_overlay = write_two_snapshot.dirty_overlay.clone();
    stale_gen_request.dirty_generations = write_two_snapshot.dirty_generations.clone();
    stale_gen_request.dirty_metadata = write_two_snapshot.dirty_metadata.clone();
    stale_gen_request.auto_reset_policy = write_two_snapshot.auto_reset_policy.clone();
    stale_gen_request.auto_reset_scheduler = write_two_snapshot.auto_reset_scheduler.clone();
    stale_gen_request.event_bus = write_two_snapshot.event_bus.clone();
    for entry in &mut stale_gen_request.auto_reset_scheduler.pending {
        if entry.generation == 1 {
            entry.deadline_unix_ms = 0;
        } else if entry.generation == 2 {
            entry.deadline_unix_ms = u64::MAX;
        }
    }

    let (_, stale_generation_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &stale_gen_request);
    assert_eq!(stale_generation_response.status, OperationStatus::Ok);
    let stale_generation_snapshot = stale_generation_response
        .runtime_snapshot
        .expect("stale generation snapshot");
    assert_eq!(
        stale_generation_snapshot
            .dirty_overlay
            .get(S1_SMOKE_SPEC.scope_root)
            .and_then(|entries| entries.get(path))
            .cloned(),
        Some(Value::Float(0.22))
    );
    assert!(!stale_generation_snapshot
        .event_bus
        .events
        .iter()
        .any(|event| {
            matches!(
                event.payload,
                RuntimeEventPayload::ResetApplied { generation: 1, .. }
            )
        }));

    let mut current_gen_request = runtime_open_request_from_resolve(&resolve_result);
    current_gen_request.committed_overlay = stale_generation_snapshot.committed_overlay.clone();
    current_gen_request.dirty_overlay = stale_generation_snapshot.dirty_overlay.clone();
    current_gen_request.dirty_generations = stale_generation_snapshot.dirty_generations.clone();
    current_gen_request.dirty_metadata = stale_generation_snapshot.dirty_metadata.clone();
    current_gen_request.auto_reset_policy = stale_generation_snapshot.auto_reset_policy.clone();
    current_gen_request.auto_reset_scheduler =
        stale_generation_snapshot.auto_reset_scheduler.clone();
    current_gen_request.event_bus = stale_generation_snapshot.event_bus.clone();
    let mut forced_deadline = None;
    if let Some(metadata) = current_gen_request
        .dirty_metadata
        .get_mut(S1_SMOKE_SPEC.scope_root)
        .and_then(|entries| entries.get_mut(path))
    {
        let deadline = metadata.dirty_since_unix_ms;
        metadata.reset_deadline_unix_ms = Some(deadline);
        forced_deadline = Some(deadline);
    }
    if let Some(deadline) = forced_deadline {
        for entry in &mut current_gen_request.auto_reset_scheduler.pending {
            if entry.generation == 2 {
                entry.deadline_unix_ms = deadline;
            }
        }
    }

    let (_, current_generation_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &current_gen_request);
    assert_eq!(current_generation_response.status, OperationStatus::Ok);
    let current_generation_snapshot = current_generation_response
        .runtime_snapshot
        .expect("current generation snapshot");
    assert!(current_generation_snapshot
        .dirty_overlay
        .get(S1_SMOKE_SPEC.scope_root)
        .and_then(|entries| entries.get(path))
        .is_none());
    assert!(current_generation_snapshot
        .dirty_metadata
        .get(S1_SMOKE_SPEC.scope_root)
        .and_then(|entries| entries.get(path))
        .is_none());
    assert!(current_generation_snapshot
        .dirty_generations
        .get(S1_SMOKE_SPEC.scope_root)
        .and_then(|entries| entries.get(path))
        .is_none());
    assert!(current_generation_snapshot
        .event_bus
        .events
        .iter()
        .any(|event| {
            matches!(
                event.payload,
                RuntimeEventPayload::ResetApplied { generation: 2, .. }
            )
        }));
    assert!(current_generation_snapshot
        .event_bus
        .events
        .iter()
        .any(|event| {
            matches!(
                event.payload,
                RuntimeEventPayload::DirtyStateChanged {
                    dirty: false,
                    generation: 2,
                    ..
                }
            )
        }));
}

#[test]
fn run_023_event_bus_sequence_and_backpressure_are_deterministic() {
    let mut snapshot = open_snapshot(&S1_SMOKE_SPEC);
    snapshot.event_bus.buffer_capacity = 2;
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");

    let first_write = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: path.to_string(),
        value: Value::Float(0.44),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, first_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &first_write);
    let first_snapshot = first_response.runtime_snapshot.expect("first snapshot");
    assert_eq!(first_snapshot.event_bus.events.len(), 2);
    assert_eq!(first_snapshot.event_bus.dropped_events, 1);
    let first_sequences: Vec<u64> = first_snapshot
        .event_bus
        .events
        .iter()
        .map(|event| event.sequence)
        .collect();
    assert_eq!(first_sequences, vec![2, 3]);

    let second_write = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: first_snapshot,
        path: path.to_string(),
        value: Value::Float(0.45),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, second_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &second_write);
    let second_snapshot = second_response.runtime_snapshot.expect("second snapshot");
    assert_eq!(second_snapshot.event_bus.events.len(), 2);
    assert_eq!(second_snapshot.event_bus.dropped_events, 3);
    let second_sequences: Vec<u64> = second_snapshot
        .event_bus
        .events
        .iter()
        .map(|event| event.sequence)
        .collect();
    assert_eq!(second_sequences, vec![4, 5]);
}

#[test]
fn run_024_set_parameters_atomically_and_dirty_metadata_commands() {
    let snapshot = open_snapshot(&S1_SMOKE_SPEC);
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");

    let request = SetParametersAtomicallyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        writes: vec![compiler::runtime_api::AtomicParameterWrite {
            path: path.to_string(),
            value: Value::Float(0.51),
        }],
        actor: "runtime-cli.test".to_string(),
        reason: Some("batch-set".to_string()),
        expected_working_configuration_id: None,
        intent: compiler::runtime_api::OverrideIntent::default(),
    };
    let (set_output, set_response): (RunOutput, SetParametersAtomicallyResult) =
        run_json_command(&["set-parameters-atomically"], &request);
    assert_eq!(set_output.exit_code, EXIT_OK);
    assert_eq!(set_response.status, OperationStatus::Ok);
    assert_eq!(set_response.applied_count, 1);
    assert!(set_response.rejected_paths.is_empty());
    assert!(set_response.dirty_generation_max >= 1);

    let updated_snapshot = set_response.runtime_snapshot.expect("updated snapshot");
    let list_request = ListDirtyParametersRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: updated_snapshot.clone(),
        scope_root: S1_SMOKE_SPEC.scope_root.to_string(),
    };
    let (list_output, list_response): (RunOutput, ListDirtyParametersResult) =
        run_json_command(&["list-dirty-parameters"], &list_request);
    assert_eq!(list_output.exit_code, EXIT_OK);
    assert_eq!(list_response.status, OperationStatus::Ok);
    assert_eq!(list_response.dirty_count, 1);
    assert_eq!(
        list_response.dirty_paths[0],
        format!("{}/{}", S1_SMOKE_SPEC.scope_root, path)
    );

    let metadata_request = GetDirtyMetadataRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: updated_snapshot,
        path: path.to_string(),
    };
    let (metadata_output, metadata_response): (RunOutput, GetDirtyMetadataResult) =
        run_json_command(&["get-dirty-metadata"], &metadata_request);
    assert_eq!(metadata_output.exit_code, EXIT_OK);
    assert_eq!(metadata_response.status, OperationStatus::Ok);
    assert!(metadata_response.dirty);
    assert_eq!(
        metadata_response.metadata.expect("metadata").actor,
        "runtime-cli.test"
    );
}

#[test]
fn run_025_commit_rollback_and_identity_commands_available() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: open_snapshot(&S1_SMOKE_SPEC),
        path: path.to_string(),
        value: Value::Float(0.52),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &set_request);
    let dirty_snapshot = set_response.runtime_snapshot.expect("dirty snapshot");

    let identity_before_request = GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_snapshot.clone(),
    };
    let (_, identity_before): (RunOutput, GetConfigurationIdentityResult) =
        run_json_command(&["get-configuration-identity"], &identity_before_request);
    let identity_before = identity_before.identity.expect("identity before");

    let commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_snapshot.clone(),
        actor: "runtime-cli.commit".to_string(),
        reason: Some("commit from cli".to_string()),
        expected_base_configuration_id: Some(identity_before.committed_configuration_id.clone()),
        changed_paths_hint: vec![path.to_string()],
    };
    let (commit_output, commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &commit_request);
    assert_eq!(commit_output.exit_code, EXIT_OK);
    assert_eq!(commit_response.status, OperationStatus::Ok);
    assert!(commit_response.commit_id.is_some());
    let committed_snapshot = commit_response
        .runtime_snapshot
        .expect("committed snapshot");

    let identity_after_request = GetConfigurationIdentityRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: committed_snapshot.clone(),
    };
    let (_, identity_after): (RunOutput, GetConfigurationIdentityResult) =
        run_json_command(&["get-configuration-identity"], &identity_after_request);
    let identity_after = identity_after.identity.expect("identity after");
    assert_eq!(
        identity_after.committed_configuration_id,
        identity_after.working_configuration_id
    );

    let set_again_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: committed_snapshot,
        path: path.to_string(),
        value: Value::Float(0.63),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, set_again_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &set_again_request);
    let rollback_request = RollbackDirtyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: set_again_response
            .runtime_snapshot
            .expect("dirty again snapshot"),
        mode: compiler::runtime_api::RollbackMode::All,
        paths: Vec::new(),
        actor: "runtime-cli.rollback".to_string(),
        reason: Some("rollback from cli".to_string()),
    };
    let (rollback_output, rollback_response): (RunOutput, RollbackDirtyResult) =
        run_json_command(&["rollback-dirty"], &rollback_request);
    assert_eq!(rollback_output.exit_code, EXIT_OK);
    assert_eq!(rollback_response.status, OperationStatus::Ok);
    assert!(!rollback_response.rolled_back_paths.is_empty());
}

#[test]
fn run_026_policy_and_sync_commands_happy_path() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let policy_request = SetAutoResetPolicyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: open_snapshot(&S1_SMOKE_SPEC),
        auto_reset_policy: AutoResetPolicy {
            enabled: true,
            default_timeout_ms: 5_000,
            per_path_overrides: BTreeMap::new(),
            policy_revision: 0,
        },
    };
    let (_, policy_response): (RunOutput, SetAutoResetPolicyResult) =
        run_json_command(&["set-auto-reset-policy"], &policy_request);
    assert_eq!(policy_response.status, OperationStatus::Ok);
    let policy_snapshot = policy_response.runtime_snapshot.expect("policy snapshot");
    assert_eq!(
        policy_response
            .auto_reset_policy
            .expect("auto reset policy")
            .policy_revision,
        1
    );

    let get_policy_request = GetAutoResetPolicyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: policy_snapshot.clone(),
    };
    let (_, get_policy_response): (RunOutput, GetAutoResetPolicyResult) =
        run_json_command(&["get-auto-reset-policy"], &get_policy_request);
    assert_eq!(get_policy_response.status, OperationStatus::Ok);
    assert_eq!(
        get_policy_response
            .auto_reset_policy
            .expect("retrieved policy")
            .policy_revision,
        1
    );

    let check_offline_request = CheckForUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: policy_snapshot,
        backend_connected: false,
        pending_update_summary: None,
    };
    let (_, check_offline_response): (RunOutput, CheckForUpdatesResult) =
        run_json_command(&["check-for-updates"], &check_offline_request);
    assert_eq!(check_offline_response.status, OperationStatus::Ok);
    let offline_snapshot = check_offline_response
        .runtime_snapshot
        .expect("offline snapshot");
    assert_eq!(
        check_offline_response
            .sync_status
            .expect("offline status")
            .sync_state,
        compiler::runtime_api::RuntimeSyncState::Offline
    );

    let check_online_request = CheckForUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: offline_snapshot,
        backend_connected: true,
        pending_update_summary: Some("one update available".to_string()),
    };
    let (_, check_online_response): (RunOutput, CheckForUpdatesResult) =
        run_json_command(&["check-for-updates"], &check_online_request);
    assert_eq!(check_online_response.status, OperationStatus::Ok);
    let sync_snapshot = check_online_response
        .runtime_snapshot
        .expect("sync snapshot");

    let source_set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: sync_snapshot.clone(),
        path: path.to_string(),
        value: Value::Float(0.66),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, source_set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &source_set_request);
    let source_commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: source_set_response
            .runtime_snapshot
            .expect("source dirty snapshot"),
        actor: "runtime-cli.sync-source".to_string(),
        reason: Some("prepare delta payload".to_string()),
        expected_base_configuration_id: None,
        changed_paths_hint: vec![path.to_string()],
    };
    let (_, source_commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &source_commit_request);
    let source_delta_manifest = source_commit_response
        .delta_manifest
        .expect("source delta manifest");
    let canonical_path = format!("{}/{}", S1_SMOKE_SPEC.scope_root, path);
    let source_delta_change = source_delta_manifest
        .changed_paths
        .iter()
        .find(|entry| entry.path == canonical_path)
        .cloned()
        .expect("source changed path");

    let pull_request = PullUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: sync_snapshot,
        actor: "runtime-cli.sync".to_string(),
        reason: Some("sync apply".to_string()),
        backend_connected: true,
        source: compiler::runtime_api::SyncApplySource::Backend,
        writes: vec![compiler::runtime_api::PullUpdateWrite {
            path: path.to_string(),
            value: source_delta_change.after_value.expect("after value"),
            before_leaf_hash: source_delta_change.before_leaf_hash,
            after_leaf_hash: source_delta_change.after_leaf_hash,
        }],
        base_configuration_id: Some(source_delta_manifest.base_configuration_id.clone()),
        full_snapshot: false,
        pending_update_summary: Some("applied update".to_string()),
        target_configuration_id: Some(source_delta_manifest.target_configuration_id),
    };
    let (pull_output, pull_response): (RunOutput, PullUpdatesResult) =
        run_json_command(&["pull-updates"], &pull_request);
    assert_eq!(pull_output.exit_code, EXIT_OK);
    assert_eq!(pull_response.status, OperationStatus::Ok);
    assert_eq!(pull_response.applied_paths.len(), 1);

    let sync_status_request = GetSyncStatusRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: pull_response.runtime_snapshot.expect("pulled snapshot"),
    };
    let (_, sync_status_response): (RunOutput, GetSyncStatusResult) =
        run_json_command(&["get-sync-status"], &sync_status_request);
    assert_eq!(sync_status_response.status, OperationStatus::Ok);
    assert_eq!(
        sync_status_response
            .sync_status
            .expect("sync status")
            .sync_state,
        compiler::runtime_api::RuntimeSyncState::Idle
    );
}

#[test]
fn run_027_audit_pipeline_is_local_first_and_idempotent() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: open_snapshot(&S1_SMOKE_SPEC),
        path: path.to_string(),
        value: Value::Float(0.77),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &set_request);

    let commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: set_response.runtime_snapshot.expect("dirty snapshot"),
        actor: "runtime-cli.audit".to_string(),
        reason: Some("audit commit".to_string()),
        expected_base_configuration_id: None,
        changed_paths_hint: vec![path.to_string()],
    };
    let (_, commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &commit_request);
    let committed_snapshot = commit_response
        .runtime_snapshot
        .expect("committed snapshot");
    assert!(committed_snapshot.audit_events.len() >= 2);

    let offline_push_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: committed_snapshot,
        backend_connected: false,
        max_events: 32,
    };
    let (_, offline_push_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &offline_push_request);
    assert_eq!(offline_push_response.status, OperationStatus::Ok);
    assert_eq!(offline_push_response.pushed_count, 0);
    assert!(offline_push_response.pending_count >= 2);

    let online_push_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: offline_push_response
            .runtime_snapshot
            .expect("offline snapshot"),
        backend_connected: true,
        max_events: 1,
    };
    let (_, online_push_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &online_push_request);
    assert_eq!(online_push_response.status, OperationStatus::Ok);
    assert_eq!(online_push_response.pushed_count, 1);
    assert_eq!(online_push_response.pushed_event_ids.len(), 1);
    assert!(online_push_response.pending_count >= 1);

    let drain_push_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: online_push_response
            .runtime_snapshot
            .expect("online snapshot"),
        backend_connected: true,
        max_events: 32,
    };
    let (_, drain_push_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &drain_push_request);
    assert_eq!(drain_push_response.status, OperationStatus::Ok);

    let idempotent_push_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: drain_push_response
            .runtime_snapshot
            .expect("drained snapshot"),
        backend_connected: true,
        max_events: 32,
    };
    let (_, idempotent_push_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &idempotent_push_request);
    assert_eq!(idempotent_push_response.status, OperationStatus::Ok);
    assert_eq!(idempotent_push_response.pushed_count, 0);
    assert_eq!(idempotent_push_response.pending_count, 0);
}

#[test]
fn run_028_backward_compat_v1_set_parameter_with_v2_state_fields() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: open_snapshot(&S1_SMOKE_SPEC),
        path: path.to_string(),
        value: Value::Float(0.88),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (set_output, set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &set_request);
    assert_eq!(set_output.exit_code, EXIT_OK);
    assert_eq!(set_response.status, OperationStatus::Ok);

    let sync_status_request = GetSyncStatusRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: set_response.runtime_snapshot.expect("updated snapshot"),
    };
    let (_, sync_status_response): (RunOutput, GetSyncStatusResult) =
        run_json_command(&["get-sync-status"], &sync_status_request);
    assert_eq!(sync_status_response.status, OperationStatus::Ok);
    assert_eq!(
        sync_status_response
            .sync_status
            .expect("sync status")
            .sync_state,
        compiler::runtime_api::RuntimeSyncState::Idle
    );
}

#[test]
fn run_029_pull_updates_enforces_delta_preconditions_and_upstream_wins_conflicts() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let baseline_snapshot = open_snapshot(&S1_SMOKE_SPEC);

    let source_set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: baseline_snapshot.clone(),
        path: path.to_string(),
        value: Value::Float(0.91),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, source_set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &source_set_request);
    let source_commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: source_set_response
            .runtime_snapshot
            .expect("source dirty snapshot"),
        actor: "runtime-cli.delta-source".to_string(),
        reason: Some("produce delta manifest".to_string()),
        expected_base_configuration_id: None,
        changed_paths_hint: vec![path.to_string()],
    };
    let (_, source_commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &source_commit_request);
    let source_manifest = source_commit_response
        .delta_manifest
        .expect("source delta manifest");
    let canonical_path = format!("{}/{}", S1_SMOKE_SPEC.scope_root, path);
    let delta_change = source_manifest
        .changed_paths
        .iter()
        .find(|entry| entry.path == canonical_path)
        .cloned()
        .expect("delta change");

    let dirty_receiver_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: baseline_snapshot.clone(),
        path: path.to_string(),
        value: Value::Float(0.55),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, dirty_receiver_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &dirty_receiver_request);
    let dirty_receiver_snapshot = dirty_receiver_response
        .runtime_snapshot
        .expect("dirty receiver snapshot");

    let wrong_base_pull_request = PullUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_receiver_snapshot.clone(),
        actor: "runtime-cli.sync".to_string(),
        reason: Some("wrong base".to_string()),
        backend_connected: true,
        source: compiler::runtime_api::SyncApplySource::Backend,
        writes: vec![compiler::runtime_api::PullUpdateWrite {
            path: path.to_string(),
            value: delta_change.after_value.clone().expect("after value"),
            before_leaf_hash: delta_change.before_leaf_hash.clone(),
            after_leaf_hash: delta_change.after_leaf_hash.clone(),
        }],
        base_configuration_id: Some("0".repeat(64)),
        full_snapshot: false,
        pending_update_summary: Some("delta payload".to_string()),
        target_configuration_id: Some(source_manifest.target_configuration_id.clone()),
    };
    let (wrong_base_output, wrong_base_response): (RunOutput, PullUpdatesResult) =
        run_json_command(&["pull-updates"], &wrong_base_pull_request);
    assert_eq!(wrong_base_output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(wrong_base_response.status, OperationStatus::Error);
    assert_eq!(
        wrong_base_response.diagnostics.diagnostics[0].code,
        compiler::runtime_api::E_RUNTIME_SYNC_BASE_MISMATCH
    );

    let wrong_before_hash_request = PullUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_receiver_snapshot.clone(),
        actor: "runtime-cli.sync".to_string(),
        reason: Some("wrong before hash".to_string()),
        backend_connected: true,
        source: compiler::runtime_api::SyncApplySource::Backend,
        writes: vec![compiler::runtime_api::PullUpdateWrite {
            path: path.to_string(),
            value: delta_change.after_value.clone().expect("after value"),
            before_leaf_hash: Some("f".repeat(64)),
            after_leaf_hash: delta_change.after_leaf_hash.clone(),
        }],
        base_configuration_id: Some(source_manifest.base_configuration_id.clone()),
        full_snapshot: false,
        pending_update_summary: Some("delta payload".to_string()),
        target_configuration_id: Some(source_manifest.target_configuration_id.clone()),
    };
    let (wrong_before_output, wrong_before_response): (RunOutput, PullUpdatesResult) =
        run_json_command(&["pull-updates"], &wrong_before_hash_request);
    assert_eq!(wrong_before_output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(wrong_before_response.status, OperationStatus::Error);
    assert_eq!(
        wrong_before_response.diagnostics.diagnostics[0].code,
        compiler::runtime_api::E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH
    );

    let unchanged_path_request = GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_receiver_snapshot.clone(),
        path: S1_SMOKE_SPEC.immutable_path.to_string(),
    };
    let (_, unchanged_before): (RunOutput, GetParameterResult) =
        run_json_command(&["get-parameter"], &unchanged_path_request);
    let unchanged_before_value = unchanged_before.parameter.expect("before param").value;

    let successful_pull_request = PullUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty_receiver_snapshot,
        actor: "runtime-cli.sync".to_string(),
        reason: Some("upstream apply".to_string()),
        backend_connected: true,
        source: compiler::runtime_api::SyncApplySource::Backend,
        writes: vec![compiler::runtime_api::PullUpdateWrite {
            path: path.to_string(),
            value: delta_change.after_value.expect("after value"),
            before_leaf_hash: delta_change.before_leaf_hash,
            after_leaf_hash: delta_change.after_leaf_hash,
        }],
        base_configuration_id: Some(source_manifest.base_configuration_id.clone()),
        full_snapshot: false,
        pending_update_summary: Some("delta applied".to_string()),
        target_configuration_id: Some(source_manifest.target_configuration_id.clone()),
    };
    let (success_output, success_response): (RunOutput, PullUpdatesResult) =
        run_json_command(&["pull-updates"], &successful_pull_request);
    assert_eq!(success_output.exit_code, EXIT_OK);
    assert_eq!(success_response.status, OperationStatus::Ok);
    assert_eq!(
        success_response.conflict_paths,
        vec![canonical_path.clone()]
    );
    assert_eq!(success_response.warning_count, 1);
    assert_eq!(
        success_response
            .sync_status
            .as_ref()
            .expect("sync status")
            .sync_diagnostics[0]
            .code,
        compiler::runtime_api::E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN
    );

    let successful_snapshot = success_response
        .runtime_snapshot
        .expect("successful snapshot");
    let unchanged_after_request = GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: successful_snapshot.clone(),
        path: S1_SMOKE_SPEC.immutable_path.to_string(),
    };
    let (_, unchanged_after): (RunOutput, GetParameterResult) =
        run_json_command(&["get-parameter"], &unchanged_after_request);
    assert_eq!(
        unchanged_after.parameter.expect("after param").value,
        unchanged_before_value
    );

    let subscribe_request = SubscribeEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: successful_snapshot,
        from_sequence: 0,
        max_events: 256,
        event_kinds: Vec::new(),
    };
    let (_, subscribe_response): (RunOutput, SubscribeEventsResult) =
        run_json_command(&["subscribe-events"], &subscribe_request);
    assert_eq!(subscribe_response.status, OperationStatus::Ok);
    assert!(subscribe_response
        .events
        .iter()
        .any(|event| event.event_kind == RuntimeEventKind::SyncConflictDetected));
    assert!(subscribe_response
        .events
        .iter()
        .any(|event| event.event_kind == RuntimeEventKind::SyncConflictResolved));
    assert!(subscribe_response
        .events
        .iter()
        .any(|event| event.event_kind == RuntimeEventKind::SyncApplyCompleted));
}

#[test]
fn run_030_direct_push_offline_and_export_pending_sync_bundle() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let baseline_snapshot = open_snapshot(&S1_SMOKE_SPEC);

    let source_set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: baseline_snapshot.clone(),
        path: path.to_string(),
        value: Value::Float(0.79),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, source_set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &source_set_request);
    let source_commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: source_set_response
            .runtime_snapshot
            .expect("source dirty snapshot"),
        actor: "runtime-cli.direct-push-source".to_string(),
        reason: Some("prepare direct push delta".to_string()),
        expected_base_configuration_id: None,
        changed_paths_hint: vec![path.to_string()],
    };
    let (_, source_commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &source_commit_request);
    let source_manifest = source_commit_response
        .delta_manifest
        .expect("source delta manifest");
    let canonical_path = format!("{}/{}", S1_SMOKE_SPEC.scope_root, path);
    let delta_change = source_manifest
        .changed_paths
        .iter()
        .find(|entry| entry.path == canonical_path)
        .cloned()
        .expect("delta change");

    let direct_push_request = PullUpdatesRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: baseline_snapshot,
        actor: "runtime-cli.direct-push".to_string(),
        reason: Some("commissioning direct push".to_string()),
        backend_connected: false,
        source: compiler::runtime_api::SyncApplySource::DirectPush,
        writes: vec![compiler::runtime_api::PullUpdateWrite {
            path: path.to_string(),
            value: delta_change.after_value.expect("after value"),
            before_leaf_hash: delta_change.before_leaf_hash,
            after_leaf_hash: delta_change.after_leaf_hash,
        }],
        base_configuration_id: Some(source_manifest.base_configuration_id),
        full_snapshot: false,
        pending_update_summary: Some("direct push applied".to_string()),
        target_configuration_id: Some(source_manifest.target_configuration_id),
    };
    let (direct_push_output, direct_push_response): (RunOutput, PullUpdatesResult) =
        run_json_command(&["pull-updates"], &direct_push_request);
    assert_eq!(direct_push_output.exit_code, EXIT_OK);
    assert_eq!(direct_push_response.status, OperationStatus::Ok);
    assert_eq!(direct_push_response.applied_paths, vec![canonical_path]);
    assert!(direct_push_response.audit_event_id.is_some());

    let export_request = ExportPendingSyncBundleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: direct_push_response
            .runtime_snapshot
            .expect("direct push snapshot"),
        max_audit_events: 64,
    };
    let (export_output, export_response): (RunOutput, ExportPendingSyncBundleResult) =
        run_json_command(&["export-pending-sync-bundle"], &export_request);
    assert_eq!(export_output.exit_code, EXIT_OK);
    assert_eq!(export_response.status, OperationStatus::Ok);
    let bundle = export_response.bundle.expect("bundle");
    assert!(bundle.bundle_id.starts_with("offline-sync-"));
    assert!(bundle.pending_audit_count >= 1);
    assert!(!bundle.pending_audit_events.is_empty());
    assert!(bundle
        .pending_audit_events
        .iter()
        .any(|event| event.event_kind == compiler::runtime_api::RuntimeAuditEventKind::DirectPush));
}

#[test]
fn run_031_malformed_state_event_bus_is_rejected_deterministically() {
    let mut malformed_snapshot = open_snapshot(&S1_SMOKE_SPEC);
    malformed_snapshot.event_bus.buffer_capacity = 0;

    let request = GetSyncStatusRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: malformed_snapshot,
    };
    let (output, response): (RunOutput, GetSyncStatusResult) =
        run_json_command(&["get-sync-status"], &request);
    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        compiler::runtime_api::E_RUNTIME_EVENT_INVALID
    );
}

#[test]
fn run_032_corrupted_persistence_audit_state_is_rejected_deterministically() {
    let mut corrupted_snapshot = open_snapshot(&S1_SMOKE_SPEC);
    corrupted_snapshot.audit_next_sequence = 0;

    let request = GetSyncStatusRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: corrupted_snapshot,
    };
    let (output, response): (RunOutput, GetSyncStatusResult) =
        run_json_command(&["get-sync-status"], &request);
    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        compiler::runtime_api::E_RUNTIME_AUDIT_INVALID
    );
}

#[test]
fn run_033_deferred_audit_replay_is_recoverable_across_reboot() {
    let path = S1_SMOKE_SPEC.mutable_path.expect("mutable path");
    let set_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: open_snapshot(&S1_SMOKE_SPEC),
        path: path.to_string(),
        value: Value::Float(0.73),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (_, set_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &set_request);

    let commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: set_response.runtime_snapshot.expect("dirty snapshot"),
        actor: "runtime-cli.audit-replay".to_string(),
        reason: Some("persist deferred upload candidate".to_string()),
        expected_base_configuration_id: None,
        changed_paths_hint: vec![path.to_string()],
    };
    let (_, commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &commit_request);
    let committed_snapshot = commit_response
        .runtime_snapshot
        .expect("committed snapshot");
    assert!(committed_snapshot.audit_events.len() >= 2);

    let offline_push_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: committed_snapshot,
        backend_connected: false,
        max_events: 64,
    };
    let (_, offline_push_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &offline_push_request);
    assert_eq!(offline_push_response.status, OperationStatus::Ok);
    assert_eq!(offline_push_response.pushed_count, 0);
    assert!(offline_push_response.pending_count >= 2);
    let expected_pending = offline_push_response.pending_count;

    let export_before_reboot_request = ExportPendingSyncBundleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: offline_push_response
            .runtime_snapshot
            .clone()
            .expect("offline snapshot"),
        max_audit_events: 64,
    };
    let (_, export_before_reboot_response): (RunOutput, ExportPendingSyncBundleResult) =
        run_json_command(
            &["export-pending-sync-bundle"],
            &export_before_reboot_request,
        );
    assert_eq!(export_before_reboot_response.status, OperationStatus::Ok);
    let bundle_before_reboot = export_before_reboot_response.bundle.expect("bundle");
    assert_eq!(bundle_before_reboot.pending_audit_count, expected_pending);

    let rebooted_snapshot = reopen_snapshot(
        &S1_SMOKE_SPEC,
        offline_push_response
            .runtime_snapshot
            .expect("offline push snapshot"),
    );
    assert_eq!(rebooted_snapshot.audit_uploaded_sequence, 0);

    let export_after_reboot_request = ExportPendingSyncBundleRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: rebooted_snapshot.clone(),
        max_audit_events: 64,
    };
    let (_, export_after_reboot_response): (RunOutput, ExportPendingSyncBundleResult) =
        run_json_command(
            &["export-pending-sync-bundle"],
            &export_after_reboot_request,
        );
    assert_eq!(export_after_reboot_response.status, OperationStatus::Ok);
    let bundle_after_reboot = export_after_reboot_response.bundle.expect("bundle");
    assert_eq!(bundle_after_reboot.pending_audit_count, expected_pending);

    let online_push_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: rebooted_snapshot,
        backend_connected: true,
        max_events: 1,
    };
    let (_, online_push_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &online_push_request);
    assert_eq!(online_push_response.status, OperationStatus::Ok);
    assert_eq!(online_push_response.pushed_count, 1);
    assert_eq!(online_push_response.pending_count, expected_pending - 1);
    assert_eq!(online_push_response.last_uploaded_sequence, 1);

    let rebooted_after_partial_upload = reopen_snapshot(
        &S1_SMOKE_SPEC,
        online_push_response
            .runtime_snapshot
            .expect("online push snapshot"),
    );
    assert_eq!(rebooted_after_partial_upload.audit_uploaded_sequence, 1);

    let drain_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: rebooted_after_partial_upload,
        backend_connected: true,
        max_events: 64,
    };
    let (_, drain_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &drain_request);
    assert_eq!(drain_response.status, OperationStatus::Ok);
    assert_eq!(drain_response.pending_count, 0);
    assert_eq!(
        drain_response.last_uploaded_sequence,
        expected_pending as u64
    );

    let idempotent_request = PushAuditEventsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: drain_response.runtime_snapshot.expect("drained snapshot"),
        backend_connected: true,
        max_events: 64,
    };
    let (_, idempotent_response): (RunOutput, PushAuditEventsResult) =
        run_json_command(&["push-audit-events"], &idempotent_request);
    assert_eq!(idempotent_response.status, OperationStatus::Ok);
    assert_eq!(idempotent_response.pushed_count, 0);
    assert_eq!(idempotent_response.pending_count, 0);
    assert_eq!(
        idempotent_response.last_uploaded_sequence,
        expected_pending as u64
    );
}

#[test]
fn run_034_cli_help_writes_usage_to_stdout() {
    let output = run_cli(&["--help"], b"");

    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("ConfigFlux runtime"));
    assert!(stdout.contains("runtime-open"));
    assert!(stdout.contains("get-sync-status"));
}

#[test]
fn run_035_cli_invalid_args_fail_closed_with_generic_transport_error() {
    let output = run_cli(&["definitely-not-a-command"], b"");

    assert_eq!(output.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains(E_RUNTIME_CLI_ARGS_INVALID));
    assert!(output.stderr.contains("Invalid CLI arguments"));
    assert!(!output.stderr.contains("definitely-not-a-command"));
}

// configflux-g3f.3 / ADR-0017 (B.1): solver-backed option validation on
// set_parameter. The runtime CLI loads the real `.ccm` sibling emitted by
// the product compile path (configflux-9hi2) through
// `solver::Session::<CuddBackend>`, re-derives the session per ADR-0017 §3,
// and rejects a constraint-violating selection write with the `E_SELECTION_*`
// family (ADR-0017 §5) while leaving the snapshot unchanged.
//
// Fixture shape: the `climate` component exposes a runtime-writable
// `cooling_brand` parameter (lifecycle=runtime, type=string) whose
// `param_key` is also a solver facet. A named constraint carries
// `cooling_brand == 'hydra' && cooling_brand != 'aeroflux'`, so the emitted
// feasible formula requires `cooling_brand.hydra` true and
// `cooling_brand.aeroflux` false. Setting the parameter to `hydra` is a
// satisfiable selection (accepted); setting it to `aeroflux` conjoins
// `cooling_brand.aeroflux` against `¬cooling_brand.aeroflux` → ⊥ → the solver
// returns `Error::Conflict`, surfaced as `E_SELECTION_CONFLICT`.
//
// ADR-0054 §8 migration: this rule used to be carried by a `policy_module`
// component whose `condition` was AND-folded into the BDD root. A component
// condition is an inclusion selector and nothing else (§3), so under §5.1 that
// shape asserts nothing and this fixture would silently stop rejecting the
// conflicting write. The facet must also be DECLARED as part of the move:
// `link_verify::validate_constraints` infers nothing from a constraint
// condition, so the `hydra`/`aeroflux` domain that used to exist only by
// inference from the very condition being migrated now has to be written down.
const SOLVER_FIXTURE_DEFS: &str = r#"{
    "package": "s_solver",
    "version": "1.0.0",
    "definitions": {
        "brand_slot": {
            "type": "string",
            "doc": "Runtime-selectable cooling brand",
            "lifecycle": "runtime",
            "safety": "q_m",
            "access": "technician"
        }
    },
    "facets": {
        "cooling_brand": {
            "values": ["hydra", "aeroflux"],
            "default": "hydra",
            "doc": "Cooling brand."
        }
    },
    "constraints": {
        "brand_guard": {
            "condition": "cooling_brand == 'hydra' && cooling_brand != 'aeroflux'",
            "doc": "Only the hydra cooling brand is permitted."
        }
    }
}"#;

const SOLVER_FIXTURE_COMPONENTS: &str = r#"{
    "package": "s_solver",
    "version": "1.0.0",
    "components": {
        "climate": {
            "type": "controller",
            "params": {
                "cooling_brand": {
                    "inherits": "brand_slot",
                    "type": "string",
                    "doc": "Runtime cooling brand selection",
                    "value": "hydra",
                    "lifecycle": "runtime",
                    "safety": "q_m",
                    "access": "technician",
                    "req_id": "req_solver_001"
                }
            }
        }
    }
}"#;

const SOLVER_FIXTURE_SCOPE: &str = "component:climate";
const SOLVER_FIXTURE_WRITE_PATH: &str = "component.climate.param.cooling_brand";

/// Compile the inline solver fixture, returning a guard that keeps the
/// emitted CMP package *and* its `.ccm` sibling on disk for the lifetime of
/// the test. The `.ccm` directory is the `ccm` sibling of the CMP output dir
/// (configflux-9hi2).
fn emitted_solver_fixture_dir() -> TempDirGuard {
    let path = unique_fixture_dir("solver");

    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![
            SourceManifestEntry {
                source_id: "scenarios/s_solver/00_definitions.toml".to_string(),
                inline_content: SOLVER_FIXTURE_DEFS.to_string(),
            },
            SourceManifestEntry {
                source_id: "scenarios/s_solver/10_components.toml".to_string(),
                inline_content: SOLVER_FIXTURE_COMPONENTS.to_string(),
            },
        ],
        output_dir: Some(path_display(&path)),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        compile_result.status,
        OperationStatus::Ok,
        "solver fixture compilation failed: {:?}",
        compile_result.verify_report.diagnostics.diagnostics
    );

    TempDirGuard {
        path,
        manifest_ref: compile_result
            .compiled_model_package_ref
            .expect("compiled_model_package_ref"),
    }
}

/// Resolve the solver fixture under the empty base selection (no choices), so
/// the runtime open envelope carries a consistent `resolve_hash` without
/// triggering the choices-based recompute, and the solver re-derivation has
/// no prior selections to replay — the constraint decision comes purely from
/// the compiled feasible formula.
fn resolve_solver_fixture_base(
    cmp_dir: &TempDirGuard,
) -> compiler::loader_api::ResolveResult {
    let handle = open_handle(cmp_dir);
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        SOLVER_FIXTURE_SCOPE.to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("selection state");

    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: SOLVER_FIXTURE_SCOPE.to_string(),
        selection_state,
    };
    let result = resolve_from_selection(request);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "solver fixture resolve failed: {:?}",
        result.diagnostics.diagnostics
    );
    result
}

#[test]
fn run_036_set_parameter_solver_option_validation() {
    let cmp_dir = emitted_solver_fixture_dir();
    let ccm_ref = cmp_dir.path.join("ccm");
    assert!(
        ccm_ref.join("ccm.manifest.json").is_file(),
        "product compile path must emit the .ccm sibling at {}",
        ccm_ref.display()
    );

    let resolve_result = resolve_solver_fixture_base(&cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();

    let (open_output, open_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &open_request);
    assert_eq!(open_output.exit_code, EXIT_OK);
    assert!(open_output.stderr.is_empty());
    assert_eq!(open_response.status, OperationStatus::Ok);
    let snapshot = open_response.runtime_snapshot.expect("runtime_snapshot");
    assert_eq!(
        snapshot.ccm_ref,
        ccm_ref.to_string_lossy(),
        "ccm_ref must round-trip onto the snapshot (configflux-9hi2)"
    );

    // Valid selection: `cooling_brand = hydra` is consistent with the
    // feasible formula, so the write succeeds through the solver gate.
    let ok_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: SOLVER_FIXTURE_WRITE_PATH.to_string(),
        value: Value::String("hydra".to_string()),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (ok_output, ok_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &ok_request);
    assert_eq!(
        ok_output.exit_code, EXIT_OK,
        "valid selection must pass the solver gate; stderr={}",
        ok_output.stderr
    );
    assert!(ok_output.stderr.is_empty());
    assert_eq!(ok_response.status, OperationStatus::Ok);
    let updated = ok_response
        .runtime_snapshot
        .expect("updated snapshot for the accepted write");
    let get_request = GetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: updated,
        path: SOLVER_FIXTURE_WRITE_PATH.to_string(),
    };
    let (_, get_response): (RunOutput, GetParameterResult) =
        run_json_command(&["get-parameter"], &get_request);
    assert_eq!(get_response.status, OperationStatus::Ok);
    assert_eq!(
        get_response.parameter.expect("parameter").value,
        Value::String("hydra".to_string())
    );

    // Constraint-violating selection: `cooling_brand = aeroflux` conjoins
    // `cooling_brand.aeroflux` against `¬cooling_brand.aeroflux` → the solver
    // returns Conflict, surfaced as the selection-family code E_SELECTION_*.
    let bad_request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: SOLVER_FIXTURE_WRITE_PATH.to_string(),
        value: Value::String("aeroflux".to_string()),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (bad_output, bad_response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &bad_request);
    assert_eq!(
        bad_output.exit_code, EXIT_COMMAND_ERROR,
        "constraint-violating selection must be rejected"
    );
    assert!(bad_output.stderr.is_empty());
    assert_eq!(bad_response.status, OperationStatus::Error);
    let code = bad_response.diagnostics.diagnostics[0].code.as_str();
    assert!(
        code.starts_with("E_SELECTION_"),
        "rejection must use the selection family (ADR-0017 §5), got '{code}'"
    );
    assert_eq!(code, "E_SELECTION_CONFLICT");
    // ADR-0017 §3/§5: on a constraint rejection the snapshot is returned
    // unchanged (here, absent — no dirty write was applied).
    assert!(
        bad_response.runtime_snapshot.is_none(),
        "rejected write must not mutate the snapshot"
    );
}

// ---------------------------------------------------------------------------
// configflux-3b5y / ADR-0031 D1–D4: runtime `explain-rejection` solver wrapper.
//
// Reuses the `s_solver` fixture from run_036: the `cooling_brand` facet is
// pinned to `hydra` and excludes `aeroflux` by the `brand_guard` policy, so
// `aeroflux` is a genuine constraint conflict (a labeled `unsat_core`), `hydra`
// is a valid option (no core), and a non-modeled `param_key` is an unknown
// facet (fail-closed exit 2, no core). The {parameter,value} -> {facet,option}
// mapping under test is `component.climate.param.cooling_brand` -> facet
// `cooling_brand`, value string -> option.
// ---------------------------------------------------------------------------

/// Open the `s_solver` fixture and return its snapshot, asserting the `.ccm`
/// round-trips. Shared setup for the explain-rejection integration tests.
fn open_solver_fixture_snapshot(cmp_dir: &TempDirGuard) -> RuntimeSnapshot {
    let ccm_ref = cmp_dir.path.join("ccm");
    assert!(
        ccm_ref.join("ccm.manifest.json").is_file(),
        "product compile path must emit the .ccm sibling at {}",
        ccm_ref.display()
    );
    let resolve_result = resolve_solver_fixture_base(cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();

    let (open_output, open_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &open_request);
    assert_eq!(open_output.exit_code, EXIT_OK, "open stderr={}", open_output.stderr);
    assert_eq!(open_response.status, OperationStatus::Ok);
    open_response.runtime_snapshot.expect("runtime_snapshot")
}

// END-TO-END core path. Exercises the real solver explain path: the runtime
// wrapper builds a fresh `Session` from the open snapshot's `.ccm`, replays the
// parameter state, calls `Session::explain_rejection`, and maps the solver's
// LabeledCore onto the compiler-side `UnsatCore` envelope. This was previously
// `#[ignore]`'d on configflux-autp, where the solver's BDD falsifying-path
// walker faulted on every real compiler-emitted `.ccm` because it accepted
// terminal child refs only in SENTINEL form, not the table-index (0=⊥/1=⊤) form
// the real oxidd/cudd serializer emits. configflux-autp has landed (the walker
// now resolves index-based terminal refs), so the exit-0/labeled-core contract
// is active again. (The LabeledCore -> UnsatCore mapping the wrapper delegates
// to is also proven independently by the unit tests `shared_conversion_*` in
// runtime/src/explain_rejection.rs, and the fail-closed/unknown-facet paths by
// run_047 below.)
#[test]
fn run_046_explain_rejection_solver_unsat_core() {
    let cmp_dir = emitted_solver_fixture_dir();
    let snapshot = open_solver_fixture_snapshot(&cmp_dir);

    // (1) A genuine constraint conflict: `cooling_brand = aeroflux` is excluded
    // by the guard, so the solver returns a labeled MUS. A rejection
    // explanation is a SUCCESS (exit 0) carrying `unsat_core` (ADR-0031 D2/D3).
    let conflict_request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: SOLVER_FIXTURE_WRITE_PATH.to_string(),
        value: Value::String("aeroflux".to_string()),
    };
    let (conflict_output, conflict_response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &conflict_request);
    assert_eq!(
        conflict_output.exit_code, EXIT_OK,
        "a rejection explanation is a success (exit 0); stderr={}",
        conflict_output.stderr
    );
    assert!(conflict_output.stderr.is_empty());
    assert_eq!(conflict_response.status, OperationStatus::Ok);
    assert_eq!(conflict_response.rejection.code, E_SELECTION_CONFLICT);
    assert_eq!(conflict_response.path, SOLVER_FIXTURE_WRITE_PATH);
    assert_eq!(conflict_response.value, Value::String("aeroflux".to_string()));

    // The labeled unsat core is present with `{facet}.{option}` names and no
    // raw BDD variable index (a bare integer) anywhere (ADR-0031 D3 invariant).
    let core = conflict_response
        .rejection
        .unsat_core
        .as_ref()
        .expect("conflict rejection must carry an unsat_core");
    assert_eq!(
        core.rejected.facet, "cooling_brand",
        "the rejected atom must name the candidate facet"
    );
    assert_eq!(core.rejected.option, "aeroflux");
    assert!(core.minimal, "M4 deletion-based extraction yields a minimal core");
    assert_eq!(core.note, "one minimal explanation; other minimal cores may exist");
    assert!(
        !core.conflicting_constraints.is_empty(),
        "a genuine conflict must name at least one conflicting constraint"
    );
    for constraint in &core.conflicting_constraints {
        assert!(
            matches!(
                constraint.kind,
                ConstraintKind::Selection | ConstraintKind::ModelRule
            ),
            "constraint kind must be a labeled selection or model rule"
        );
        for atom in &constraint.facets {
            assert!(
                !atom.facet.is_empty() && !atom.option.is_empty(),
                "every core atom must carry non-empty labeled {{facet}}.{{option}} names"
            );
            assert!(
                is_labeled_name(&atom.facet) && is_labeled_name(&atom.option),
                "core atom must be a labeled name, never a raw BDD index: \
                 {}.{}",
                atom.facet,
                atom.option
            );
        }
    }
    // Whole-JSON guard: serialize the core and assert no conflicting-constraint
    // atom is a bare integer (the configflux-osp exit-criterion invariant).
    let core_json = serde_json::to_value(core).expect("serialize core");
    assert_no_bare_integer_atoms(&core_json);

    // (2) A genuinely valid option: `cooling_brand = hydra` is consistent, so
    // there is no rejection to explain. SUCCESS (exit 0) with no core
    // (ADR-0030 D5).
    let valid_request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: SOLVER_FIXTURE_WRITE_PATH.to_string(),
        value: Value::String("hydra".to_string()),
    };
    let (valid_output, valid_response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &valid_request);
    assert_eq!(
        valid_output.exit_code, EXIT_OK,
        "explaining a valid option succeeds; stderr={}",
        valid_output.stderr
    );
    assert_eq!(valid_response.status, OperationStatus::Ok);
    assert!(
        valid_response.rejection.unsat_core.is_none(),
        "a valid option has no conflict, so no core is emitted"
    );
}

#[test]
fn run_047_explain_rejection_unknown_facet_fails_closed() {
    let cmp_dir = emitted_solver_fixture_dir();
    let snapshot = open_solver_fixture_snapshot(&cmp_dir);

    // A `component.<id>.param.<key>` path whose `param_key` is not a solver
    // facet is a division-of-labor case (ADR-0030 D5): there is no modeled
    // option to explain. It FAILS CLOSED at exit 2 with E_SELECTION_UNKNOWN_FACET
    // and no core (acceptance criterion #2).
    let unknown_request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: "component.climate.param.cooling_brand_unknown".to_string(),
        value: Value::String("aeroflux".to_string()),
    };
    let (unknown_output, unknown_response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &unknown_request);
    assert_eq!(
        unknown_output.exit_code, EXIT_COMMAND_ERROR,
        "an unknown facet must fail closed at exit 2; stderr={}",
        unknown_output.stderr
    );
    assert_eq!(unknown_response.status, OperationStatus::Error);
    assert_eq!(unknown_response.rejection.code, E_SELECTION_UNKNOWN_FACET);
    assert!(
        unknown_response.rejection.unsat_core.is_none(),
        "an unknown-facet rejection carries no core (ADR-0031 D3)"
    );
    // The error envelope carries a single matching diagnostic.
    assert_eq!(unknown_response.diagnostics.diagnostics.len(), 1);
    assert_eq!(
        unknown_response.diagnostics.diagnostics[0].code,
        E_SELECTION_UNKNOWN_FACET
    );
}

/// Whether `name` is a labeled `{facet}` or `{value}` name and not a raw BDD
/// variable index: it must contain at least one non-digit character (a bare
/// integer like `"7"` would be a raw index, which ADR-0031 D3 forbids).
fn is_labeled_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().all(|c| c.is_ascii_digit())
}

/// Walk an `unsat_core` JSON value and assert no `facet`/`option` string in any
/// `conflicting_constraints[*].facets[*]` (or `rejected`) is a bare integer —
/// the configflux-osp labeled-MUS exit-criterion invariant (ADR-0031 D3).
fn assert_no_bare_integer_atoms(core_json: &serde_json::Value) {
    let check_atom = |atom: &serde_json::Value| {
        for key in ["facet", "option"] {
            if let Some(s) = atom.get(key).and_then(|v| v.as_str()) {
                assert!(
                    is_labeled_name(s),
                    "core atom {key} must be a labeled name, never a raw index: {s:?}"
                );
            }
        }
    };
    if let Some(rejected) = core_json.get("rejected") {
        check_atom(rejected);
    }
    if let Some(constraints) = core_json
        .get("conflicting_constraints")
        .and_then(|v| v.as_array())
    {
        for constraint in constraints {
            if let Some(facets) = constraint.get("facets").and_then(|v| v.as_array()) {
                for atom in facets {
                    check_atom(atom);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// configflux-vwhj — labeled-MUS exit-criterion E2E (runtime `explain-rejection`).
//
// The configflux-osp M4 epic's HARD EXIT CRITERION on the runtime path,
// parallel to the interpreter `explain` test and the solver-level invariant
// (//solver:labeled_mus_test). Compiles the committed THREE-facet
// cross-facet-`requires` fixture (compiler/scenarios/s_labeled_mus/), opens a
// runtime snapshot carrying the prior selection `cpu=highperf`, and drives the
// runtime `explain-rejection` subcommand for `component.rig.param.cooling` =
// `air`. The candidate is in-domain but unsatisfiable under the highperf pin
// (highperf REQUIRES liquid), so the response is a SUCCESS (exit 0) carrying a
// labeled `unsat_core` in which NO facet/option position is a raw BDD variable
// index — a bare integer (ADR-0031 D3). The {parameter,value} -> {facet,option}
// mapping under test is `component.rig.param.cooling` -> facet `cooling`,
// value string -> option.
// ---------------------------------------------------------------------------

const LABELED_MUS_FIXTURE_DEFS: &str =
    include_str!("../../compiler/scenarios/s_labeled_mus/00_definitions.json");
const LABELED_MUS_FIXTURE_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s_labeled_mus/10_components.json");
const LABELED_MUS_FIXTURE_SCOPE: &str = "component:rig";
const LABELED_MUS_COOLING_WRITE_PATH: &str = "component.rig.param.cooling";

/// Compile the committed labeled-MUS fixture, returning a guard that keeps the
/// emitted CMP package *and* its `.ccm` sibling on disk for the lifetime of the
/// test (mirrors `emitted_solver_fixture_dir`).
fn emitted_labeled_mus_fixture_dir() -> TempDirGuard {
    let path = unique_fixture_dir("labeled-mus");

    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![
            SourceManifestEntry {
                source_id: "scenarios/s_labeled_mus/00_definitions.toml".to_string(),
                inline_content: LABELED_MUS_FIXTURE_DEFS.to_string(),
            },
            SourceManifestEntry {
                source_id: "scenarios/s_labeled_mus/10_components.toml".to_string(),
                inline_content: LABELED_MUS_FIXTURE_COMPONENTS.to_string(),
            },
        ],
        output_dir: Some(path_display(&path)),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        compile_result.status,
        OperationStatus::Ok,
        "labeled-MUS fixture compilation failed: {:?}",
        compile_result.verify_report.diagnostics.diagnostics
    );

    TempDirGuard {
        path,
        manifest_ref: compile_result
            .compiled_model_package_ref
            .expect("compiled_model_package_ref"),
    }
}

/// Resolve the labeled-MUS fixture under the empty base selection (the runtime
/// `apply_selection` path validates against declared param values, not the
/// solver BDD domain, so it cannot itself select the in-domain-but-not-default
/// `cpu=highperf`; the prior selection is injected into the snapshot's
/// `choices` after open instead — see `open_labeled_mus_snapshot`).
fn resolve_labeled_mus_fixture_base(
    cmp_dir: &TempDirGuard,
) -> compiler::loader_api::ResolveResult {
    let handle = open_handle(cmp_dir);
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        LABELED_MUS_FIXTURE_SCOPE.to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("selection state");
    let result = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: LABELED_MUS_FIXTURE_SCOPE.to_string(),
        selection_state,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "labeled-MUS fixture resolve failed: {:?}",
        result.diagnostics.diagnostics
    );
    result
}

/// Open the labeled-MUS fixture and return its snapshot with the prior
/// selection `cpu=highperf` injected into `choices` — the committed state the
/// runtime `explain-rejection` wrapper replays through the solver (ADR-0017 §3)
/// before deciding the candidate. The cross-facet `requires` then makes
/// `cooling=air` a genuine conflict against that replayed pin.
fn open_labeled_mus_snapshot(cmp_dir: &TempDirGuard) -> RuntimeSnapshot {
    let ccm_ref = cmp_dir.path.join("ccm");
    assert!(
        ccm_ref.join("ccm.manifest.json").is_file(),
        "product compile path must emit the .ccm sibling at {}",
        ccm_ref.display()
    );
    let resolve_result = resolve_labeled_mus_fixture_base(cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();

    let (open_output, open_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &open_request);
    assert_eq!(open_output.exit_code, EXIT_OK, "open stderr={}", open_output.stderr);
    assert_eq!(open_response.status, OperationStatus::Ok);
    let mut snapshot = open_response.runtime_snapshot.expect("runtime_snapshot");
    // Inject the prior solver selection the explain accounts for. The runtime
    // wrapper replays `snapshot.choices` via the solver (which knows the full
    // BDD option domain), so the cross-facet conflict is genuine.
    snapshot
        .choices
        .insert("cpu".to_string(), "highperf".to_string());
    snapshot
}

#[test]
fn run_048_explain_rejection_labeled_mus_has_no_raw_indices() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_snapshot(&cmp_dir);

    // A genuine cross-facet conflict: `cooling = air` is forbidden under the
    // committed `cpu = highperf` pin (highperf REQUIRES liquid). A rejection
    // explanation is a SUCCESS (exit 0) carrying a labeled `unsat_core`.
    let conflict_request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: LABELED_MUS_COOLING_WRITE_PATH.to_string(),
        value: Value::String("air".to_string()),
    };
    let (conflict_output, conflict_response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &conflict_request);
    assert_eq!(
        conflict_output.exit_code, EXIT_OK,
        "a rejection explanation is a success (exit 0); stderr={}",
        conflict_output.stderr
    );
    assert!(conflict_output.stderr.is_empty());
    assert_eq!(conflict_response.status, OperationStatus::Ok);
    assert_eq!(conflict_response.rejection.code, E_SELECTION_CONFLICT);
    assert_eq!(conflict_response.path, LABELED_MUS_COOLING_WRITE_PATH);
    assert_eq!(conflict_response.value, Value::String("air".to_string()));

    let core = conflict_response
        .rejection
        .unsat_core
        .as_ref()
        .expect("a genuine cross-facet conflict must carry an unsat_core");
    assert_eq!(core.rejected.facet, "cooling");
    assert_eq!(core.rejected.option, "air");
    assert!(core.minimal, "M4 deletion-based extraction yields a minimal core");
    assert!(
        !core.conflicting_constraints.is_empty(),
        "a genuine cross-facet conflict must name at least one conflicting constraint"
    );
    for constraint in &core.conflicting_constraints {
        for atom in &constraint.facets {
            assert!(
                is_labeled_name(&atom.facet) && is_labeled_name(&atom.option),
                "core atom must be a labeled name, never a raw BDD index: {}.{}",
                atom.facet,
                atom.option
            );
        }
    }
    // The cross-facet relation between cpu and cooling must appear in the core.
    let names_cross_facet = core.conflicting_constraints.iter().any(|c| {
        c.facets.iter().any(|f| f.facet == "cpu")
            && c.facets.iter().any(|f| f.facet == "cooling")
    });
    assert!(
        names_cross_facet,
        "the MUS must name the cross-facet cpu/cooling relation; got {:?}",
        core.conflicting_constraints
    );

    // THE INVARIANT, on the serialized JSON: no bare integer in any facet/option
    // position anywhere in the unsat_core (the configflux-osp exit criterion).
    let core_json = serde_json::to_value(core).expect("serialize core");
    assert_no_bare_integer_atoms(&core_json);

    // A genuinely valid option (`cooling = liquid`, the required one) is
    // consistent under the pin: SUCCESS (exit 0) with no core (ADR-0030 D5).
    let valid_request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: LABELED_MUS_COOLING_WRITE_PATH.to_string(),
        value: Value::String("liquid".to_string()),
    };
    let (valid_output, valid_response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &valid_request);
    assert_eq!(
        valid_output.exit_code, EXIT_OK,
        "explaining the required option succeeds; stderr={}",
        valid_output.stderr
    );
    assert_eq!(valid_response.status, OperationStatus::Ok);
    assert!(
        valid_response.rejection.unsat_core.is_none(),
        "a valid option has no conflict, so no core is emitted"
    );
}

// ---------------------------------------------------------------------------
// configflux-ykae — the two explain surfaces must explain one model one way.
//
// `cfx` and the interpreter reach explain through `session_compose::explain`;
// the runtime binary composes its own {parameter, value} envelope around the
// same solver decision. The core INSIDE both envelopes must be the same core:
// the violated `constraints:` entry named by id with its condition quoted
// (ADR-0054 §5.4), and the same wording. The runtime used to convert the
// labeled MUS with a converter of its own, which is how it came to report
// `blocked by model rule over cpu.highperf, cooling.air` for the conflict
// `cfx` reports as `blocked by constraint highperf_requires_liquid`.
// ---------------------------------------------------------------------------

/// The declared-constraint ids the compiled fixture's `.ccm` roster carries.
/// They are the ONLY ids a core may name: ADR-0054 §5.4 forbids naming a
/// synthesized cardinality conjunct as if it were authored policy.
fn declared_constraint_ids(cmp_dir: &TempDirGuard) -> Vec<String> {
    let ccm = solver::Session::<solver::CuddBackend>::load_ccm(&cmp_dir.path.join("ccm"))
        .expect("load the fixture .ccm");
    ccm.constraint_roster()
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

#[test]
fn run_051_explain_rejection_surfaces_agree_on_the_named_constraint() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_snapshot(&cmp_dir);
    // The runtime skips the facet under explanation when it replays the
    // committed choices (the candidate supersedes any prior pin on it), while
    // `session_compose` replays them all. The two are the same replay only
    // while no committed choice pins the explained facet — which is the
    // fixture's shape, asserted rather than assumed.
    assert!(
        !snapshot.choices.contains_key("cooling"),
        "fixture precondition: the committed state must not pin the explained facet"
    );

    // Surface 1 — the runtime binary's `explain-rejection`.
    let runtime_request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot.clone(),
        path: LABELED_MUS_COOLING_WRITE_PATH.to_string(),
        value: Value::String("air".to_string()),
    };
    let (runtime_output, runtime_response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &runtime_request);
    assert_eq!(
        runtime_output.exit_code, EXIT_OK,
        "a rejection explanation is a success (exit 0); stderr={}",
        runtime_output.stderr
    );
    let runtime_core = runtime_response
        .rejection
        .unsat_core
        .expect("a genuine cross-facet conflict must carry an unsat_core");

    // Surface 2 — `session_compose::explain`, the seam `cfx` and the
    // interpreter call, over the SAME model, the SAME committed choices, and
    // the SAME candidate.
    let handle = open_handle(&cmp_dir);
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        LABELED_MUS_FIXTURE_SCOPE.to_string(),
        BTreeMap::new(),
        snapshot.choices.clone(),
    )
    .expect("selection state");
    let composed = session_compose::explain(ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: LABELED_MUS_FIXTURE_SCOPE.to_string(),
        selection_state,
        rejected_option: SelectionDelta {
            facet: "cooling".to_string(),
            option: "air".to_string(),
        },
    });
    let composed_core = composed
        .rejection
        .unsat_core
        .expect("session_compose must explain the same conflict with a core");

    // The whole core, field for field: same rejected candidate, same
    // constraints in the same order, same ids, same condition text, same note.
    assert_eq!(
        runtime_core, composed_core,
        "the runtime and session_compose explain surfaces must report one core"
    );

    // ADR-0054 §5.4, the content both surfaces carry: the violated
    // `constraints:` entry is named by id and its condition is quoted.
    assert!(
        runtime_core.conflicting_constraints.iter().any(|constraint| {
            constraint.kind == ConstraintKind::ModelRule
                && constraint.constraint_id.as_deref() == Some("highperf_requires_liquid")
                && constraint.summary == "cpu != 'highperf' || cooling != 'air'"
        }),
        "the core must name the violated constraint by id and quote its condition; got {:?}",
        runtime_core.conflicting_constraints
    );

    // §5.4's hard rule: a model clause is EITHER attributed to a declared
    // constraint OR reported as the model being over-constrained. A
    // synthesized cardinality conjunct is never named as if it were policy.
    let declared = declared_constraint_ids(&cmp_dir);
    for constraint in &runtime_core.conflicting_constraints {
        if constraint.kind != ConstraintKind::ModelRule {
            continue;
        }
        match constraint.constraint_id.as_deref() {
            Some(id) => assert!(
                declared.iter().any(|declared_id| declared_id == id),
                "core named '{id}', absent from the model's declared roster {declared:?}"
            ),
            None => assert_eq!(
                constraint.summary, MODEL_OVER_CONSTRAINED_SUMMARY,
                "an unattributed model clause must report the model as over-constrained"
            ),
        }
    }

    // The human surface: one text from both cores, naming the constraint once.
    // The "blocked by" label belongs to the renderer, never to the embedded
    // gloss — a gloss that carried its own label printed it twice
    // (configflux-hdgn, fixed on the `session_compose` side only).
    let rendered = crate::explain_renderer::render_unsat_core(&runtime_core);
    assert_eq!(
        rendered,
        crate::explain_renderer::render_unsat_core(&composed_core),
        "both surfaces must render one text"
    );
    assert!(
        rendered.contains(
            "blocked by constraint highperf_requires_liquid: cpu != 'highperf' || cooling != 'air'"
        ),
        "the rendered explanation must name the violated constraint; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("blocked by model rule: blocked by model rule"),
        "the 'blocked by' label belongs to the renderer, never the gloss; got:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// configflux-jraj / ADR-0017 amendment (2026-08-03) — write-path enforcement
// over the session's total known assignment.
//
// The runtime re-derived its solver session from `snapshot.choices` alone, so a
// sibling written DURING the session was invisible to the next write's check:
// `cooling=air` then `cpu=highperf` were each accepted even though
// `highperf_requires_liquid` forbids the pair. Same for a single atomic batch
// carrying both, and the C ABI ran no check at all.
//
// These drive the committed `s_labeled_mus` fixture, which has never had write
// coverage. Its all-defaults open carries an EMPTY `choices`, so the baseline
// tier of the assignment (D2 tier 4) is what supplies the sibling values —
// asserted below rather than assumed, because a silent miss in that projection
// would make every case here pass for the wrong reason.
// ---------------------------------------------------------------------------

const LABELED_MUS_CPU_WRITE_PATH: &str = "component.rig.param.cpu";
const LABELED_MUS_PSU_WRITE_PATH: &str = "component.rig.param.psu";
const LABELED_MUS_CONSTRAINT_ID: &str = "highperf_requires_liquid";

/// Open the labeled-MUS fixture with NO prior selection injected — the
/// reproduction's starting point, and the shape `open_labeled_mus_snapshot`
/// deliberately is not (it pins `cpu=highperf` in `choices`).
fn open_labeled_mus_session(cmp_dir: &TempDirGuard) -> RuntimeSnapshot {
    let ccm_ref = cmp_dir.path.join("ccm");
    let resolve_result = resolve_labeled_mus_fixture_base(cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();

    let (open_output, open_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &open_request);
    assert_eq!(open_output.exit_code, EXIT_OK, "open stderr={}", open_output.stderr);
    let snapshot = open_response.runtime_snapshot.expect("runtime_snapshot");
    assert!(
        snapshot.choices.is_empty(),
        "fixture precondition: the all-defaults open carries no choices, which is \
         what makes the baseline tier load-bearing; got {:?}",
        snapshot.choices
    );
    assert_labeled_mus_baseline(&snapshot);
    snapshot
}

/// The baseline values the assignment's lowest tier reads, asserted rather than
/// assumed. The projection walks
/// `resolved_output[<scope root>].components[<component>].params[<key>].value`,
/// and the three key spellings in play (scope id `component:rig`, scope-root key
/// `rig`, dotted overlay path `component.rig.param.cooling`) make a silent miss
/// easy — a miss would leave the assignment empty and every case below green.
fn assert_labeled_mus_baseline(snapshot: &RuntimeSnapshot) {
    let scope = snapshot
        .resolved_output
        .get("rig")
        .expect("resolved_output must carry the 'rig' scope root");
    let params = &scope
        .components
        .get("rig")
        .expect("the 'rig' scope root must carry the 'rig' component")
        .params;
    for (key, expected) in [("cpu", "standard"), ("cooling", "liquid"), ("psu", "gold")] {
        assert_eq!(
            params.get(key).expect("resolved param").value,
            Value::String(expected.to_string()),
            "baseline value for facet-bound param '{key}'"
        );
    }
}

fn labeled_mus_write(
    snapshot: RuntimeSnapshot,
    path: &str,
    value: &str,
) -> SetParameterRequest {
    SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: path.to_string(),
        value: Value::String(value.to_string()),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    }
}

/// Drive one `set-parameter` through the CLI and require it to be accepted,
/// returning the snapshot carrying the write.
fn accept_labeled_mus_write(snapshot: RuntimeSnapshot, path: &str, value: &str) -> RuntimeSnapshot {
    let (output, response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &labeled_mus_write(snapshot, path, value));
    assert_eq!(
        output.exit_code, EXIT_OK,
        "'{path}={value}' must be accepted here; response={:?}",
        response.diagnostics.diagnostics
    );
    response.runtime_snapshot.expect("accepted write returns a snapshot")
}

/// Assert a write response is the D4 constraint rejection: exit 2,
/// `status = error`, `E_SELECTION_CONFLICT`, no snapshot.
fn assert_constraint_rejected(output: &RunOutput, response: &SetParameterResult) {
    assert_eq!(
        output.exit_code, EXIT_COMMAND_ERROR,
        "a constraint-violating write is a domain rejection (exit 2); stderr={}",
        output.stderr
    );
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code, E_SELECTION_CONFLICT,
        "rejection reuses the existing selection-conflict code (D4); got {:?}",
        response.diagnostics.diagnostics
    );
    assert!(
        response.runtime_snapshot.is_none(),
        "a rejected write changes nothing, so no snapshot is returned"
    );
}

/// Case 1: two sequential DIRTY writes. `cooling=air` alone is legal (cpu is
/// still `standard`); `cpu=highperf` on the session carrying that dirty write is
/// the reported defect — the pair violates `highperf_requires_liquid`.
#[test]
fn run_052_write_enforcement_rejects_sequential_dirty_sibling_violation() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);

    let dirty = accept_labeled_mus_write(snapshot, LABELED_MUS_COOLING_WRITE_PATH, "air");

    let (output, response): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(dirty, LABELED_MUS_CPU_WRITE_PATH, "highperf"),
    );
    assert_constraint_rejected(&output, &response);
}

/// Case 2: the same pair with the first write COMMITTED. `commit_configuration`
/// promotes the dirty entry to the committed overlay; the assignment is
/// invariant under that promotion (D6), so the second write is rejected exactly
/// as in case 1.
#[test]
fn run_053_write_enforcement_rejects_committed_sibling_violation() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);

    let dirty = accept_labeled_mus_write(snapshot, LABELED_MUS_COOLING_WRITE_PATH, "air");

    let commit_request = CommitConfigurationRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty,
        actor: "runtime-cli.test".to_string(),
        reason: Some("commit the cooling write".to_string()),
        expected_base_configuration_id: None,
        changed_paths_hint: Vec::new(),
    };
    let (commit_output, commit_response): (RunOutput, CommitConfigurationResult) =
        run_json_command(&["commit-configuration"], &commit_request);
    assert_eq!(
        commit_output.exit_code, EXIT_OK,
        "committing a legal write must succeed; {:?}",
        commit_response.diagnostics.diagnostics
    );
    let committed = commit_response
        .runtime_snapshot
        .expect("commit returns a snapshot");
    assert!(
        !committed.committed_overlay.is_empty(),
        "precondition: the commit must actually populate the committed overlay"
    );

    let (output, response): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(committed, LABELED_MUS_CPU_WRITE_PATH, "highperf"),
    );
    assert_constraint_rejected(&output, &response);
}

/// Case 3: both writes in ONE atomic batch. Each is individually valid, so the
/// old per-write loop passed them both; the batch must be checked as a whole and
/// rejected with `applied_count = 0` and BOTH paths named.
#[test]
fn run_054_write_enforcement_rejects_jointly_violating_atomic_batch() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);

    let request = SetParametersAtomicallyRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        writes: vec![
            compiler::runtime_api::AtomicParameterWrite {
                path: LABELED_MUS_COOLING_WRITE_PATH.to_string(),
                value: Value::String("air".to_string()),
            },
            compiler::runtime_api::AtomicParameterWrite {
                path: LABELED_MUS_CPU_WRITE_PATH.to_string(),
                value: Value::String("highperf".to_string()),
            },
        ],
        actor: "runtime-cli.test".to_string(),
        reason: Some("jointly violating batch".to_string()),
        expected_working_configuration_id: None,
        intent: compiler::runtime_api::OverrideIntent::default(),
    };
    let (output, response): (RunOutput, SetParametersAtomicallyResult) =
        run_json_command(&["set-parameters-atomically"], &request);

    assert_eq!(
        output.exit_code, EXIT_COMMAND_ERROR,
        "a jointly-violating batch is rejected; stderr={}",
        output.stderr
    );
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(response.diagnostics.diagnostics[0].code, E_SELECTION_CONFLICT);
    assert_eq!(response.applied_count, 0, "all writes fail or all apply");
    assert_eq!(response.dirty_generation_max, 0);
    assert!(response.runtime_snapshot.is_none());
    assert_eq!(
        response.rejected_paths,
        vec![
            LABELED_MUS_COOLING_WRITE_PATH.to_string(),
            LABELED_MUS_CPU_WRITE_PATH.to_string(),
        ],
        "both writes participate in the violation, so both are named"
    );
}

/// Case 4: the reproduction's step 4. `explain-rejection` for `cpu=highperf`
/// against a session carrying the in-session `cooling=air` write must name the
/// violated constraint, not report "no conflict to explain".
#[test]
fn run_055_write_enforcement_explain_sees_in_session_sibling_write() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);
    let dirty = accept_labeled_mus_write(snapshot, LABELED_MUS_COOLING_WRITE_PATH, "air");

    let request = RuntimeExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: dirty,
        path: LABELED_MUS_CPU_WRITE_PATH.to_string(),
        value: Value::String("highperf".to_string()),
    };
    let (output, response): (RunOutput, RuntimeExplainRejectionResult) =
        run_json_command(&["explain-rejection"], &request);

    assert_eq!(
        output.exit_code, EXIT_OK,
        "a rejection explanation is a successful query (exit 0); stderr={}",
        output.stderr
    );
    assert_eq!(response.rejection.code, E_SELECTION_CONFLICT);
    let core = response
        .rejection
        .unsat_core
        .expect("the in-session write makes this a genuine conflict with a core");
    assert!(
        core.conflicting_constraints.iter().any(|constraint| {
            constraint.constraint_id.as_deref() == Some(LABELED_MUS_CONSTRAINT_ID)
        }),
        "the core must name the violated constraint by id; got {:?}",
        core.conflicting_constraints
    );
}

/// Case 5: the rejection envelope. The structured core rides on the result under
/// `unsat_core`, and the machine-consumer attribution rides on the diagnostic's
/// `entity_path` as `constraints/<id>` (ADR-0054 §6). Doubles as the
/// serialization guard: the new field is omitted from every other outcome, so no
/// previously-recorded payload changes a byte.
#[test]
fn run_056_write_enforcement_rejection_carries_named_core_and_omits_it_otherwise() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);
    let dirty = accept_labeled_mus_write(snapshot.clone(), LABELED_MUS_COOLING_WRITE_PATH, "air");

    let (_, rejected): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(dirty, LABELED_MUS_CPU_WRITE_PATH, "highperf"),
    );
    let core = rejected
        .unsat_core
        .as_ref()
        .expect("a constraint rejection carries the shared unsat core");
    assert!(
        core.conflicting_constraints.iter().any(|constraint| {
            constraint.kind == ConstraintKind::ModelRule
                && constraint.constraint_id.as_deref() == Some(LABELED_MUS_CONSTRAINT_ID)
                && constraint.summary == "cpu != 'highperf' || cooling != 'air'"
        }),
        "the write surface must name the constraint with the same wording the \
         explain surface uses; got {:?}",
        core.conflicting_constraints
    );
    assert_eq!(
        rejected.diagnostics.diagnostics[0].entity_path.as_deref(),
        Some("constraints/highperf_requires_liquid"),
        "ADR-0054 §6: a constraint violation attributes to constraints/<id>"
    );

    // The serialization guard. `unsat_core` is `skip_serializing_if` + `None` on
    // every non-constraint outcome, so the key is absent from the wire entirely —
    // an accepted write and a non-constraint rejection serialize as they do today.
    let (_, accepted): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(snapshot.clone(), LABELED_MUS_PSU_WRITE_PATH, "bronze"),
    );
    assert_eq!(accepted.status, OperationStatus::Ok);
    let (_, unknown_path): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(snapshot, "component.rig.param.no_such_param", "air"),
    );
    assert_eq!(unknown_path.status, OperationStatus::Error);
    assert_ne!(
        unknown_path.diagnostics.diagnostics[0].code, E_SELECTION_CONFLICT,
        "precondition: this must be a NON-constraint rejection"
    );
    for (label, response) in [("accepted", &accepted), ("non-constraint", &unknown_path)] {
        assert!(response.unsat_core.is_none(), "{label}: no core is set");
        let text = serde_json::to_string(response).expect("serialize");
        assert!(
            !text.contains("unsat_core"),
            "{label}: the omitted field must not appear on the wire; got {text}"
        );
    }
}

/// Case 6: the regression guard for the bypass. The C ABI dispatched writes to
/// the raw compiler entry points with no solver check at all, so every C++/ROS2
/// SDK write was unchecked. The ABI must now reject what the CLI rejects.
#[test]
fn run_057_write_enforcement_c_abi_matches_the_cli() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let ccm_ref = cmp_dir.path.join("ccm");
    let resolve_result = resolve_labeled_mus_fixture_base(&cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();
    let open_json =
        CString::new(serde_json::to_string(&open_request).expect("serialize")).expect("cstring");

    let mut handle: *mut ConfigFluxRuntimeSessionHandle = ptr::null_mut();
    let mut open_response_json: *mut c_char = ptr::null_mut();
    let open_status = unsafe {
        configflux_runtime_session_open(open_json.as_ptr(), &mut handle, &mut open_response_json)
    };
    assert_eq!(open_status, ConfigFluxRuntimeAbiStatus::Ok);
    assert!(!handle.is_null());
    unsafe { configflux_runtime_string_free(open_response_json) };

    // The handle carries the session snapshot forward across execute calls, so
    // the second write sees the first one's overlay — the same session shape the
    // CLI reproduction builds by threading snapshots.
    let first: SetParameterResult =
        c_abi_set_parameter(handle, LABELED_MUS_COOLING_WRITE_PATH, "air");
    assert_eq!(
        first.status,
        OperationStatus::Ok,
        "the first write is legal on its own; {:?}",
        first.diagnostics.diagnostics
    );

    let second: SetParameterResult =
        c_abi_set_parameter(handle, LABELED_MUS_CPU_WRITE_PATH, "highperf");
    assert_eq!(
        second.status,
        OperationStatus::Error,
        "the ABI must reject the sibling violation the CLI rejects"
    );
    assert_eq!(second.diagnostics.diagnostics[0].code, E_SELECTION_CONFLICT);
    assert!(second.runtime_snapshot.is_none());

    assert_eq!(
        unsafe { configflux_runtime_session_close(handle) },
        ConfigFluxRuntimeAbiStatus::Ok
    );
}

/// Execute one `SetParameter` through the C ABI against the live handle. The ABI
/// injects the session snapshot into the request, so the payload carries only the
/// command-specific fields.
fn c_abi_set_parameter(
    handle: *mut ConfigFluxRuntimeSessionHandle,
    path: &str,
    value: &str,
) -> SetParameterResult {
    let payload = serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "path": path,
        "value": value,
    });
    let request = CString::new(serde_json::to_string(&payload).expect("serialize")).expect("cstring");
    let mut response_json: *mut c_char = ptr::null_mut();
    let status = unsafe {
        configflux_runtime_session_execute_json(
            handle,
            ConfigFluxRuntimeOperation::SetParameter as u32,
            request.as_ptr(),
            &mut response_json,
        )
    };
    assert_eq!(status, ConfigFluxRuntimeAbiStatus::Ok, "ABI transport status");
    let text = unsafe { take_c_string(response_json) };
    serde_json::from_str(&text).expect("set-parameter response")
}

/// Case 7: the ordering correction. The compiler's own preconditions run FIRST,
/// so a constraint-violating write to a parameter that is ALSO immutable at
/// runtime reports the lifecycle failure — not a selection conflict. The
/// fixture's params are all `lifecycle: runtime`, so the snapshot (test-owned
/// data) supplies the immutability rather than a new model.
#[test]
fn run_058_write_enforcement_lifecycle_immutability_is_reported_first() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);
    let mut dirty = accept_labeled_mus_write(snapshot, LABELED_MUS_COOLING_WRITE_PATH, "air");

    dirty
        .resolved_output
        .get_mut("rig")
        .expect("scope root")
        .components
        .get_mut("rig")
        .expect("component")
        .params
        .get_mut("cpu")
        .expect("param")
        .lifecycle = compiler::schema::Lifecycle::Startup;

    let (output, response): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(dirty, LABELED_MUS_CPU_WRITE_PATH, "highperf"),
    );
    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(
        response.diagnostics.diagnostics[0].code, E_RUNTIME_LIFECYCLE_IMMUTABLE,
        "type/limit/lifecycle failures are reported as themselves, ahead of the \
         constraint check; got {:?}",
        response.diagnostics.diagnostics
    );
    assert!(
        response.unsat_core.is_none(),
        "a non-constraint rejection carries no core"
    );
}

/// D3's partial-assignment semantics: a facet that has neither been selected nor
/// written stays FREE, so it can never cause a rejection on its own. Two shapes:
/// a legal write against the full baseline still succeeds, and a legal write
/// still succeeds when a sibling facet is genuinely absent from the assignment.
#[test]
fn run_059_write_enforcement_unset_facets_stay_free() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);

    // A legal write on top of an in-session write: `psu=bronze` is fine while cpu
    // is `standard`, so widening the replay must not turn it into a rejection.
    let dirty = accept_labeled_mus_write(snapshot.clone(), LABELED_MUS_COOLING_WRITE_PATH, "air");
    let _ = accept_labeled_mus_write(dirty, LABELED_MUS_PSU_WRITE_PATH, "bronze");

    // With `cpu` absent from the resolved output it is unset — neither chosen nor
    // written — so `cooling=air` must still be accepted: a completion exists
    // (`cpu=standard`), and the runtime must not reject on the strength of a
    // facet nothing has committed to.
    let mut without_cpu = snapshot;
    without_cpu
        .resolved_output
        .get_mut("rig")
        .expect("scope root")
        .components
        .get_mut("rig")
        .expect("component")
        .params
        .remove("cpu")
        .expect("precondition: the cpu param exists to be removed");
    let _ = accept_labeled_mus_write(without_cpu, LABELED_MUS_COOLING_WRITE_PATH, "air");
}

/// D2's skip-on-disagreement rule. `parse_param_key` maps a path to a facet by
/// last segment, which is many-to-one: two paths in different scope roots can
/// land on one facet. When their values DISAGREE the facet is omitted from the
/// assignment — treated as unset, and therefore free — so iteration order can
/// never decide whether a write is accepted. The agreeing case is asserted
/// alongside it, otherwise the disagreeing case would pass just as well if the
/// second scope root were ignored entirely.
#[test]
fn run_060_write_enforcement_disagreeing_paths_leave_the_facet_free() {
    let cmp_dir = emitted_labeled_mus_fixture_dir();
    let snapshot = open_labeled_mus_session(&cmp_dir);

    // Both scope roots say `cooling = air`: they agree, the agreed value is used,
    // and `cpu = highperf` is rejected. This proves the projection really reads
    // the second scope root.
    let agreeing = with_second_cooling_scope(&snapshot, "air", "air");
    let (agree_output, agree_response): (RunOutput, SetParameterResult) = run_json_command(
        &["set-parameter"],
        &labeled_mus_write(agreeing, LABELED_MUS_CPU_WRITE_PATH, "highperf"),
    );
    assert_constraint_rejected(&agree_output, &agree_response);

    // The same two paths now disagree (`liquid` vs `air`). The facet is omitted,
    // so `cpu = highperf` is accepted. A silent last-wins would pick `air` here
    // and reject.
    let disagreeing = with_second_cooling_scope(&snapshot, "liquid", "air");
    let _ = accept_labeled_mus_write(disagreeing, LABELED_MUS_CPU_WRITE_PATH, "highperf");
}

/// A snapshot carrying a SECOND scope root whose component also has a `cooling`
/// param, so two distinct paths map to the one `cooling` facet. The component id
/// differs from `rig` so the written path still resolves to exactly one scope
/// root; only the facet projection sees both.
fn with_second_cooling_scope(
    snapshot: &RuntimeSnapshot,
    first_cooling: &str,
    second_cooling: &str,
) -> RuntimeSnapshot {
    let mut snapshot = snapshot.clone();
    let mut second = snapshot
        .resolved_output
        .get("rig")
        .expect("scope root")
        .clone();
    let mut component = second.components.remove("rig").expect("component");
    component
        .params
        .get_mut("cooling")
        .expect("cooling param")
        .value = Value::String(second_cooling.to_string());
    second.components.insert("spare_rig".to_string(), component);
    snapshot.resolved_output.insert("spare".to_string(), second);

    snapshot
        .resolved_output
        .get_mut("rig")
        .expect("scope root")
        .components
        .get_mut("rig")
        .expect("component")
        .params
        .get_mut("cooling")
        .expect("cooling param")
        .value = Value::String(first_cooling.to_string());
    snapshot
}

// ---------------------------------------------------------------------------
// ADR-0030 (configflux-dj7f) D2/D4: runtime-open CCM precondition + fail-closed.
//
// `runtime-open` now enforces that a usable `.ccm` solver model is reachable
// (new behavior — pre-ADR-0030 it never touched the `.ccm`). When the
// snapshot's `ccm_ref` is empty, unloadable, or symbol-less, the open FAILS
// CLOSED with `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` and returns no snapshot.
// These tests build a valid open request from the solver fixture (which emits a
// real `.ccm`) and then clobber `ccm_ref` to assert the precondition.
// ---------------------------------------------------------------------------

/// D2: `runtime-open` fails closed with `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`
/// when the snapshot's `ccm_ref` is empty or unloadable, even though every
/// other part of the open envelope is valid.
#[test]
fn run_037_runtime_open_fails_closed_without_usable_ccm() {
    let cmp_dir = emitted_solver_fixture_dir();
    let resolve_result = resolve_solver_fixture_base(&cmp_dir);

    for bogus in ["", "/nonexistent/configflux/dj7f/runtime/ccm"] {
        let mut open_request = runtime_open_request_from_resolve(&resolve_result);
        open_request.ccm_ref = bogus.to_string();

        let (output, response): (RunOutput, RuntimeOpenResult) =
            run_json_command(&["runtime-open"], &open_request);
        assert_eq!(
            output.exit_code, EXIT_COMMAND_ERROR,
            "runtime-open must fail closed (exit code) for ccm_ref {bogus:?}"
        );
        assert_eq!(response.status, OperationStatus::Error);
        assert!(
            response.runtime_snapshot.is_none(),
            "fail-closed runtime-open must not return a snapshot for {bogus:?}"
        );
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE,
            "runtime-open must surface the open model-unavailable code for {bogus:?}"
        );
    }
}

/// D2 positive: `runtime-open` succeeds when the snapshot carries a usable
/// `.ccm` (the solver fixture's real emitted sibling). Confirms the precondition
/// is a gate, not a blanket rejection.
#[test]
fn run_038_runtime_open_succeeds_with_usable_ccm() {
    let cmp_dir = emitted_solver_fixture_dir();
    let ccm_ref = cmp_dir.path.join("ccm");
    assert!(
        ccm_ref.join("ccm.manifest.json").is_file(),
        "solver fixture must emit a usable .ccm sibling at {}",
        ccm_ref.display()
    );
    let resolve_result = resolve_solver_fixture_base(&cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();

    let (output, response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &open_request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    let snapshot = response.runtime_snapshot.expect("runtime_snapshot");
    assert_eq!(
        snapshot.ccm_ref,
        ccm_ref.to_string_lossy(),
        "the usable ccm_ref must round-trip onto the opened snapshot"
    );
}

/// D2 helper unit test: `ccm_usable_for_open` is the precondition predicate.
/// It is `false` for an empty reference and an unloadable path, and `true` for
/// the solver fixture's real `.ccm` (loadable, populated symbol table).
#[test]
fn run_039_ccm_usable_for_open_predicate() {
    use crate::solver_validation::ccm_usable_for_open;
    assert!(!ccm_usable_for_open(""), "empty ref is not usable");
    assert!(!ccm_usable_for_open("   "), "whitespace ref is not usable");
    assert!(
        !ccm_usable_for_open("/nonexistent/configflux/dj7f/ccm"),
        "unloadable path is not usable"
    );

    let cmp_dir = emitted_solver_fixture_dir();
    let ccm_ref = cmp_dir.path.join("ccm");
    assert!(
        ccm_usable_for_open(&ccm_ref.to_string_lossy()),
        "the solver fixture's emitted .ccm must be usable"
    );
}

/// Determinism guard (configflux-6gzn). The historical flake was a temp-dir
/// collision: `emitted_solver_fixture_dir` named its output `solver-<pid>-<nanos>`
/// and many `#[test]` threads compile the byte-identical fixture (same file
/// names). `<pid>` is constant and `<nanos>` repeats under a coarse clock on a
/// loaded gate host, so two concurrent builds collided on one directory and
/// truncated each other's files; a reader then saw a half-written index/chunk and
/// `open_model` returned `status=Error` (the `open_handle` panic). The fix adds a
/// per-process monotonic `seq` to the name.
///
/// This guard is deterministic, not probabilistic: it holds `label`, `pid`,
/// `thread`, and `nanos` identical (the worst case the old scheme could not
/// survive) and requires names differing only in `seq` to stay distinct. Drop
/// `seq` from the name and it fails every run — so the flake cannot silently
/// return. The live compile+open path under contention is covered by running the
/// suite with `--runs_per_test` under `--nocache_test_results`.
#[test]
fn run_040b_fixture_dir_name_is_collision_free_without_a_clock() {
    use std::collections::BTreeSet;

    const NANOS: u128 = 1_700_000_000_000_000_000; // frozen: clock contributes nothing
    let thread = thread_token();
    let names: BTreeSet<String> = (0..10_000)
        .map(|seq| fixture_dir_name("solver", seq, &thread, NANOS))
        .collect();

    assert_eq!(
        names.len(),
        10_000,
        "fixture dir names must be unique from the monotonic seq alone, even when \
         the label, pid, thread, and clock reading are all identical"
    );
}

/// D4: a solver fault on a modeled `set-parameter` write FAILS CLOSED with the
/// internal-fault code, instead of being silently skipped (ADR-0017 §5's
/// retired "backend faults defer to the compiler" rule). The fault is induced
/// deterministically: the snapshot's `ccm_ref` points at a directory that has a
/// `ccm.manifest.json` (so it resolves as a CCM dir) but lacks the v2
/// `partition-manifest.json`, which makes `Session::load_ccm` return an error
/// rather than the empty-stub fallback. The write path is a real facet path, so
/// the load error occurs on a solver-owned query.
#[test]
fn run_040_set_parameter_fails_closed_on_solver_load_fault() {
    // Open a valid snapshot first, then redirect its ccm_ref at a malformed
    // CCM directory to force the load-fault arm.
    let cmp_dir = emitted_solver_fixture_dir();
    let ccm_ref = cmp_dir.path.join("ccm");
    let resolve_result = resolve_solver_fixture_base(&cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();
    let (_, open_response): (RunOutput, RuntimeOpenResult) =
        run_json_command(&["runtime-open"], &open_request);
    let mut snapshot = open_response.runtime_snapshot.expect("runtime_snapshot");

    // Build a malformed CCM dir: a manifest is present (so it is recognized as
    // a CCM directory) but the v2 partition manifest is absent (so the load
    // errors instead of falling back to the empty stub).
    let malformed = unique_fixture_dir("malformed-ccm");
    std::fs::write(malformed.join("ccm.manifest.json"), b"{\"not\":\"a real ccm\"}")
        .expect("write malformed manifest");
    let malformed_guard = TempDirGuard {
        path: malformed.clone(),
        manifest_ref: String::new(),
    };
    snapshot.ccm_ref = malformed.to_string_lossy().into_owned();

    let request = SetParameterRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        runtime_snapshot: snapshot,
        path: SOLVER_FIXTURE_WRITE_PATH.to_string(),
        value: Value::String("hydra".to_string()),
        intent: compiler::runtime_api::OverrideIntent::default(),
        actor: None,
        reason: None,
    };
    let (output, response): (RunOutput, SetParameterResult) =
        run_json_command(&["set-parameter"], &request);
    assert_eq!(
        output.exit_code, EXIT_COMMAND_ERROR,
        "a solver load fault on a modeled write must fail closed"
    );
    assert_eq!(response.status, OperationStatus::Error);
    assert!(
        response.runtime_snapshot.is_none(),
        "a fault-rejected write must not mutate the snapshot"
    );
    assert_eq!(
        response.diagnostics.diagnostics[0].code, E_SELECTION_ENGINE_DIVERGENCE,
        "the solver fault must surface the internal-fault code, not be skipped"
    );

    drop(malformed_guard);
}

// ---------------------------------------------------------------------------
// Runtime C ABI (`crate::runtime_c_abi`) — configflux-u32v.
//
// The C ABI moved out of the compiler crate into the runtime crate so its open
// entrypoint enforces the ADR-0030 D2 `.ccm` solver-model precondition (the
// compiler may not import `solver`, ADR-0003 §2). These tests drive the
// `#[no_mangle] extern "C"` symbols directly. The fail-closed test
// (`run_041`) is the regression that pins the gap: an SDK-driven open with an
// unusable `.ccm` must be refused, exactly as the CLI handler refuses it. The
// round-trip and boundary tests (`run_042`/`run_043`/`run_044`) preserve the
// FFI coverage that previously lived in compiler/src/runtime_c_abi.rs
// (REQ-RUN-033/034/035), now exercising the real solver fixture so the
// happy-path open carries a usable `.ccm`.
// ---------------------------------------------------------------------------

use crate::runtime_c_abi::{
    configflux_runtime_abi_handshake, configflux_runtime_session_close,
    configflux_runtime_session_execute_json, configflux_runtime_session_open,
    configflux_runtime_session_snapshot_json, configflux_runtime_string_free,
    ConfigFluxRuntimeAbiStatus, ConfigFluxRuntimeAbiVersion, ConfigFluxRuntimeOperation,
    ConfigFluxRuntimeSessionHandle, CONFIGFLUX_RUNTIME_C_ABI_VERSION_MAJOR,
    CONFIGFLUX_RUNTIME_C_ABI_VERSION_MINOR,
};
use std::ffi::{c_char, CStr, CString};
use std::ptr;

/// Build a `runtime-open` request JSON for the solver fixture whose `ccm_ref`
/// points at the real emitted `.ccm` directory (so the ADR-0030 D2 precondition
/// is satisfied). Returns the JSON plus the fixture guard, which the caller must
/// keep alive for the duration of the test so the `.ccm` stays on disk.
fn usable_solver_open_request_json() -> (CString, TempDirGuard) {
    let cmp_dir = emitted_solver_fixture_dir();
    let ccm_ref = cmp_dir.path.join("ccm");
    assert!(
        ccm_ref.join("ccm.manifest.json").is_file(),
        "solver fixture must emit a usable .ccm sibling at {}",
        ccm_ref.display()
    );
    let resolve_result = resolve_solver_fixture_base(&cmp_dir);
    let mut open_request = runtime_open_request_from_resolve(&resolve_result);
    open_request.ccm_ref = ccm_ref.to_string_lossy().into_owned();
    let json = CString::new(serde_json::to_string(&open_request).expect("serialize open request"))
        .expect("cstring");
    (json, cmp_dir)
}

/// Take ownership of an ABI-allocated C string, copy it to a Rust `String`, and
/// free it through the ABI's deallocator (matching the ownership contract).
unsafe fn take_c_string(ptr: *mut c_char) -> String {
    let value = CStr::from_ptr(ptr).to_str().expect("utf8").to_string();
    configflux_runtime_string_free(ptr);
    value
}

/// configflux-u32v / ADR-0030 D2 (the bug this task fixes): an open driven
/// through the C ABI with an empty or unloadable `ccm_ref` must FAIL CLOSED with
/// `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` and yield NO session handle — the
/// same fail-closed contract the runtime CLI handler enforces. Before the fix,
/// `configflux_runtime_session_open` called bare `runtime_open` and accepted the
/// snapshot, bypassing the precondition for SDK (C++/ROS2) callers.
#[test]
fn run_041_ffi_open_fails_closed_without_usable_ccm() {
    let cmp_dir = emitted_solver_fixture_dir();
    let resolve_result = resolve_solver_fixture_base(&cmp_dir);

    for bogus in ["", "/nonexistent/configflux/u32v/runtime/ccm"] {
        let mut open_request = runtime_open_request_from_resolve(&resolve_result);
        open_request.ccm_ref = bogus.to_string();
        let request_json =
            CString::new(serde_json::to_string(&open_request).expect("serialize request"))
                .expect("cstring");

        let mut handle: *mut ConfigFluxRuntimeSessionHandle = ptr::null_mut();
        let mut response_json: *mut c_char = ptr::null_mut();
        let status = unsafe {
            configflux_runtime_session_open(request_json.as_ptr(), &mut handle, &mut response_json)
        };

        // Boundary status is Ok (the call itself is well-formed); the rejection
        // is a runtime-domain error carried in the response envelope.
        assert_eq!(
            status,
            ConfigFluxRuntimeAbiStatus::Ok,
            "boundary status must be Ok for {bogus:?}; the rejection is in the envelope"
        );
        assert!(
            handle.is_null(),
            "fail-closed FFI open must NOT return a session handle for {bogus:?}"
        );
        assert!(!response_json.is_null());
        let response_text = unsafe { take_c_string(response_json) };
        let response: RuntimeOpenResult =
            serde_json::from_str(&response_text).expect("open response");
        assert_eq!(
            response.status,
            OperationStatus::Error,
            "FFI open must fail closed for {bogus:?}"
        );
        assert!(
            response.runtime_snapshot.is_none(),
            "fail-closed FFI open must not return a snapshot for {bogus:?}"
        );
        assert_eq!(
            response.diagnostics.diagnostics[0].code, E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE,
            "FFI open must surface the open model-unavailable code for {bogus:?}"
        );
    }
}

/// REQ-RUN-033: full FFI lifecycle — open (with a usable `.ccm`), execute a
/// read op, snapshot, close — round-trips successfully. The positive companion
/// to `run_041`: a usable `.ccm` opens cleanly and returns a live handle.
#[test]
fn run_042_ffi_open_execute_snapshot_and_close_round_trip() {
    let (request_json, _fixture) = usable_solver_open_request_json();

    let mut handle: *mut ConfigFluxRuntimeSessionHandle = ptr::null_mut();
    let mut response_json: *mut c_char = ptr::null_mut();
    let status = unsafe {
        configflux_runtime_session_open(request_json.as_ptr(), &mut handle, &mut response_json)
    };
    assert_eq!(status, ConfigFluxRuntimeAbiStatus::Ok);
    assert!(!response_json.is_null());
    let open_response_text = unsafe { take_c_string(response_json) };
    let open_response: RuntimeOpenResult =
        serde_json::from_str(&open_response_text).expect("open response");
    assert_eq!(
        open_response.status,
        OperationStatus::Ok,
        "a usable .ccm must open cleanly through the FFI"
    );
    assert!(!handle.is_null());

    let execute_request = CString::new(r#"{"schema_version":4}"#).expect("cstring");
    let mut execute_response_json: *mut c_char = ptr::null_mut();
    let execute_status = unsafe {
        configflux_runtime_session_execute_json(
            handle,
            ConfigFluxRuntimeOperation::GetSyncStatus as u32,
            execute_request.as_ptr(),
            &mut execute_response_json,
        )
    };
    assert_eq!(execute_status, ConfigFluxRuntimeAbiStatus::Ok);
    let execute_response_text = unsafe { take_c_string(execute_response_json) };
    let execute_response: compiler::runtime_api::GetSyncStatusResult =
        serde_json::from_str(&execute_response_text).expect("execute response");
    assert_eq!(execute_response.status, OperationStatus::Ok);

    let mut snapshot_json: *mut c_char = ptr::null_mut();
    let snapshot_status =
        unsafe { configflux_runtime_session_snapshot_json(handle, &mut snapshot_json) };
    assert_eq!(snapshot_status, ConfigFluxRuntimeAbiStatus::Ok);
    let snapshot_text = unsafe { take_c_string(snapshot_json) };
    let _snapshot: RuntimeSnapshot = serde_json::from_str(&snapshot_text).expect("snapshot");

    let close_status = unsafe { configflux_runtime_session_close(handle) };
    assert_eq!(close_status, ConfigFluxRuntimeAbiStatus::Ok);
}

/// REQ-RUN-034: the FFI execute path rejects an unknown operation code
/// deterministically, leaving the response pointer null, and the session
/// remains valid for a clean close.
#[test]
fn run_043_ffi_execute_rejects_unknown_operation() {
    let (request_json, _fixture) = usable_solver_open_request_json();

    let mut handle: *mut ConfigFluxRuntimeSessionHandle = ptr::null_mut();
    let mut response_json: *mut c_char = ptr::null_mut();
    let status = unsafe {
        configflux_runtime_session_open(request_json.as_ptr(), &mut handle, &mut response_json)
    };
    assert_eq!(status, ConfigFluxRuntimeAbiStatus::Ok);
    assert!(!handle.is_null());
    unsafe {
        configflux_runtime_string_free(response_json);
    }

    let execute_request = CString::new(r#"{"schema_version":4}"#).expect("cstring");
    let mut execute_response_json: *mut c_char = ptr::null_mut();
    let execute_status = unsafe {
        configflux_runtime_session_execute_json(
            handle,
            99,
            execute_request.as_ptr(),
            &mut execute_response_json,
        )
    };
    assert_eq!(execute_status, ConfigFluxRuntimeAbiStatus::UnsupportedOperation);
    assert!(execute_response_json.is_null());

    let close_status = unsafe { configflux_runtime_session_close(handle) };
    assert_eq!(close_status, ConfigFluxRuntimeAbiStatus::Ok);
}

/// REQ-RUN-035: malformed open-request JSON is mapped to the deterministic
/// `InvalidJson` boundary status with no handle and no response allocated.
#[test]
fn run_044_ffi_invalid_json_is_mapped_deterministically() {
    let request_json = CString::new("{not json}").expect("cstring");
    let mut handle: *mut ConfigFluxRuntimeSessionHandle = ptr::null_mut();
    let mut response_json: *mut c_char = ptr::null_mut();
    let status = unsafe {
        configflux_runtime_session_open(request_json.as_ptr(), &mut handle, &mut response_json)
    };
    assert_eq!(status, ConfigFluxRuntimeAbiStatus::InvalidJson);
    assert!(handle.is_null());
    assert!(response_json.is_null());
}

/// The handshake rejects a major-version mismatch and reports the current ABI
/// version (now 1.1 after configflux-u32v). The minor bump keeps existing
/// `expected_minor = 0` clients compatible (`expected_minor <= ABI minor`).
#[test]
fn run_045_ffi_handshake_version_contract() {
    let mut version = ConfigFluxRuntimeAbiVersion {
        major: 0,
        minor: 0,
        patch: 0,
    };
    let status = unsafe { configflux_runtime_abi_handshake(99, 0, &mut version) };
    assert_eq!(status, ConfigFluxRuntimeAbiStatus::VersionMismatch);
    assert_eq!(version.major, CONFIGFLUX_RUNTIME_C_ABI_VERSION_MAJOR);
    assert_eq!(version.minor, CONFIGFLUX_RUNTIME_C_ABI_VERSION_MINOR);

    // A current-major, minor=0 client still handshakes Ok against ABI 1.1.
    let mut v2 = ConfigFluxRuntimeAbiVersion {
        major: 0,
        minor: 0,
        patch: 0,
    };
    let ok = unsafe {
        configflux_runtime_abi_handshake(CONFIGFLUX_RUNTIME_C_ABI_VERSION_MAJOR, 0, &mut v2)
    };
    assert_eq!(
        ok,
        ConfigFluxRuntimeAbiStatus::Ok,
        "an expected_minor=0 client must remain compatible with ABI 1.1"
    );
}

/// configflux-jlrm (cold-eval F4): a request envelope that omits a required
/// field must NAME that field in the transport diagnostic instead of reporting
/// a generic "malformed envelope". serde reports the first missing struct
/// field; the CLI surfaces it under `E_RUNTIME_CLI_REQUEST_INVALID`.
#[test]
fn run_049_transport_request_invalid_names_offending_field() {
    // Top-level `schema_version` omitted.
    let missing_schema = run_cli(&["get-parameter"], br#"{}"#);
    assert_eq!(missing_schema.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(missing_schema.stdout.is_empty());
    assert!(missing_schema.stderr.contains(E_RUNTIME_CLI_REQUEST_INVALID));
    assert!(
        missing_schema.stderr.contains("schema_version"),
        "diagnostic must name the missing field, got: {}",
        missing_schema.stderr
    );

    // Required `runtime_snapshot` omitted (schema_version supplied).
    let missing_snapshot = run_cli(&["get-parameter"], br#"{"schema_version":1}"#);
    assert_eq!(missing_snapshot.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(
        missing_snapshot.stderr.contains("runtime_snapshot"),
        "diagnostic must name the missing field, got: {}",
        missing_snapshot.stderr
    );

    // The cold-eval scenario: a valid snapshot is threaded but the
    // command-specific selector `scope_root` is omitted (the caller wrote
    // `scope`, an ignored unknown field, instead — see the service integration
    // guide's troubleshooting note).
    let payload = serde_json::json!({
        "schema_version": 1,
        "runtime_snapshot": {
            "schema_version": 1,
            "model_hash": "m",
            "resolve_hash": "r",
            "scope": "component:runtime_tuner",
            "resolved_output": {}
        },
        "scope": "runtime_tuner"
    });
    let missing_scope_root = run_cli(
        &["get-scope-metadata"],
        &serde_json::to_vec(&payload).expect("serialize request"),
    );
    assert_eq!(missing_scope_root.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(
        missing_scope_root.stderr.contains("scope_root"),
        "diagnostic must name the missing field, got: {}",
        missing_scope_root.stderr
    );
}

/// configflux-jlrm: naming the offending field must not regress the
/// no-request-payload-secret-echo transport invariant
/// (docs/runtime-cli-contract.md §7). A type-mismatched field carries its value
/// in serde's own message, so the diagnostic must report the failure by
/// category + location only — never by echoing the field value.
#[test]
fn run_050_transport_request_invalid_does_not_echo_field_values() {
    // `schema_version` expects an integer; a secret-bearing string triggers a
    // serde type error whose Display contains the value. It must not surface.
    let payload = br#"{"schema_version":"sk_live_should_not_leak"}"#;
    let output = run_cli(&["get-parameter"], payload);

    assert_eq!(output.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains(E_RUNTIME_CLI_REQUEST_INVALID));
    assert!(
        !output.stderr.contains("sk_live_should_not_leak"),
        "diagnostic must not echo request field values, got: {}",
        output.stderr
    );
}
