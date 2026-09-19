// SPDX-License-Identifier: BUSL-1.1

use super::*;
use compiler::loader_api::{
    canonical_selection_state, ApplySelectionRequest, ConstraintKind,
    InitializeSelectionStateRequest, InitializeSelectionStateResult, ModelHandle, OpenModelRequest,
    SelectionDelta, SelectionState, SoftwareBomV1, UnsatCore, EXPORT_PROFILE_CPP_EARLY_BINDING_V1,
    EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT, E_EXPORT_PROFILE_INVALID, E_LOADER_INDEX_INVALID,
    E_RESOLVE_CONTEXT_UNSATISFIED, E_RESOLVE_SOLVER_MODEL_UNAVAILABLE, E_SBOM_PROFILE_INVALID,
    E_SELECTION_CONFLICT, E_SELECTION_INVALID_OPTION, E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
    E_SELECTION_STATE_INVALID, E_SELECTION_UNKNOWN_FACET, E_SELECTION_UNSATISFIABLE,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, DiagnosticSeverity, DiagnosticsReport, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::io::Cursor;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
// E-1c (ADR-0027, configflux-zxni): these were inline TOML chunk literals with
// un-resolved `inherits` that relied on the resolve-time gap-fill deleted in
// B-5. They now point at the existing inheritance-resolved CUE-JSON siblings
// (the same migration E-1b applied to S2/S4/S5), fed through `add_chunk_auto`
// (JSON). The `*_SOURCE_*` source_id strings are preserved byte-for-byte (they
// fold into model_hash), and the resolved model is identical, so the behavior
// assertions below pass unchanged — the proof the swap is content-preserving.
const S1_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s1_water_pump/smoke/cue/10_components.json");
const S1_SCOPE: &str = "component:thermal_control";

const S3_SOURCE_DEFS: &str = "scenarios/s3_automation_cell/smoke/chunks/00_definitions.toml";
const S3_SOURCE_COMPONENTS: &str = "scenarios/s3_automation_cell/smoke/chunks/10_components.toml";
// E-1c (ADR-0027, configflux-zxni): migrated from inline TOML to the
// inheritance-resolved CUE-JSON sibling; source_id strings preserved.
const S3_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s3_automation_cell/smoke/cue/00_definitions.json");
const S3_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s3_automation_cell/smoke/cue/10_components.json");
const S3_SCOPE: &str = "component:swift_ring_standard";

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
const S2_SCOPE: &str = "component:turbine_controller";

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
const S4_SCOPE: &str = "component:robot_platform";

const S5_SMOKE_SOURCE_DEFS: &str = "scenarios/s5_building_hvac/smoke/chunks/00_definitions.toml";
const S5_SMOKE_SOURCE_COMPONENTS: &str =
    "scenarios/s5_building_hvac/smoke/chunks/10_components.toml";
const S5_SMOKE_CHUNK_DEFS: &str =
    include_str!("../../compiler/scenarios/s5_building_hvac/smoke/cue/00_definitions.json");
const S5_SMOKE_CHUNK_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s5_building_hvac/smoke/cue/10_components.json");
const S5_SCOPE: &str = "component:climate_controller";

struct TempDirGuard {
    path: PathBuf,
    manifest_ref: String,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

struct RunOutput {
    exit_code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

fn run_cli(args: &[&str], stdin_payload: &[u8]) -> RunOutput {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("configflux-interpreter");
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

fn assert_transport_stderr_redacted(stderr: &str) {
    let lowered = stderr.to_lowercase();
    assert!(!lowered.contains("panic"));
    assert!(!lowered.contains("stack backtrace"));
}

fn assert_transport_failure(output: &RunOutput, code: &str) {
    assert_eq!(output.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains(code));
    assert_transport_stderr_redacted(&output.stderr);
}

const S2_SELECTION_STEPS: &[(&str, &str)] =
    &[("gearbox_type", "direct_drive"), ("grid_code", "iec_61400")];
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

struct InterpreterFixture {
    name: &'static str,
    source_defs: &'static str,
    source_components: &'static str,
    chunk_defs: &'static str,
    chunk_components: &'static str,
    scope: &'static str,
    context_tags: &'static [(&'static str, &'static str)],
    selection_steps: &'static [(&'static str, &'static str)],
}

const S2_SMOKE_FIXTURE: InterpreterFixture = InterpreterFixture {
    name: "s2-smoke",
    source_defs: S2_SMOKE_SOURCE_DEFS,
    source_components: S2_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S2_SMOKE_CHUNK_DEFS,
    chunk_components: S2_SMOKE_CHUNK_COMPONENTS,
    scope: S2_SCOPE,
    context_tags: &[],
    selection_steps: S2_SELECTION_STEPS,
};

const S2_MEDIUM_FIXTURE: InterpreterFixture = InterpreterFixture {
    name: "s2-medium",
    source_defs: S2_MEDIUM_SOURCE_DEFS,
    source_components: S2_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S2_MEDIUM_CHUNK_DEFS,
    chunk_components: S2_MEDIUM_CHUNK_COMPONENTS,
    scope: S2_SCOPE,
    context_tags: &[],
    selection_steps: S2_SELECTION_STEPS,
};

const S4_SMOKE_FIXTURE: InterpreterFixture = InterpreterFixture {
    name: "s4-smoke",
    source_defs: S4_SMOKE_SOURCE_DEFS,
    source_components: S4_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S4_SMOKE_CHUNK_DEFS,
    chunk_components: S4_SMOKE_CHUNK_COMPONENTS,
    scope: S4_SCOPE,
    context_tags: &[],
    selection_steps: S4_SELECTION_STEPS,
};

const S4_MEDIUM_FIXTURE: InterpreterFixture = InterpreterFixture {
    name: "s4-medium",
    source_defs: S4_MEDIUM_SOURCE_DEFS,
    source_components: S4_MEDIUM_SOURCE_COMPONENTS,
    chunk_defs: S4_MEDIUM_CHUNK_DEFS,
    chunk_components: S4_MEDIUM_CHUNK_COMPONENTS,
    scope: S4_SCOPE,
    context_tags: &[],
    selection_steps: S4_SELECTION_STEPS,
};

const S5_SMOKE_FIXTURE: InterpreterFixture = InterpreterFixture {
    name: "s5-smoke",
    source_defs: S5_SMOKE_SOURCE_DEFS,
    source_components: S5_SMOKE_SOURCE_COMPONENTS,
    chunk_defs: S5_SMOKE_CHUNK_DEFS,
    chunk_components: S5_SMOKE_CHUNK_COMPONENTS,
    scope: S5_SCOPE,
    context_tags: &[],
    selection_steps: S5_SELECTION_STEPS,
};

fn init_selection_state_for_fixture(
    handle: &ModelHandle,
    scope: &str,
    context_tags: BTreeMap<String, String>,
) -> SelectionState {
    let request = InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        context_tags,
    };
    let (output, response): (RunOutput, InitializeSelectionStateResult) =
        run_json_command(&["init-selection-state"], &request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    response.selection_state.expect("selection_state")
}

/// Per-process monotonic counter for fixture dir names — unique across threads
/// regardless of clock resolution. Ported from `runtime/src/tests.rs`
/// (configflux-6gzn): the bare `<pid>-<nanos>` name collided when `<nanos>`
/// repeated under a coarse clock on a loaded gate host, and two parallel
/// `#[test]` threads truncated each other's fixture files (configflux-xowl.3).
static FIXTURE_DIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn thread_token() -> String {
    format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Build and create a collision-proof temp dir: the atomic `seq` alone keeps
/// names distinct within the process; pid separates processes; `nanos` is kept
/// only for triage readability.
fn unique_fixture_dir(prefix: &str, label: &str) -> std::path::PathBuf {
    let seq = FIXTURE_DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("duration")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{label}-{}-{}-{seq}-{nanos}",
        std::process::id(),
        thread_token()
    ));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn emitted_cmp_dir_with_chunks(
    label: &str,
    source_defs: &str,
    source_components: &str,
    chunk_defs: &str,
    chunk_components: &str,
) -> TempDirGuard {
    let path = unique_fixture_dir("configflux-interpreter", label);

    let compile_result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![
            SourceManifestEntry {
                source_id: source_defs.to_string(),
                inline_content: chunk_defs.to_string(),
            },
            SourceManifestEntry {
                source_id: source_components.to_string(),
                inline_content: chunk_components.to_string(),
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
    let manifest_ref = compile_result
        .compiled_model_package_ref
        .expect("compiled_model_package_ref");
    TempDirGuard { path, manifest_ref }
}

fn emitted_cmp_dir(label: &str) -> TempDirGuard {
    emitted_cmp_dir_with_chunks(
        label,
        S1_SOURCE_DEFS,
        S1_SOURCE_COMPONENTS,
        S1_CHUNK_DEFS,
        S1_CHUNK_COMPONENTS,
    )
}

fn open_handle(cmp_dir: &TempDirGuard) -> ModelHandle {
    let request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_dir.manifest_ref.clone(),
    };
    let (output, response): (RunOutput, OpenModelResult) = run_json_command(&["open"], &request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    response.model_handle.expect("model_handle")
}

fn s1_context_tags() -> BTreeMap<String, String> {
    BTreeMap::from([("region".to_string(), "us".to_string())])
}

fn s1_empty_selection_state(model_hash: &str) -> SelectionState {
    canonical_selection_state(
        model_hash.to_string(),
        S1_SCOPE.to_string(),
        s1_context_tags(),
        BTreeMap::new(),
    )
    .expect("empty selection state")
}

fn s1_full_selection_state(model_hash: &str) -> SelectionState {
    canonical_selection_state(
        model_hash.to_string(),
        S1_SCOPE.to_string(),
        s1_context_tags(),
        BTreeMap::from([
            ("cooling_brand".to_string(), "hydra".to_string()),
            ("cooling_model".to_string(), "x200".to_string()),
            ("pump_type".to_string(), "dual".to_string()),
        ]),
    )
    .expect("full selection state")
}

fn s3_full_selection_state(model_hash: &str) -> SelectionState {
    canonical_selection_state(
        model_hash.to_string(),
        S3_SCOPE.to_string(),
        BTreeMap::new(),
        BTreeMap::from([
            ("conveyor_brand".to_string(), "swiftmove".to_string()),
            ("vision_stack".to_string(), "opticore".to_string()),
            ("safety_mode".to_string(), "pl_d".to_string()),
            ("network_topology".to_string(), "ring".to_string()),
        ]),
    )
    .expect("s3 full selection state")
}

fn apply_selection_step_for_scope(
    handle: &ModelHandle,
    scope: &str,
    selection_state: SelectionState,
    facet: &str,
    option: &str,
) -> SelectionState {
    let request = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state,
        selection_delta: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    };
    let (output, response): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    response.selection_state.expect("selection_state")
}

fn apply_selection_step(
    handle: &ModelHandle,
    selection_state: SelectionState,
    facet: &str,
    option: &str,
) -> SelectionState {
    apply_selection_step_for_scope(handle, S1_SCOPE, selection_state, facet, option)
}

fn resolve_ok_for_scope(
    handle: &ModelHandle,
    scope: &str,
    selection_state: SelectionState,
) -> ResolveResult {
    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state,
        implied_choices: Default::default(),
    };
    let (output, response): (RunOutput, ResolveResult) = run_json_command(&["resolve"], &request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    response
}

fn resolve_ok(handle: &ModelHandle, selection_state: SelectionState) -> ResolveResult {
    resolve_ok_for_scope(handle, S1_SCOPE, selection_state)
}

fn io_temp_dir(label: &str) -> TempDirGuard {
    let path = unique_fixture_dir("configflux-interpreter-io", label);
    TempDirGuard {
        path,
        manifest_ref: String::new(),
    }
}

fn run_open_with_request_file(request_path: &Path) -> RunOutput {
    let request_path_str = request_path.to_string_lossy().into_owned();
    run_cli(&["open", "--request-file", &request_path_str], b"")
}

fn run_open_with_file_io(request_path: &Path, response_path: &Path) -> RunOutput {
    let request_path_str = request_path.to_string_lossy().into_owned();
    let response_path_str = response_path.to_string_lossy().into_owned();
    run_cli(
        &[
            "open",
            "--request-file",
            &request_path_str,
            "--response-file",
            &response_path_str,
        ],
        b"",
    )
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms).expect("set permissions");
}

#[cfg(unix)]
fn make_fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo exited with {status}");
}

/// Feed `len` bytes into `path` once something opens the read end.
///
/// The thread blocks in `open(2)` until a reader arrives, so a caller asserting
/// that the transport refuses the FIFO WITHOUT reading it has to release the
/// writer with `drain_fifo` before joining. That asymmetry is the point: a
/// transport that reads the stream unblocks the writer by itself.
#[cfg(unix)]
fn spawn_fifo_writer(path: &Path, len: usize) -> std::thread::JoinHandle<()> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let mut fifo = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open fifo for writing");
        // A reader that walks away mid-stream is not a failure of this writer.
        let _ = fifo.write_all(&vec![b'a'; len]);
    })
}

#[cfg(unix)]
fn drain_fifo(path: &Path) {
    let mut sink = Vec::new();
    std::fs::File::open(path)
        .expect("open fifo for reading")
        .read_to_end(&mut sink)
        .expect("drain fifo");
}

fn deterministic_noise_payload(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut payload = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        payload.push((state >> 56) as u8);
    }
    payload
}

fn malformed_envelope_corpus() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"{".to_vec(),
        b"[]".to_vec(),
        b"null".to_vec(),
        b"{\"schema_version\":1".to_vec(),
        b"{\"schema_version\":1,\"secret\":\"sk_live_malformed_corpus\"".to_vec(),
        b"{\"schema_version\":\"one\"}".to_vec(),
        b"{\"schema_version\":1,\"unexpected\":true}".to_vec(),
        deterministic_noise_payload(0xC0DEC0DE, 17),
        deterministic_noise_payload(0xFEEDFACE, 73),
    ]
}

fn assert_command_determinism_and_file_mode<Req: Serialize, Res: DeserializeOwned>(
    command: &str,
    request: &Req,
    label: &str,
) -> Res {
    let payload = serde_json::to_vec(request).expect("serialize request");

    let first = run_cli(&[command], &payload);
    let second = run_cli(&[command], &payload);
    assert_eq!(first.exit_code, second.exit_code);
    assert_eq!(first.stderr, second.stderr);
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stderr.is_empty());

    let io_dir = io_temp_dir(label);
    let request_path = io_dir.path.join("request.json");
    let response_path = io_dir.path.join("response.json");
    std::fs::write(&request_path, &payload).expect("write request file");

    let request_path_str = request_path.to_string_lossy().into_owned();
    let response_path_str = response_path.to_string_lossy().into_owned();
    let output = run_cli(
        &[
            command,
            "--request-file",
            &request_path_str,
            "--response-file",
            &response_path_str,
        ],
        b"",
    );
    assert_eq!(output.exit_code, first.exit_code);
    assert!(output.stderr.is_empty());
    assert!(output.stdout.is_empty());

    let file_response = std::fs::read(response_path).expect("read response file");
    assert_eq!(file_response, first.stdout);
    serde_json::from_slice::<Res>(&first.stdout).expect("parse response")
}

fn run_full_chain_for_fixture(fixture: &InterpreterFixture) {
    let cmp_dir = emitted_cmp_dir_with_chunks(
        fixture.name,
        fixture.source_defs,
        fixture.source_components,
        fixture.chunk_defs,
        fixture.chunk_components,
    );

    let open_request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_dir.manifest_ref.clone(),
    };
    let open_response: OpenModelResult = assert_command_determinism_and_file_mode(
        "open",
        &open_request,
        &format!("{}-open", fixture.name),
    );
    assert_eq!(open_response.status, OperationStatus::Ok);
    let handle = open_response.model_handle.expect("model handle");

    let init_request = InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: fixture.scope.to_string(),
        context_tags: fixture
            .context_tags
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    };
    let init_response: InitializeSelectionStateResult = assert_command_determinism_and_file_mode(
        "init-selection-state",
        &init_request,
        &format!("{}-init-selection-state", fixture.name),
    );
    assert_eq!(init_response.status, OperationStatus::Ok);
    let mut selection_state = init_response.selection_state.expect("selection state");

    let first_step = fixture.selection_steps[0];
    let options_request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: fixture.scope.to_string(),
        selection_state: selection_state.clone(),
        facet: first_step.0.to_string(),
        include_pruned_reasons: true,
    };
    let options_response: GetSelectionOptionsResult = assert_command_determinism_and_file_mode(
        "options",
        &options_request,
        &format!("{}-options", fixture.name),
    );
    assert_eq!(options_response.status, OperationStatus::Ok);
    assert!(options_response
        .valid_options
        .contains(&first_step.1.to_string()));

    for (idx, (facet, option)) in fixture.selection_steps.iter().enumerate() {
        let select_request = ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: fixture.scope.to_string(),
            selection_state,
            selection_delta: SelectionDelta {
                facet: (*facet).to_string(),
                option: (*option).to_string(),
            },
        };
        let select_response: ApplySelectionResult = assert_command_determinism_and_file_mode(
            "select",
            &select_request,
            &format!("{}-select-{idx}", fixture.name),
        );
        assert_eq!(select_response.status, OperationStatus::Ok);
        selection_state = select_response.selection_state.expect("selection state");
    }

    let mutation_request = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: fixture.scope.to_string(),
        selection_state: selection_state.clone(),
        selection_delta: SelectionDelta {
            facet: first_step.0.to_string(),
            option: "__invalid_option__".to_string(),
        },
    };
    let (mutation_output, mutation_response): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &mutation_request);
    assert_eq!(mutation_output.exit_code, EXIT_COMMAND_ERROR);
    assert!(mutation_output.stderr.is_empty());
    assert_eq!(mutation_response.status, OperationStatus::Error);
    assert!(mutation_response.diagnostics.diagnostics[0]
        .code
        .starts_with("E_SELECTION_"));

    let explain_request = ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: fixture.scope.to_string(),
        selection_state: selection_state.clone(),
        rejected_option: SelectionDelta {
            facet: first_step.0.to_string(),
            option: "__invalid_option__".to_string(),
        },
    };
    let explain_response: ExplainRejectionResult = assert_command_determinism_and_file_mode(
        "explain",
        &explain_request,
        &format!("{}-explain", fixture.name),
    );
    // configflux-whyt / ADR-0031 D2: explaining a rejected option is a
    // *successful query* ("why is X rejected? — here is why"), not a command
    // error. `__invalid_option__` is a division-of-labor invalid-option
    // rejection (ADR-0030 D5): status ok, exit 0, the canonical E_SELECTION_*
    // code, and no solver core (there is no constraint conflict to minimize).
    assert_eq!(explain_response.status, OperationStatus::Ok);
    assert!(explain_response.rejection.code.starts_with("E_SELECTION_"));
    assert!(
        explain_response.rejection.unsat_core.is_none(),
        "a division-of-labor invalid-option rejection carries no unsat core"
    );

    let resolve_request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: fixture.scope.to_string(),
        selection_state,
        implied_choices: Default::default(),
    };
    let resolve_response: ResolveResult = assert_command_determinism_and_file_mode(
        "resolve",
        &resolve_request,
        &format!("{}-resolve", fixture.name),
    );
    assert_eq!(resolve_response.status, OperationStatus::Ok);
    let resolve_hash = resolve_response.resolve_hash.clone().expect("resolve hash");

    let export_request = ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_response.clone(),
        profile: EXPORT_PROFILE_CPP_EARLY_BINDING_V1.to_string(),
    };
    let export_response: ExportResolvedResult = assert_command_determinism_and_file_mode(
        "export-resolved",
        &export_request,
        &format!("{}-export", fixture.name),
    );
    assert_eq!(export_response.status, OperationStatus::Ok);
    assert_eq!(export_response.resolve_hash, Some(resolve_hash.clone()));
    assert!(export_response.generated_artifacts.is_some());

    let bom_request = ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_response,
        profile: EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT.to_string(),
    };
    let bom_response: ExportSoftwareBomResult = assert_command_determinism_and_file_mode(
        "export-software-bom",
        &bom_request,
        &format!("{}-bom", fixture.name),
    );
    assert_eq!(bom_response.status, OperationStatus::Ok);
    assert_eq!(bom_response.resolve_hash, Some(resolve_hash));
    assert!(bom_response.bom_hash.is_some());
    assert!(bom_response.software_bom.is_some());
}

/// configflux-zdf1: when `include_pruned_reasons` is set, `session_compose::options`
/// took `pruned_options` from the compiler's legacy narrowing, which UNDER-reports an
/// override-gated facet (configflux-z1hj). On S1 after `cooling_brand=hydra` the solver
/// holds `cooling_model` `a9` VALID while the compiler prunes it, so `a9` was reported
/// as BOTH valid and pruned. The fix recomputes `pruned_options` against the solver's
/// valid set, so no option is ever in both lists.
#[test]
fn options_pruned_reasons_never_contradict_solver_valid_set() {
    let cmp_dir = emitted_cmp_dir("zdf1-pruned-consistency");
    let handle = open_handle(&cmp_dir);
    let selection_state = init_selection_state_for_fixture(&handle, S1_SCOPE, s1_context_tags());
    let after_hydra = apply_selection_step(&handle, selection_state, "cooling_brand", "hydra");

    let options_request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: S1_SCOPE.to_string(),
        selection_state: after_hydra,
        facet: "cooling_model".to_string(),
        include_pruned_reasons: true,
    };
    let (options_output, options_result): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &options_request);
    assert_eq!(options_output.exit_code, EXIT_OK);
    assert_eq!(options_result.status, OperationStatus::Ok);

    // Solver-authoritative valid set: `a9` remains satisfiable under `hydra`
    // (override-not-exclusion — configflux-z1hj), so both options are valid.
    assert!(
        options_result.valid_options.contains(&"a9".to_string()),
        "solver holds cooling_model=a9 valid under hydra; valid_options={:?}",
        options_result.valid_options
    );
    assert!(options_result.valid_options.contains(&"x200".to_string()));

    // The envelope must be internally consistent: no option may be reported as
    // BOTH valid and pruned. Pre-fix, `a9` appeared in both.
    if let Some(pruned) = &options_result.pruned_options {
        for reason in pruned {
            assert!(
                !options_result.valid_options.contains(&reason.option),
                "cooling_model option {:?} is reported as BOTH valid and pruned",
                reason.option
            );
        }
    }
}

#[test]
fn int_001_happy_path_e2e_chain() {
    let cmp_dir = emitted_cmp_dir("int-001");
    let handle = open_handle(&cmp_dir);

    let mut selection_state =
        init_selection_state_for_fixture(&handle, S1_SCOPE, s1_context_tags());
    let options_request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: selection_state.clone(),
        facet: "cooling_brand".to_string(),
        include_pruned_reasons: true,
    };
    let (options_output, options_result): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &options_request);
    assert_eq!(options_output.exit_code, EXIT_OK);
    assert_eq!(options_result.status, OperationStatus::Ok);
    assert!(options_result.valid_options.contains(&"hydra".to_string()));

    selection_state = apply_selection_step(&handle, selection_state, "cooling_brand", "hydra");
    selection_state = apply_selection_step(&handle, selection_state, "cooling_model", "x200");
    selection_state = apply_selection_step(&handle, selection_state, "pump_type", "dual");

    let resolve_result = resolve_ok(&handle, selection_state);
    let resolve_hash = resolve_result.resolve_hash.clone().expect("resolve hash");
    // ADR-0059 D3: the `resolve` response carries the payload identity beside
    // the resolution identity. It reaches the interpreter for free — same
    // shared struct — so what INT-001 pins is that the CLI response actually
    // serializes it, and that the two hashes are genuinely distinct values
    // rather than one pre-image computed twice. `cfx` printing the same value
    // for the same target is asserted across both front ends by
    // cfx/tests/cfx_resolve_differential.py.
    let resolved_output_hash = resolve_result
        .resolved_output_hash
        .clone()
        .expect("resolve response must carry resolved_output_hash");
    assert_eq!(resolved_output_hash.len(), 64, "sha256 hex");
    assert_ne!(
        resolved_output_hash, resolve_hash,
        "the payload identity must not be the resolution identity"
    );

    let export_request = ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_result.clone(),
        profile: EXPORT_PROFILE_CPP_EARLY_BINDING_V1.to_string(),
    };
    let (export_output, export_result): (RunOutput, ExportResolvedResult) =
        run_json_command(&["export-resolved"], &export_request);
    assert_eq!(export_output.exit_code, EXIT_OK);
    assert_eq!(export_result.status, OperationStatus::Ok);
    let generated = export_result
        .generated_artifacts
        .expect("generated artifacts");
    assert!(!generated.generator_hash.is_empty());

    let bom_request = ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT.to_string(),
    };
    let (bom_output, bom_result): (RunOutput, ExportSoftwareBomResult) =
        run_json_command(&["export-software-bom"], &bom_request);
    assert_eq!(bom_output.exit_code, EXIT_OK);
    assert_eq!(bom_result.status, OperationStatus::Ok);
    assert_eq!(bom_result.resolve_hash, Some(resolve_hash));
    assert!(bom_result.software_bom.is_some());
}

#[test]
fn int_002_corrupted_cmp_artifacts_produce_deterministic_loader_diagnostics() {
    let cmp_dir = emitted_cmp_dir("int-002");
    let index_path = cmp_dir.path.join("index.cfir.json");
    let mut index_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index_path).expect("read index"))
            .expect("parse index json");
    index_json["config_hash"] = serde_json::Value::String("00".repeat(32));
    std::fs::write(
        &index_path,
        serde_json::to_vec_pretty(&index_json).expect("serialize index"),
    )
    .expect("write corrupt index");

    let open_request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_dir.manifest_ref.clone(),
    };
    let payload = serde_json::to_vec(&open_request).expect("serialize open request");
    let first = run_cli(&["open"], &payload);
    let second = run_cli(&["open"], &payload);

    assert_eq!(first.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(first.stderr, second.stderr);
    assert_eq!(first.stdout, second.stdout);

    let response: OpenModelResult =
        serde_json::from_slice(&first.stdout).expect("parse open response");
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_LOADER_INDEX_INVALID.to_string()
    );
}

#[test]
fn int_003_malformed_envelope_fails_closed_with_stable_transport_error() {
    let payload = br#"{"schema_version":1,"scope":"component:thermal_control""#;
    let first = run_cli(&["resolve"], payload);
    let second = run_cli(&["resolve"], payload);

    assert_eq!(first.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(first.stdout.is_empty());
    assert_eq!(first.stderr, second.stderr);
    assert!(first.stderr.contains(E_INTERPRETER_CLI_REQUEST_INVALID));
}

#[test]
fn int_004_selection_tampering_rejected_with_selection_family_code() {
    let cmp_dir = emitted_cmp_dir("int-004");
    let handle = open_handle(&cmp_dir);
    let mut tampered_state = s1_full_selection_state(&handle.model_hash);
    tampered_state.selection_state_hash = "00".repeat(32);

    let request = ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: S1_SCOPE.to_string(),
        selection_state: tampered_state,
        rejected_option: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "x100".to_string(),
        },
    };
    let (output, response): (RunOutput, ExplainRejectionResult) =
        run_json_command(&["explain"], &request);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.rejection.code,
        E_SELECTION_STATE_INVALID.to_string()
    );
}

#[test]
fn int_005_resolve_integrity_failure_uses_resolve_family_code() {
    let cmp_dir = emitted_cmp_dir("int-005");
    let handle = open_handle(&cmp_dir);
    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: s1_empty_selection_state(&handle.model_hash),
        implied_choices: Default::default(),
    };
    let (output, response): (RunOutput, ResolveResult) = run_json_command(&["resolve"], &request);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_RESOLVE_CONTEXT_UNSATISFIED.to_string()
    );
}

#[test]
fn int_006_export_misuse_preserves_export_and_sbom_error_families() {
    let cmp_dir = emitted_cmp_dir("int-006");
    let handle = open_handle(&cmp_dir);
    let resolve_result = resolve_ok(&handle, s1_full_selection_state(&handle.model_hash));

    let export_request = ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_result.clone(),
        profile: "invalid_profile".to_string(),
    };
    let (export_output, export_response): (RunOutput, ExportResolvedResult) =
        run_json_command(&["export-resolved"], &export_request);
    assert_eq!(export_output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(export_response.status, OperationStatus::Error);
    assert_eq!(
        export_response.diagnostics.diagnostics[0].code,
        E_EXPORT_PROFILE_INVALID.to_string()
    );

    let bom_request = ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: "invalid_profile".to_string(),
    };
    let (bom_output, bom_response): (RunOutput, ExportSoftwareBomResult) =
        run_json_command(&["export-software-bom"], &bom_request);
    assert_eq!(bom_output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(bom_response.status, OperationStatus::Error);
    assert_eq!(
        bom_response.diagnostics.diagnostics[0].code,
        E_SBOM_PROFILE_INVALID.to_string()
    );
}

#[test]
fn int_007_determinism_replay_returns_byte_identical_json() {
    let cmp_dir = emitted_cmp_dir("int-007");
    let handle = open_handle(&cmp_dir);
    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: s1_full_selection_state(&handle.model_hash),
        implied_choices: Default::default(),
    };
    let payload = serde_json::to_vec(&request).expect("serialize request");

    let first = run_cli(&["resolve"], &payload);
    let second = run_cli(&["resolve"], &payload);

    assert_eq!(first.exit_code, EXIT_OK);
    assert_eq!(second.exit_code, EXIT_OK);
    assert_eq!(first.stderr, second.stderr);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn int_008_diagnostics_do_not_leak_payload_or_stack_details() {
    let payload = br#"{"schema_version":1,"secret":"sk_live_should_not_leak","scope":"oops""#;
    let output = run_cli(&["resolve"], payload);

    assert_eq!(output.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains(E_INTERPRETER_CLI_REQUEST_INVALID));
    assert!(!output.stderr.contains("sk_live_should_not_leak"));
    assert!(!output.stderr.to_lowercase().contains("panic"));
    assert!(!output.stderr.to_lowercase().contains("stack backtrace"));
}

#[test]
fn int_009_oversized_input_is_rejected_fail_closed() {
    let oversized = vec![b'a'; REQUEST_SIZE_LIMIT_BYTES + 1];
    let output = run_cli(&["open"], &oversized);

    assert_eq!(output.exit_code, EXIT_TRANSPORT_ERROR);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains(E_INTERPRETER_CLI_REQUEST_TOO_LARGE));
}

#[test]
fn int_010_traceability_hash_lineage_is_complete() {
    let cmp_dir = emitted_cmp_dir("int-010");
    let handle = open_handle(&cmp_dir);
    let resolve_result = resolve_ok(&handle, s1_full_selection_state(&handle.model_hash));
    let resolve_hash = resolve_result.resolve_hash.clone().expect("resolve hash");

    let export_request = ExportResolvedRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result: resolve_result.clone(),
        profile: EXPORT_PROFILE_CPP_EARLY_BINDING_V1.to_string(),
    };
    let (_, export_result): (RunOutput, ExportResolvedResult) =
        run_json_command(&["export-resolved"], &export_request);
    assert_eq!(export_result.status, OperationStatus::Ok);
    assert_eq!(export_result.resolve_hash, Some(resolve_hash.clone()));
    let generated = export_result
        .generated_artifacts
        .expect("generated artifacts");
    assert!(!generated.generator_hash.is_empty());
    assert!(!generated.files.is_empty());
    assert!(generated
        .files
        .iter()
        .all(|file| !file.path.is_empty() && !file.content_hash.is_empty()));

    let bom_request = ExportSoftwareBomRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        resolve_result,
        profile: EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT.to_string(),
    };
    let (_, bom_result): (RunOutput, ExportSoftwareBomResult) =
        run_json_command(&["export-software-bom"], &bom_request);
    assert_eq!(bom_result.status, OperationStatus::Ok);
    assert_eq!(bom_result.resolve_hash, Some(resolve_hash.clone()));
    let bom_hash = bom_result.bom_hash.clone().expect("bom hash");
    let software_bom: SoftwareBomV1 = bom_result.software_bom.expect("software bom");
    assert_eq!(software_bom.model_hash, handle.model_hash);
    assert_eq!(software_bom.resolve_hash, resolve_hash);
    assert_eq!(software_bom.bom_hash, bom_hash);
    assert!(software_bom.stats.component_count > 0);
    assert!(software_bom.stats.parameter_count > 0);
}

#[test]
fn fixture_pack_includes_s3_happy_path() {
    let cmp_dir = emitted_cmp_dir_with_chunks(
        "s3-happy-path",
        S3_SOURCE_DEFS,
        S3_SOURCE_COMPONENTS,
        S3_CHUNK_DEFS,
        S3_CHUNK_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);
    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S3_SCOPE.to_string(),
        selection_state: s3_full_selection_state(&handle.model_hash),
        implied_choices: Default::default(),
    };
    let (output, response): (RunOutput, ResolveResult) = run_json_command(&["resolve"], &request);
    assert_eq!(output.exit_code, EXIT_OK);
    assert_eq!(response.status, OperationStatus::Ok);
    assert!(response.resolve_hash.is_some());
}

#[test]
fn fixture_pack_includes_s5_non_robotics_happy_path() {
    run_full_chain_for_fixture(&S5_SMOKE_FIXTURE);
}

#[test]
fn fixture_pack_includes_s2_s4_smoke_and_medium_profiles() {
    for fixture in [
        &S2_SMOKE_FIXTURE,
        &S2_MEDIUM_FIXTURE,
        &S4_SMOKE_FIXTURE,
        &S4_MEDIUM_FIXTURE,
    ] {
        run_full_chain_for_fixture(fixture);
    }
}

#[test]
fn int_011_determinism_across_all_commands_and_file_mode_equivalence() {
    run_full_chain_for_fixture(&S4_MEDIUM_FIXTURE);
}

#[test]
fn int_012_transport_request_file_negative_matrix_is_deterministic() {
    let io_dir = io_temp_dir("int-012");

    let missing_path = io_dir.path.join("missing.request.json");
    let missing_first = run_open_with_request_file(&missing_path);
    let missing_second = run_open_with_request_file(&missing_path);
    assert_eq!(missing_first.stderr, missing_second.stderr);
    assert_transport_failure(&missing_first, E_INTERPRETER_CLI_REQUEST_IO);
    assert!(missing_first.stderr.contains("not_found"));

    let malformed_path = io_dir.path.join("malformed.request.json");
    std::fs::write(
        &malformed_path,
        br#"{"schema_version":1,"secret":"sk_live_request_file_secret""#,
    )
    .expect("write malformed request");
    let malformed_first = run_open_with_request_file(&malformed_path);
    let malformed_second = run_open_with_request_file(&malformed_path);
    assert_eq!(malformed_first.stderr, malformed_second.stderr);
    assert_transport_failure(&malformed_first, E_INTERPRETER_CLI_REQUEST_INVALID);
    assert!(!malformed_first
        .stderr
        .contains("sk_live_request_file_secret"));

    #[cfg(unix)]
    {
        let denied_path = io_dir.path.join("permission-denied.request.json");
        std::fs::write(&denied_path, b"{\"schema_version\":1}")
            .expect("write permission denied request");
        set_mode(&denied_path, 0o000);

        let denied_first = run_open_with_request_file(&denied_path);
        let denied_second = run_open_with_request_file(&denied_path);
        set_mode(&denied_path, 0o600);

        assert_eq!(denied_first.stderr, denied_second.stderr);
        assert_transport_failure(&denied_first, E_INTERPRETER_CLI_REQUEST_IO);
        assert!(denied_first.stderr.contains("permission_denied"));
    }

    #[cfg(unix)]
    {
        // configflux-mtmi. Neither size check can refuse a path whose `stat()`
        // size understates what a read returns -- a directory reports a small
        // size and a FIFO reports none at all -- so the transport refuses
        // anything that is not a regular file before it opens it. Only a
        // regular file's recorded size bounds what the read will deliver.
        let directory_path = io_dir.path.join("request-directory");
        std::fs::create_dir_all(&directory_path).expect("create request directory");
        let directory_first = run_open_with_request_file(&directory_path);
        let directory_second = run_open_with_request_file(&directory_path);
        assert_eq!(directory_first.stderr, directory_second.stderr);
        assert_transport_failure(&directory_first, E_INTERPRETER_CLI_REQUEST_IO);
        assert!(directory_first.stderr.contains("not a regular file"));

        // The FIFO carries more than the bound accepts. Refusing it from its
        // file type means none of that stream is ever buffered. The assertions
        // run BEFORE the drain deliberately: a transport that reads the stream
        // instead fails here, rather than blocking forever on the second
        // invocation once the one writer has already been consumed.
        let fifo_path = io_dir.path.join("request.fifo");
        make_fifo(&fifo_path);
        let fifo_writer = spawn_fifo_writer(&fifo_path, REQUEST_SIZE_LIMIT_BYTES + 2);
        let fifo_first = run_open_with_request_file(&fifo_path);
        assert_transport_failure(&fifo_first, E_INTERPRETER_CLI_REQUEST_IO);
        assert!(fifo_first.stderr.contains("not a regular file"));
        let fifo_second = run_open_with_request_file(&fifo_path);
        assert_eq!(fifo_first.stderr, fifo_second.stderr);
        drain_fifo(&fifo_path);
        fifo_writer.join().expect("fifo writer");
    }
}
#[test]
fn transport_request_reader_is_bounded_one_byte_past_the_limit() {
    // An endless stream: the `take` bound is the only thing that can end this
    // read, so a reader waiting for EOF instead would never return at all. Both
    // transports share this reader, which is what makes the FILE path bounded
    // even though a file's own recorded size is not trustworthy
    // (configflux-mtmi).
    struct Endless;
    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            buf.fill(b'a');
            Ok(buf.len())
        }
    }

    let mut endless = Endless;
    let capped = read_capped(&mut endless).expect("capped read");
    assert_eq!(capped.len(), REQUEST_SIZE_LIMIT_BYTES + 1);

    // The bound sits one byte PAST the limit and not on it, so a request of
    // exactly the maximum accepted size still arrives whole.
    let io_dir = io_temp_dir("transport-bound");
    let at_limit_path = io_dir.path.join("at-limit.request.json");
    std::fs::write(&at_limit_path, vec![b'a'; REQUEST_SIZE_LIMIT_BYTES])
        .expect("write at-limit request");
    match read_request_file(&at_limit_path) {
        Ok(payload) => assert_eq!(payload.len(), REQUEST_SIZE_LIMIT_BYTES),
        Err(_) => panic!("a request of exactly the maximum size must be read whole"),
    }
}

#[test]
fn int_013_transport_response_file_negative_matrix_is_deterministic() {
    let cmp_dir = emitted_cmp_dir("int-013");
    let io_dir = io_temp_dir("int-013");
    let request_path = io_dir.path.join("open.request.json");
    let request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_dir.manifest_ref.clone(),
    };
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize request"),
    )
    .expect("write request");

    let missing_response_path = io_dir.path.join("missing-dir").join("open.response.json");
    let missing_first = run_open_with_file_io(&request_path, &missing_response_path);
    let missing_second = run_open_with_file_io(&request_path, &missing_response_path);
    assert_eq!(missing_first.stderr, missing_second.stderr);
    assert_transport_failure(&missing_first, E_INTERPRETER_CLI_RESPONSE_IO);
    assert!(missing_first.stderr.contains("not_found"));

    #[cfg(unix)]
    {
        let denied_dir = io_dir.path.join("readonly-dir");
        std::fs::create_dir_all(&denied_dir).expect("create readonly dir");
        set_mode(&denied_dir, 0o555);

        let denied_response_path = denied_dir.join("open.response.json");
        let denied_first = run_open_with_file_io(&request_path, &denied_response_path);
        let denied_second = run_open_with_file_io(&request_path, &denied_response_path);
        set_mode(&denied_dir, 0o755);

        assert_eq!(denied_first.stderr, denied_second.stderr);
        assert_transport_failure(&denied_first, E_INTERPRETER_CLI_RESPONSE_IO);
        assert!(denied_first.stderr.contains("permission_denied"));
    }
}

#[test]
fn int_014_malformed_envelope_property_corpus_is_fail_closed() {
    let commands = [
        "open",
        "init-selection-state",
        "options",
        "select",
        "explain",
        "resolve",
        "export-resolved",
        "export-software-bom",
    ];
    let corpus = malformed_envelope_corpus();

    for command in commands {
        for payload in &corpus {
            let first = run_cli(&[command], payload);
            let second = run_cli(&[command], payload);
            assert_eq!(first.stderr, second.stderr);
            assert_transport_failure(&first, E_INTERPRETER_CLI_REQUEST_INVALID);
            assert!(!first.stderr.contains("sk_live_malformed_corpus"));
        }
    }
}

#[test]
fn int_015_dispatch_invalid_args_property_is_fail_closed() {
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["unknown-command"],
        vec!["--unknown-flag"],
        vec!["open", "--unknown-flag"],
        vec!["init-selection-state", "--request-file"],
        vec!["resolve", "--request-file"],
        vec!["options", "--response-file"],
    ];

    for args in &cases {
        let first = run_cli(args, b"");
        let second = run_cli(args, b"");
        assert_eq!(first.stderr, second.stderr);
        assert_transport_failure(&first, E_INTERPRETER_CLI_ARGS_INVALID);
    }
}

#[test]
fn supports_request_and_response_file_mode() {
    let cmp_dir = emitted_cmp_dir("file-mode");
    let io_dir = emitted_cmp_dir("file-io");
    let request_path = io_dir.path.join("open-request.json");
    let response_path = io_dir.path.join("open-response.json");

    let request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: cmp_dir.manifest_ref.clone(),
    };
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize request"),
    )
    .expect("write request file");

    let args_vec = vec![
        "open",
        "--request-file",
        request_path.to_str().expect("request path utf8"),
        "--response-file",
        response_path.to_str().expect("response path utf8"),
    ];
    let output = run_cli(&args_vec, b"");

    assert_eq!(output.exit_code, EXIT_OK);
    assert!(output.stderr.is_empty());
    assert!(output.stdout.is_empty());

    let response_bytes = std::fs::read(response_path).expect("read response file");
    let response: OpenModelResult =
        serde_json::from_slice(&response_bytes).expect("parse response file");
    assert_eq!(response.status, OperationStatus::Ok);
}

#[test]
fn command_error_exit_path_keeps_diagnostics_and_exit_code_mapping() {
    let cmp_dir = emitted_cmp_dir("diagnostic-mapping");
    let handle = open_handle(&cmp_dir);
    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: s1_empty_selection_state(&handle.model_hash),
        implied_choices: Default::default(),
    };

    let (output, response): (RunOutput, ResolveResult) = run_json_command(&["resolve"], &request);
    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(response.error_count, 1);
    assert_eq!(
        response.diagnostics.diagnostics[0].severity,
        DiagnosticSeverity::Error
    );
}

/// Single-path end-to-end coverage for the solver-wired interpreter CLI
/// (configflux-g3f.5 / ADR-0017 + the 2026-06-09 amendment). This replaces the
/// retired g3f.2 dual-path parity test: the interpreter selection commands are
/// migrated off the legacy `compiler::loader_api` selection entry points
/// (g3f.2 wired the `SolverSession` wrapper; g3f.5 retires the legacy path), so
/// the test drives the one production path — `init -> options ->
/// select(accept) -> select(chain) -> select(reject) -> resolve` through the
/// real CLI dispatch — and asserts the real expected output at each step
/// instead of diffing against the legacy functions (which g3f.6 deletes).
///
/// Why no legacy comparison: the migration safety the dual-path test guarded
/// (solver answer == legacy answer) is confirmed and recorded on g3f.2. Calling
/// `loader_api::{get_selection_options, apply_selection, resolve_from_selection}`
/// here would re-introduce exactly the legacy selection-path coupling g3f.5
/// removes and g3f.6 deletes. Coverage is retained by asserting the concrete
/// contract (domain membership, committed selection state, the ADR-0017 §5
/// `E_SELECTION_*` rejection family + exit code, and the compiler-composed rich
/// `ResolveResult`), not by a parity diff.
///
/// Hard guard: the fixture is the constraint-bearing S1 model, which emits a
/// `.ccm` that `open` advertises on `ModelHandle.ccm_ref`. The guard asserts
/// the `.ccm` is present and that `options` enumerates a real domain value, so
/// the wrapper cannot silently fall back to the pure-legacy path and leave this
/// test vacuously green.
#[test]
fn int_016_solver_session_single_path_e2e() {
    let cmp_dir = emitted_cmp_dir("int-016-e2e");
    let handle = open_handle(&cmp_dir);

    // Hard guard #1: the solver path must be reachable, otherwise this test is
    // vacuous (the wrapper would silently serve the pure-legacy fallback). The
    // product compile path emits the `.ccm` sibling and `open` advertises it on
    // `ModelHandle.ccm_ref` (configflux-9hi2 / ADR-0017 §2).
    assert!(
        !handle.ccm_ref.trim().is_empty(),
        "open must advertise a .ccm ref so the solver path is exercised"
    );
    assert!(
        Path::new(handle.ccm_ref.trim())
            .join("ccm.manifest.json")
            .is_file(),
        "the product compile path must emit a non-empty .ccm at {}",
        handle.ccm_ref
    );

    let base_state = init_selection_state_for_fixture(&handle, S1_SCOPE, s1_context_tags());

    // ---- options (solver Session::valid_options) ---------------------------
    let options_request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: base_state.clone(),
        facet: "cooling_brand".to_string(),
        include_pruned_reasons: false,
    };
    let (options_output, options_cli): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &options_request);
    assert_eq!(options_output.exit_code, EXIT_OK);
    assert!(options_output.stderr.is_empty());
    assert_eq!(options_cli.status, OperationStatus::Ok);
    assert_eq!(options_cli.facet, "cooling_brand");
    // Hard guard #2 (and the real options assertion): the solver genuinely
    // produced a selection decision, not an empty stub — `hydra` is a real
    // option of `cooling_brand` in the S1 model.
    assert!(
        options_cli.valid_options.contains(&"hydra".to_string()),
        "solver-backed options must enumerate the real cooling_brand domain; got {:?}",
        options_cli.valid_options
    );

    // ---- select #1 (accepted selection: cooling_brand = hydra) -------------
    let apply1_request = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: base_state.clone(),
        selection_delta: SelectionDelta {
            facet: "cooling_brand".to_string(),
            option: "hydra".to_string(),
        },
    };
    let (apply1_output, apply1_cli): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &apply1_request);
    assert_eq!(apply1_output.exit_code, EXIT_OK);
    assert!(apply1_output.stderr.is_empty());
    assert_eq!(apply1_cli.status, OperationStatus::Ok);
    let state_after_1 = apply1_cli
        .selection_state
        .clone()
        .expect("accepted apply must return the next selection state");
    // The accepted choice must be committed into the next selection state.
    assert_eq!(
        state_after_1.choices.get("cooling_brand"),
        Some(&"hydra".to_string()),
        "accepted select must commit cooling_brand=hydra into the next state"
    );

    // ---- select #2 (chained selection: cooling_model = x200) ---------------
    // Runs against the state produced by select #1, exercising replay of a
    // prior committed choice onto the re-derived ephemeral session.
    let apply2_request = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: state_after_1.clone(),
        selection_delta: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "x200".to_string(),
        },
    };
    let (apply2_output, apply2_cli): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &apply2_request);
    assert_eq!(apply2_output.exit_code, EXIT_OK);
    assert!(apply2_output.stderr.is_empty());
    assert_eq!(apply2_cli.status, OperationStatus::Ok);
    let state_after_2 = apply2_cli
        .selection_state
        .clone()
        .expect("accepted chained apply must return the next selection state");
    // Both the replayed and the new choice must be present after the chain.
    assert_eq!(
        state_after_2.choices.get("cooling_brand"),
        Some(&"hydra".to_string()),
        "chained select must retain the prior committed cooling_brand=hydra"
    );
    assert_eq!(
        state_after_2.choices.get("cooling_model"),
        Some(&"x200".to_string()),
        "chained select must commit cooling_model=x200 into the next state"
    );

    // ---- select #3 (rejected selection: invalid option) --------------------
    // An option outside the facet domain must be rejected through the §5
    // mapping: command-error exit code, error status, no next state, and an
    // E_SELECTION_* diagnostic. This is the solver's accept/reject decision,
    // surfaced as the selection family the CLI contract promises.
    let reject_request = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: base_state.clone(),
        selection_delta: SelectionDelta {
            facet: "cooling_brand".to_string(),
            option: "nonexistent_brand".to_string(),
        },
    };
    let (reject_output, reject_cli): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &reject_request);
    assert_eq!(reject_output.exit_code, EXIT_COMMAND_ERROR);
    assert!(reject_output.stderr.is_empty());
    assert_eq!(reject_cli.status, OperationStatus::Error);
    assert!(reject_cli.selection_state.is_none());
    assert!(
        !reject_cli.diagnostics.diagnostics.is_empty(),
        "a rejected select must carry at least one diagnostic"
    );
    assert!(
        reject_cli.diagnostics.diagnostics[0]
            .code
            .starts_with("E_SELECTION_"),
        "rejection must use the selection family (ADR-0017 §5), got '{}'",
        reject_cli.diagnostics.diagnostics[0].code
    );

    // ---- resolve (solver-gated, compiler-composed rich ResolveResult) ------
    let full_state = s1_full_selection_state(&handle.model_hash);
    let resolve_request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: full_state,
        implied_choices: Default::default(),
    };
    let (resolve_output, resolve_cli): (RunOutput, ResolveResult) =
        run_json_command(&["resolve"], &resolve_request);
    assert_eq!(resolve_output.exit_code, EXIT_OK);
    assert!(resolve_output.stderr.is_empty());
    assert_eq!(resolve_cli.status, OperationStatus::Ok);
    // The rich compiler-composed payload must be present (proves the wrapper
    // delegated envelope construction to the compiler resolver, not the
    // solver's flat shape — amendment "i0ne demotion"): the legacy-recipe
    // resolve_hash and the nested resolved_output tree keyed by scope.
    assert!(
        resolve_cli.resolve_hash.is_some(),
        "resolve must carry the legacy-recipe resolve_hash from the compiler"
    );
    let resolved_output = resolve_cli
        .resolved_output
        .as_ref()
        .expect("resolve must carry the compiler's nested resolved_output tree");
    let resolved_obj = resolved_output
        .as_object()
        .expect("resolved_output must be a JSON object keyed by component");
    // The resolved tree is keyed by the bare component name (the `component:`
    // scope prefix is stripped), so derive the expected key from S1_SCOPE.
    let expected_component = S1_SCOPE
        .strip_prefix("component:")
        .expect("S1_SCOPE is a component scope");
    assert!(
        resolved_obj.contains_key(expected_component),
        "resolved_output must contain the resolved config for component {expected_component}; got {:?}",
        resolved_obj.keys().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ADR-0030 (configflux-dj7f): CCM hard precondition + fallback retirement.
//
// These tests pin the FAIL-CLOSED contract: when no usable `.ccm` solver model
// is reachable, the interpreter selection commands return a stable diagnostic
// instead of silently degrading to the legacy compiler path. The base
// selection state is still derived from a real opened handle (so the request is
// otherwise valid); only the `ccm_ref` is clobbered to simulate an unreachable
// or unloadable solver model — which is exactly the case ADR-0017's fallback
// used to absorb and ADR-0030 D1 now refuses.
// ---------------------------------------------------------------------------

/// Clone `handle` with its `.ccm` reference replaced — used to simulate an
/// unreachable (`""`) or unloadable (bogus path) solver model.
fn handle_with_ccm_ref(handle: &ModelHandle, ccm_ref: &str) -> ModelHandle {
    let mut clone = handle.clone();
    clone.ccm_ref = ccm_ref.to_string();
    clone
}

// ----- Solver-conflict fixture (configflux-whyt / configflux-autp) ----------
//
// The S1 smoke fixture models cross-facet logic only as conditional value
// *overrides* on `control_driver` (e.g. "if brand=aeroflux & model=a9, use the
// aeroflux driver"). An override is not an exclusion: every (cooling_brand,
// cooling_model) pair stays satisfiable, so the now-correct solver
// (post-configflux-autp) reports `cooling_model=a9` as VALID under
// `cooling_brand=hydra` — there is no conflict to minimise and no core to
// populate. That is the right answer for that model; it just isn't a conflict.
//
// The populated-core E2E below therefore needs a model that actually encodes a
// hard cross-facet exclusion. This fixture mirrors the semantics the solver's
// own real-`.ccm` explain regression uses (solver/tests/explain_rejection_real_ccm.rs:
// a feasible space that requires one option and forbids the conflicting one),
// authored here in the compilable scenario grammar as a named `constraints:`
// entry that excludes exactly the `(hydra, a9)` combination
// (`cooling_brand != 'hydra' || cooling_model != 'a9'` — the De Morgan negation
// of the forbidden pair; the condition grammar is a Boolean conjunction/
// disjunction of tag-equality atoms, compiler/src/conditions/mod.rs). Selecting
// `cooling_brand=hydra` then explaining `cooling_model=a9` is then a genuine
// conflict whose minimal core names the prior `cooling_brand.hydra` selection
// and the model rule relating the two facets.
//
// ADR-0054 §8 migration: this rule used to be carried by a `policy_module`
// component whose `condition` was AND-folded into the BDD root. A component
// condition is an inclusion selector and nothing else (§3), so under §5.1 that
// shape asserts nothing and this fixture would silently stop encoding a
// conflict — the exact phantom-policy failure the `constraints` namespace
// exists to make impossible. The two facets must also be DECLARED as part of
// the move: `link_verify::validate_constraints` builds its legal-facet set from
// declared facets plus facets inferred from component/definition conditions,
// and a constraint condition infers nothing, so the domains that used to exist
// only by inference from the very condition being migrated now have to be
// written down.
const SOLVER_CONFLICT_SCOPE: &str = "component:climate";

const SOLVER_CONFLICT_DEFS: &str = r#"{
    "package": "s_whyt_conflict",
    "version": "1.0.0",
    "definitions": {
        "brand_slot": {
            "type": "string",
            "doc": "Runtime-selectable cooling brand",
            "lifecycle": "runtime",
            "safety": "q_m",
            "access": "technician"
        },
        "model_slot": {
            "type": "string",
            "doc": "Runtime-selectable cooling model",
            "lifecycle": "runtime",
            "safety": "q_m",
            "access": "technician"
        }
    },
    "facets": {
        "cooling_brand": {
            "values": ["hydra", "arctic"],
            "default": "hydra",
            "doc": "Cooling brand."
        },
        "cooling_model": {
            "values": ["x200", "a9"],
            "default": "x200",
            "doc": "Cooling model."
        }
    },
    "constraints": {
        "combo_guard": {
            "condition": "cooling_brand != 'hydra' || cooling_model != 'a9'",
            "doc": "The hydra brand may not be paired with the a9 model."
        }
    }
}"#;

const SOLVER_CONFLICT_COMPONENTS: &str = r#"{
    "package": "s_whyt_conflict",
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
                    "req_id": "req_whyt_001"
                },
                "cooling_model": {
                    "inherits": "model_slot",
                    "type": "string",
                    "doc": "Runtime cooling model selection",
                    "value": "x200",
                    "lifecycle": "runtime",
                    "safety": "q_m",
                    "access": "technician",
                    "req_id": "req_whyt_002"
                }
            }
        }
    }
}"#;

/// Compile and open the solver-conflict fixture, returning the model handle.
/// The handle advertises a usable `.ccm`, so the solver explain path is live.
fn open_solver_conflict_handle(label: &str) -> (TempDirGuard, ModelHandle) {
    let cmp_dir = emitted_cmp_dir_with_chunks(
        label,
        "scenarios/s_whyt_conflict/00_definitions.toml",
        "scenarios/s_whyt_conflict/10_components.toml",
        SOLVER_CONFLICT_DEFS,
        SOLVER_CONFLICT_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);
    (cmp_dir, handle)
}

/// Build the `cooling_brand=hydra` selection state on the conflict fixture, the
/// committed prior choice the populated-core E2E explains *against*.
fn solver_conflict_state_after_hydra(handle: &ModelHandle) -> SelectionState {
    let base = init_selection_state_for_fixture(handle, SOLVER_CONFLICT_SCOPE, BTreeMap::new());
    apply_selection_step_for_scope(handle, SOLVER_CONFLICT_SCOPE, base, "cooling_brand", "hydra")
}

/// D1: `options` fails closed with `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` when
/// no usable solver model is reachable — for both an empty ref and a bogus
/// (unloadable) path. The legacy `get_selection_options` path is NOT served.
#[test]
fn dj7f_options_fails_closed_when_ccm_unavailable() {
    let cmp_dir = emitted_cmp_dir("dj7f-options-unavailable");
    let real_handle = open_handle(&cmp_dir);
    let base_state = init_selection_state_for_fixture(&real_handle, S1_SCOPE, s1_context_tags());

    for bogus in ["", "/nonexistent/configflux/dj7f/ccm"] {
        let handle = handle_with_ccm_ref(&real_handle, bogus);
        let request = GetSelectionOptionsRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle,
            scope: S1_SCOPE.to_string(),
            selection_state: base_state.clone(),
            facet: "cooling_brand".to_string(),
            include_pruned_reasons: false,
        };
        let (output, response): (RunOutput, GetSelectionOptionsResult) =
            run_json_command(&["options"], &request);
        assert_eq!(
            output.exit_code, EXIT_COMMAND_ERROR,
            "options must fail closed (exit code) for ccm_ref {bogus:?}"
        );
        assert_eq!(response.status, OperationStatus::Error);
        assert!(
            response.valid_options.is_empty(),
            "fail-closed options must not enumerate any legacy options for {bogus:?}"
        );
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
            "options must surface the model-unavailable code for {bogus:?}"
        );
    }
}

/// D1: `select` fails closed with `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` when
/// no usable solver model is reachable. The selection delta is one that legacy
/// would accept — proving the fail-closed decision is the solver precondition,
/// not a constraint rejection.
#[test]
fn dj7f_select_fails_closed_when_ccm_unavailable() {
    let cmp_dir = emitted_cmp_dir("dj7f-select-unavailable");
    let real_handle = open_handle(&cmp_dir);
    let base_state = init_selection_state_for_fixture(&real_handle, S1_SCOPE, s1_context_tags());

    for bogus in ["", "/nonexistent/configflux/dj7f/ccm"] {
        let handle = handle_with_ccm_ref(&real_handle, bogus);
        let request = ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle,
            scope: S1_SCOPE.to_string(),
            selection_state: base_state.clone(),
            selection_delta: SelectionDelta {
                facet: "cooling_brand".to_string(),
                option: "hydra".to_string(),
            },
        };
        let (output, response): (RunOutput, ApplySelectionResult) =
            run_json_command(&["select"], &request);
        assert_eq!(
            output.exit_code, EXIT_COMMAND_ERROR,
            "select must fail closed (exit code) for ccm_ref {bogus:?}"
        );
        assert_eq!(response.status, OperationStatus::Error);
        assert!(
            response.selection_state.is_none(),
            "fail-closed select must not return a next state for {bogus:?}"
        );
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
            "select must surface the model-unavailable code for {bogus:?}"
        );
    }
}

/// D1: `resolve` fails closed with `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` when no
/// usable solver model is reachable. The sat-gate is no longer
/// advisory-on-absence — the solver model is part of every resolve decision's
/// lineage, even though the compiler resolver would independently accept this
/// (satisfiable) selection.
#[test]
fn dj7f_resolve_fails_closed_when_ccm_unavailable() {
    let cmp_dir = emitted_cmp_dir("dj7f-resolve-unavailable");
    let real_handle = open_handle(&cmp_dir);

    for bogus in ["", "/nonexistent/configflux/dj7f/ccm"] {
        let handle = handle_with_ccm_ref(&real_handle, bogus);
        let full_state = s1_full_selection_state(&handle.model_hash);
        let request = ResolveFromSelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle,
            scope: S1_SCOPE.to_string(),
            selection_state: full_state,
            implied_choices: Default::default(),
        };
        let (output, response): (RunOutput, ResolveResult) =
            run_json_command(&["resolve"], &request);
        assert_eq!(
            output.exit_code, EXIT_COMMAND_ERROR,
            "resolve must fail closed (exit code) for ccm_ref {bogus:?}"
        );
        assert_eq!(response.status, OperationStatus::Error);
        assert!(
            response.resolved_output.is_none(),
            "fail-closed resolve must not compose a payload for {bogus:?}"
        );
        assert!(
            response.resolve_hash.is_none(),
            "fail-closed resolve must not carry a resolve_hash for {bogus:?}"
        );
        assert_eq!(
            response.diagnostics.diagnostics[0].code,
            E_RESOLVE_SOLVER_MODEL_UNAVAILABLE,
            "resolve must surface the resolve model-unavailable code for {bogus:?}"
        );
    }
}

/// D1 (unloadable artifact case): a `.ccm` reference that points at a malformed
/// CCM directory — a `ccm.manifest.json` is present (so it resolves as a CCM
/// dir) but the v2 `partition-manifest.json` is absent (so the load errors) —
/// is treated as "no usable model" and `options` fails closed with
/// `E_SELECTION_SOLVER_MODEL_UNAVAILABLE`. This is distinct from the empty/
/// nonexistent ref: it proves an *unloadable* artifact also fails closed.
#[test]
fn dj7f_options_fails_closed_on_unloadable_ccm() {
    let cmp_dir = emitted_cmp_dir("dj7f-options-unloadable");
    let real_handle = open_handle(&cmp_dir);
    let base_state = init_selection_state_for_fixture(&real_handle, S1_SCOPE, s1_context_tags());

    let malformed = unique_fixture_dir("configflux-interp", "malformed-ccm");
    std::fs::write(
        malformed.join("ccm.manifest.json"),
        b"{\"not\":\"a real ccm\"}",
    )
    .expect("write malformed manifest");

    let handle = handle_with_ccm_ref(&real_handle, &malformed.to_string_lossy());
    let request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: S1_SCOPE.to_string(),
        selection_state: base_state,
        facet: "cooling_brand".to_string(),
        include_pruned_reasons: false,
    };
    let (output, response): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &request);
    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(response.status, OperationStatus::Error);
    assert_eq!(
        response.diagnostics.diagnostics[0].code,
        E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
        "an unloadable .ccm must fail closed with the model-unavailable code"
    );

    std::fs::remove_dir_all(&malformed).ok();
}

/// D5 guardrail (division of labor stays intact under ADR-0030): an
/// unconstrained facet — present in the model but absent from the CCM symbol
/// table — is still owned by the compiler and surfaces
/// `E_SELECTION_UNKNOWN_FACET`, NOT the new model-unavailable code. This proves
/// the fail-closed change did not over-reach into the legitimate delegations.
#[test]
fn dj7f_unknown_facet_still_delegates_to_compiler() {
    let cmp_dir = emitted_cmp_dir("dj7f-unknown-facet");
    let handle = open_handle(&cmp_dir);
    let base_state = init_selection_state_for_fixture(&handle, S1_SCOPE, s1_context_tags());

    let request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: S1_SCOPE.to_string(),
        selection_state: base_state,
        facet: "this_facet_does_not_exist".to_string(),
        include_pruned_reasons: false,
    };
    let (output, response): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &request);
    // The compiler owns the unknown-facet diagnostic; whatever it returns, it
    // must NOT be the solver model-unavailable code (the model is reachable).
    assert_ne!(
        response.diagnostics.diagnostics.first().map(|d| d.code.as_str()),
        Some(E_SELECTION_SOLVER_MODEL_UNAVAILABLE),
        "an unknown facet on a reachable model must not surface the unavailable code"
    );
    // Sanity: the command still resolves to a definite outcome (the handle is
    // valid and the facet is simply unconstrained).
    let _ = output.exit_code;
}

// ---------------------------------------------------------------------------
// configflux-whyt / ADR-0031: interpreter `explain` is now a read-side,
// solver-decided query routed through `solver_session::explain_via_solver`.
// These tests pin the M4 contract on the real product `.ccm` (so the solver
// path is genuinely exercised, not the legacy fallback):
//   * a genuine cross-facet constraint conflict returns a populated, fully
//     labeled `unsat_core` with exit 0 (ADR-0031 D2/D3);
//   * a no-usable-`.ccm` handle fails closed with exit 2 +
//     E_SELECTION_SOLVER_MODEL_UNAVAILABLE (ADR-0030 D1 / ADR-0031 D4);
//   * a rejection (blocked option) is the success path: exit 0, not exit 2
//     (ADR-0031 D2 — "a rejection is not an error").
//
// Conflict fixture (no new files): in S1, `cooling_model=a9` is feasible only
// with `cooling_brand=aeroflux`, and `cooling_model=x200` only with
// `cooling_brand=hydra`. So committing `cooling_brand=hydra` makes the
// in-domain option `cooling_model=a9` genuinely UNSATISFIABLE — a real
// constraint conflict the solver explains with a labeled MUS.
// ---------------------------------------------------------------------------

/// Assert that every facet/option string in an unsat core is a non-empty
/// labeled name — never a bare integer (the ADR-0031 D3 / configflux-osp schema
/// invariant: no raw BDD variable indices anywhere in the core).
fn assert_core_is_labeled(core: &UnsatCore) {
    let assert_labeled = |facet: &str, option: &str, where_: &str| {
        assert!(
            !facet.trim().is_empty() && !option.trim().is_empty(),
            "{where_}: labeled name must be non-empty, got facet={facet:?} option={option:?}"
        );
        assert!(
            facet.parse::<i64>().is_err(),
            "{where_}: facet must be a label, not a bare integer (got {facet:?})"
        );
        assert!(
            option.parse::<i64>().is_err(),
            "{where_}: option must be a label, not a bare integer (got {option:?})"
        );
    };
    assert_labeled(&core.rejected.facet, &core.rejected.option, "core.rejected");
    assert!(
        !core.conflicting_constraints.is_empty(),
        "a genuine conflict core must name at least one conflicting constraint"
    );
    for (i, c) in core.conflicting_constraints.iter().enumerate() {
        assert!(
            !c.facets.is_empty(),
            "conflicting_constraints[{i}] must name at least one facet"
        );
        for f in &c.facets {
            assert_labeled(&f.facet, &f.option, &format!("conflicting_constraints[{i}]"));
        }
    }
}

/// ADR-0031 D2/D3: explaining a genuinely-conflicting in-domain option returns
/// the labeled unsat core with exit 0. On the solver-conflict fixture (the
/// `combo_guard` constraint forbids exactly the `(hydra, a9)` combination),
/// selecting `cooling_brand=hydra` then explaining `cooling_model=a9` is a real
/// cross-facet conflict; the response must carry a populated `unsat_core` whose
/// facet/option values are all labeled (no integers), and the rejection code
/// must be the genuine-conflict family.
///
/// This is the executable spec of the populated-core behavior, written against
/// the final contract. It runs live against the real solver path: it exercises
/// `compile_model -> .ccm -> Session::explain_rejection` end to end, the path
/// that depends on the configflux-autp fix (solver/src/explain.rs now resolves
/// the index-based ⊥/⊤ terminal references the real BDD serializer emits, so the
/// MUS walker no longer faults on a compiler-emitted `.ccm`).
#[test]
fn whyt_explain_conflict_returns_labeled_unsat_core_exit_zero() {
    let (_cmp_dir, handle) = open_solver_conflict_handle("whyt-explain-conflict");

    // Hard guard: the solver path must be reachable, else this test is vacuous.
    assert!(
        !handle.ccm_ref.trim().is_empty()
            && Path::new(handle.ccm_ref.trim())
                .join("ccm.manifest.json")
                .is_file(),
        "open must advertise a usable .ccm so the solver explain path is exercised"
    );

    // Commit cooling_brand=hydra (an accepted selection), producing a valid,
    // hash-pinned next state.
    let state_after_hydra = solver_conflict_state_after_hydra(&handle);

    // Explain cooling_model=a9: in-domain, but unsatisfiable under hydra (the
    // `combo_guard` constraint forbids the hydra+a9 pair).
    let explain_request = ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: SOLVER_CONFLICT_SCOPE.to_string(),
        selection_state: state_after_hydra,
        rejected_option: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "a9".to_string(),
        },
    };
    let (output, response): (RunOutput, ExplainRejectionResult) =
        run_json_command(&["explain"], &explain_request);

    // A rejection explanation is a successful query (ADR-0031 D2): exit 0.
    assert_eq!(
        output.exit_code, EXIT_OK,
        "explaining a rejected option is the success path, not a command error"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    assert_eq!(response.facet, "cooling_model");
    assert_eq!(response.option, "a9");

    // Genuine-conflict family + populated, fully-labeled core (ADR-0031 D3).
    assert!(
        response.rejection.code == E_SELECTION_CONFLICT
            || response.rejection.code == E_SELECTION_UNSATISFIABLE,
        "a genuine cross-facet conflict must use the conflict family, got '{}'",
        response.rejection.code
    );
    let core = response
        .rejection
        .unsat_core
        .as_ref()
        .expect("a genuine constraint conflict must carry a populated unsat_core");
    assert_eq!(core.rejected.facet, "cooling_model");
    assert_eq!(core.rejected.option, "a9");
    assert!(core.minimal, "M4 extraction yields a minimal subset");
    assert!(!core.note.trim().is_empty(), "the advisory note must be set");
    assert_core_is_labeled(core);

    // The prior committed choice that blocks a9 must surface as a `selection`
    // constraint naming cooling_brand.hydra — proving the core reflects the
    // replayed selection state, not just static model rules.
    let names_hydra_selection = core.conflicting_constraints.iter().any(|c| {
        c.kind == ConstraintKind::Selection
            && c.facets
                .iter()
                .any(|f| f.facet == "cooling_brand" && f.option == "hydra")
    });
    assert!(
        names_hydra_selection,
        "the unsat core must name the conflicting prior selection cooling_brand=hydra; got {:?}",
        core.conflicting_constraints
    );
}

/// ADR-0031 D2/D3 (determinism of shape): the same conflict explain request
/// produces a byte-identical response and a populated core in both stdin and
/// file-I/O transport modes. Keys the assertion on the labeled shape, not an
/// exact MUS witness (ADR-0031 D3 deliberately does not promise cross-run
/// witness identity), but on this fixture the witness is in fact stable. Runs
/// live against the real solver path alongside
/// `whyt_explain_conflict_returns_labeled_unsat_core_exit_zero` (both depend on
/// the configflux-autp index-based-terminal fix).
#[test]
fn whyt_explain_conflict_is_deterministic_and_file_mode_parity() {
    let (_cmp_dir, handle) = open_solver_conflict_handle("whyt-explain-determinism");
    let state_after_hydra = solver_conflict_state_after_hydra(&handle);

    let explain_request = ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: SOLVER_CONFLICT_SCOPE.to_string(),
        selection_state: state_after_hydra,
        rejected_option: SelectionDelta {
            facet: "cooling_model".to_string(),
            option: "a9".to_string(),
        },
    };
    let response: ExplainRejectionResult = assert_command_determinism_and_file_mode(
        "explain",
        &explain_request,
        "whyt-explain-determinism",
    );
    assert_eq!(response.status, OperationStatus::Ok);
    assert!(
        response.rejection.unsat_core.is_some(),
        "the conflict explain must carry a populated unsat_core in both transport modes"
    );
}

/// ADR-0030 D1 / ADR-0031 D4: `explain` fails closed with exit 2 +
/// E_SELECTION_SOLVER_MODEL_UNAVAILABLE when no usable solver model is
/// reachable — for both an empty ref and a bogus (unloadable) path. There is no
/// "degraded explanation." The selection state is otherwise valid (derived from
/// a real handle); only the `ccm_ref` is clobbered.
#[test]
fn whyt_explain_fails_closed_when_ccm_unavailable() {
    let cmp_dir = emitted_cmp_dir("whyt-explain-unavailable");
    let real_handle = open_handle(&cmp_dir);
    let base_state = init_selection_state_for_fixture(&real_handle, S1_SCOPE, s1_context_tags());

    for bogus in ["", "/nonexistent/configflux/whyt/ccm"] {
        let handle = handle_with_ccm_ref(&real_handle, bogus);
        let request = ExplainRejectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle,
            scope: S1_SCOPE.to_string(),
            selection_state: base_state.clone(),
            rejected_option: SelectionDelta {
                facet: "cooling_model".to_string(),
                option: "a9".to_string(),
            },
        };
        let (output, response): (RunOutput, ExplainRejectionResult) =
            run_json_command(&["explain"], &request);
        assert_eq!(
            output.exit_code, EXIT_COMMAND_ERROR,
            "explain must fail closed (exit 2) for ccm_ref {bogus:?}"
        );
        assert!(output.stderr.is_empty());
        assert_eq!(response.status, OperationStatus::Error);
        assert_eq!(
            response.rejection.code, E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
            "explain must surface the model-unavailable code for {bogus:?}"
        );
        assert!(
            response.rejection.unsat_core.is_none(),
            "a fail-closed explain must not carry a core for {bogus:?}"
        );
        assert_eq!(
            response.diagnostics.diagnostics[0].code, E_SELECTION_SOLVER_MODEL_UNAVAILABLE,
            "explain diagnostics must carry the model-unavailable code for {bogus:?}"
        );
    }
}

/// ADR-0031 D2: a division-of-labor rejection (invalid option — outside the
/// facet domain) is still the success path: exit 0, status ok, the canonical
/// `E_SELECTION_*` rejection code, and NO solver core (there is no constraint
/// conflict to minimize). This pins the "a rejection is not an error" rule for
/// the compiler-owned rejection family on a reachable model.
#[test]
fn whyt_explain_invalid_option_rejection_is_exit_zero_no_core() {
    let cmp_dir = emitted_cmp_dir("whyt-explain-invalid-option");
    let handle = open_handle(&cmp_dir);
    let base_state = init_selection_state_for_fixture(&handle, S1_SCOPE, s1_context_tags());

    let request = ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: S1_SCOPE.to_string(),
        selection_state: base_state,
        rejected_option: SelectionDelta {
            facet: "cooling_brand".to_string(),
            option: "nonexistent_brand".to_string(),
        },
    };
    let (output, response): (RunOutput, ExplainRejectionResult) =
        run_json_command(&["explain"], &request);

    assert_eq!(
        output.exit_code, EXIT_OK,
        "explaining an invalid option is a successful query (ADR-0031 D2), not exit 2"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    assert!(
        response.rejection.code.starts_with("E_SELECTION_"),
        "the rejection must use the selection family, got '{}'",
        response.rejection.code
    );
    assert!(
        response.rejection.unsat_core.is_none(),
        "a division-of-labor invalid-option rejection carries no solver core"
    );
}

// ---------------------------------------------------------------------------
// configflux-vwhj — labeled-MUS exit-criterion E2E (interpreter `explain`).
//
// The configflux-osp M4 epic's HARD EXIT CRITERION on the interpreter path:
// piping a well-formed ExplainRejectionRequest through the `explain` command
// over the committed THREE-facet cross-facet-`requires` fixture
// (compiler/scenarios/s_labeled_mus/) must return an `unsat_core` whose every
// facet/option position is a labeled name — NEVER a raw BDD variable index
// (a bare integer) — and the response SHAPE must be deterministic (ADR-0031
// D3). The SAME committed fixture is exercised by the solver-level invariant
// test (//solver:labeled_mus_test) and the runtime path (run_048).
// ---------------------------------------------------------------------------

const VWHJ_DEFS: &str =
    include_str!("../../compiler/scenarios/s_labeled_mus/00_definitions.json");
const VWHJ_COMPONENTS: &str =
    include_str!("../../compiler/scenarios/s_labeled_mus/10_components.json");
const VWHJ_SCOPE: &str = "component:rig";

/// Compile and open the committed labeled-MUS fixture, returning the handle.
fn open_labeled_mus_handle(label: &str) -> (TempDirGuard, ModelHandle) {
    let cmp_dir = emitted_cmp_dir_with_chunks(
        label,
        "scenarios/s_labeled_mus/00_definitions.toml",
        "scenarios/s_labeled_mus/10_components.toml",
        VWHJ_DEFS,
        VWHJ_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);
    (cmp_dir, handle)
}

/// Walk an `unsat_core` JSON value and assert NO `facet`/`option` string in
/// `rejected` or any `conflicting_constraints[*].facets[*]` is a bare integer
/// — the configflux-osp labeled-MUS exit-criterion invariant (ADR-0031 D3).
fn vwhj_assert_no_integer_atoms(core_json: &serde_json::Value) {
    let check_atom = |atom: &serde_json::Value| {
        for key in ["facet", "option"] {
            if let Some(s) = atom.get(key).and_then(|v| v.as_str()) {
                assert!(
                    !s.is_empty() && s.parse::<i64>().is_err(),
                    "core atom {key} must be a labeled name, never a raw BDD \
                     index: {s:?}"
                );
            }
        }
    };
    check_atom(core_json.get("rejected").expect("core has a rejected atom"));
    let constraints = core_json
        .get("conflicting_constraints")
        .and_then(|v| v.as_array())
        .expect("core has conflicting_constraints");
    assert!(
        !constraints.is_empty(),
        "a genuine conflict core must name at least one conflicting constraint"
    );
    for constraint in constraints {
        let facets = constraint
            .get("facets")
            .and_then(|v| v.as_array())
            .expect("each conflicting constraint has facets");
        assert!(!facets.is_empty(), "each constraint must name at least one facet");
        for atom in facets {
            check_atom(atom);
        }
    }
}

/// Build the ExplainRejectionRequest that exercises the genuine cross-facet
/// conflict: pin `cpu=highperf` (a feasible selection that REQUIRES
/// `cooling.liquid`), then explain `cooling=air` (in-domain, but unsatisfiable
/// under the pin).
fn vwhj_conflict_request(handle: &ModelHandle) -> ExplainRejectionRequest {
    let base = init_selection_state_for_fixture(handle, VWHJ_SCOPE, BTreeMap::new());
    let after = apply_selection_step_for_scope(handle, VWHJ_SCOPE, base, "cpu", "highperf");
    ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: VWHJ_SCOPE.to_string(),
        selection_state: after,
        rejected_option: SelectionDelta {
            facet: "cooling".to_string(),
            option: "air".to_string(),
        },
    }
}

#[test]
fn vwhj_interpreter_explain_labeled_mus_has_no_raw_indices() {
    let (_cmp_dir, handle) = open_labeled_mus_handle("vwhj-interp-explain");

    // The solver explain path must be reachable, else this test is vacuous.
    assert!(
        !handle.ccm_ref.trim().is_empty()
            && Path::new(handle.ccm_ref.trim())
                .join("ccm.manifest.json")
                .is_file(),
        "open must advertise a usable .ccm so the solver explain path is exercised"
    );

    let (output, response): (RunOutput, ExplainRejectionResult) =
        run_json_command(&["explain"], &vwhj_conflict_request(&handle));

    // A rejection explanation is a successful query (ADR-0031 D2): exit 0.
    assert_eq!(
        output.exit_code, EXIT_OK,
        "explaining a rejected option is the success path; stderr={}",
        output.stderr
    );
    assert!(output.stderr.is_empty());
    assert_eq!(response.status, OperationStatus::Ok);
    assert_eq!(response.facet, "cooling");
    assert_eq!(response.option, "air");
    assert!(
        response.rejection.code == E_SELECTION_CONFLICT
            || response.rejection.code == E_SELECTION_UNSATISFIABLE,
        "a genuine cross-facet conflict must use the conflict family, got '{}'",
        response.rejection.code
    );

    let core = response
        .rejection
        .unsat_core
        .as_ref()
        .expect("a genuine cross-facet conflict must carry a populated unsat_core");

    // Typed-level assertions (the labeled UnsatCore surface).
    assert_eq!(core.rejected.facet, "cooling");
    assert_eq!(core.rejected.option, "air");
    assert!(core.minimal, "M4 extraction yields a minimal subset");
    assert_core_is_labeled(core);

    // THE INVARIANT, on the serialized JSON: no integer in any facet/option
    // position anywhere in the unsat_core (the exit-criterion guard).
    let core_json = serde_json::to_value(core).expect("serialize core");
    vwhj_assert_no_integer_atoms(&core_json);
}

#[test]
fn vwhj_interpreter_explain_labeled_mus_shape_is_deterministic() {
    let (_cmp_dir, handle) = open_labeled_mus_handle("vwhj-interp-determinism");

    // ADR-0031 D3: keys the assertion on the labeled response SHAPE (a
    // populated, fully-labeled core in both stdin and file-I/O transport
    // modes), not an exact MUS witness. `assert_command_determinism_and_file_mode`
    // additionally proves the two transports return byte-identical responses.
    let response: ExplainRejectionResult = assert_command_determinism_and_file_mode(
        "explain",
        &vwhj_conflict_request(&handle),
        "vwhj-interp-determinism",
    );
    assert_eq!(response.status, OperationStatus::Ok);
    let core = response
        .rejection
        .unsat_core
        .as_ref()
        .expect("the conflict explain must carry a populated unsat_core in both transport modes");
    assert_core_is_labeled(core);
    let core_json = serde_json::to_value(core).expect("serialize core");
    vwhj_assert_no_integer_atoms(&core_json);
}

// ---------------------------------------------------------------------------
// ADR-0054 §2/§6 (configflux-4sjk) — the interpreter's `resolve` verb enforces
// declared constraints, reporting the SAME code the `cfx` surface reports.
//
// This is the machine/agent seam, and it is a genuinely independent path from
// `cfx`: `session_compose::resolve` gates satisfiability with the solver and,
// on EITHER verdict, delegates the response bytes to the compiler's
// `resolve_from_selection`. That delegation assumed the two engines agree —
// and for a policy constraint they did not: the solver could hold a selection
// unsatisfiable while the compiler composed a snapshot for it anyway. These
// tests pin the agreement on the resolve verb.
// ---------------------------------------------------------------------------

const CONSTRAINT_SOURCE_DEFS: &str = "00_definitions.json";
const CONSTRAINT_SOURCE_COMPONENTS: &str = "10_components.json";
const CONSTRAINT_SCOPE: &str = "all";

const CONSTRAINT_CHUNK_DEFS: &str = r#"{
  "package": "policy_demo",
  "version": "1.0.0",
  "definitions": {
    "log_sink": {
      "type": "string",
      "lifecycle": "startup",
      "access": "integrator",
      "doc": "Where the service writes its log stream"
    }
  },
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },
  "constraints": {
    "prod_forbids_debug": {
      "condition": "environment != 'prod' || log_level != 'debug'",
      "doc": "Debug logging is not permitted in production."
    }
  }
}"#;

const CONSTRAINT_CHUNK_COMPONENTS: &str = r#"{
  "package": "policy_demo",
  "version": "1.0.0",
  "components": {
    "webapp": {
      "type": "service",
      "params": {
        "log_sink": {
          "inherits": "log_sink",
          "type": "string",
          "lifecycle": "startup",
          "access": "integrator",
          "doc": "Where the service writes its log stream",
          "value": "stdout"
        }
      }
    }
  }
}"#;

fn constraint_selection_state(model_hash: &str, choices: &[(&str, &str)]) -> SelectionState {
    canonical_selection_state(
        model_hash.to_string(),
        CONSTRAINT_SCOPE.to_string(),
        BTreeMap::new(),
        choices
            .iter()
            .map(|(facet, option)| (facet.to_string(), option.to_string()))
            .collect(),
    )
    .expect("constraint selection state")
}

#[test]
fn interpreter_resolve_rejects_a_constraint_violating_selection() {
    let cmp_dir = emitted_cmp_dir_with_chunks(
        "constraint-resolve",
        CONSTRAINT_SOURCE_DEFS,
        CONSTRAINT_SOURCE_COMPONENTS,
        CONSTRAINT_CHUNK_DEFS,
        CONSTRAINT_CHUNK_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);
    let request = ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: CONSTRAINT_SCOPE.to_string(),
        selection_state: constraint_selection_state(
            &handle.model_hash,
            &[("environment", "prod"), ("log_level", "debug")],
        ),
        implied_choices: Default::default(),
    };

    let (output, response): (RunOutput, ResolveResult) = run_json_command(&["resolve"], &request);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(response.status, OperationStatus::Error);
    // Nothing to export and nothing to pin: a rejected resolve produces no
    // artifact lineage at all.
    assert!(response.resolve_hash.is_none());
    assert!(response.resolved_output.is_none());

    let diagnostic = &response.diagnostics.diagnostics[0];
    assert_eq!(
        diagnostic.code, E_SELECTION_CONFLICT,
        "the interpreter must report the SAME code cfx does, not a new one"
    );
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    assert_eq!(
        diagnostic.entity_path.as_deref(),
        Some("constraints/prod_forbids_debug")
    );
    assert!(
        diagnostic.message.contains("prod_forbids_debug")
            && diagnostic
                .message
                .contains("environment != 'prod' || log_level != 'debug'"),
        "message must name the constraint and quote its condition: {}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.source_id.as_deref(),
        Some(CONSTRAINT_SOURCE_DEFS),
        "ADR-0054 §6: source_id is the chunk that declared the constraint"
    );
}

#[test]
fn interpreter_resolve_accepts_a_selection_the_policy_permits() {
    // The control: same model, same facets, a legal combination. Fail-closed
    // must mean "closed on violations", not "closed".
    let cmp_dir = emitted_cmp_dir_with_chunks(
        "constraint-resolve-ok",
        CONSTRAINT_SOURCE_DEFS,
        CONSTRAINT_SOURCE_COMPONENTS,
        CONSTRAINT_CHUNK_DEFS,
        CONSTRAINT_CHUNK_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);
    let response = resolve_ok_for_scope(
        &handle,
        CONSTRAINT_SCOPE,
        constraint_selection_state(
            &handle.model_hash,
            &[("environment", "dev"), ("log_level", "debug")],
        ),
    );

    assert!(response.resolve_hash.is_some());
    assert!(response.diagnostics.diagnostics.is_empty());
}

#[test]
fn interpreter_select_rejects_a_constraint_violating_choice_as_a_conflict() {
    // configflux-narb, through the real binary on the real `.ccm`. The solver
    // has rejected this choice since the policy became a root conjunct; the
    // LOADER used to accept it, so `session_compose::apply` saw solver-REJECT vs
    // legacy-ACCEPT and fell closed with `E_SELECTION_ENGINE_DIVERGENCE` — the
    // INTERNAL-FAULT family, hinting "recompile the model", for a model that was
    // simply doing what it says. The two engines now answer the same question
    // the same way, so what reaches the machine seam is the policy verdict.
    let cmp_dir = emitted_cmp_dir_with_chunks(
        "constraint-select",
        CONSTRAINT_SOURCE_DEFS,
        CONSTRAINT_SOURCE_COMPONENTS,
        CONSTRAINT_CHUNK_DEFS,
        CONSTRAINT_CHUNK_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);

    // Step 1: environment=prod alone decides no constraint — it must apply.
    let first = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: CONSTRAINT_SCOPE.to_string(),
        selection_state: constraint_selection_state(&handle.model_hash, &[]),
        selection_delta: SelectionDelta {
            facet: "environment".to_string(),
            option: "prod".to_string(),
        },
    };
    let (first_output, first_cli): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &first);
    assert_eq!(first_output.exit_code, EXIT_OK);
    assert_eq!(first_cli.status, OperationStatus::Ok);
    let after_prod = first_cli.selection_state.expect("state after prod");

    // Step 2: log_level=debug decides it, and breaks it.
    let second = ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: CONSTRAINT_SCOPE.to_string(),
        selection_state: after_prod,
        selection_delta: SelectionDelta {
            facet: "log_level".to_string(),
            option: "debug".to_string(),
        },
    };
    let (output, response): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &second);

    assert_eq!(output.exit_code, EXIT_COMMAND_ERROR);
    assert_eq!(response.status, OperationStatus::Error);
    assert!(response.selection_state.is_none());

    let diagnostic = &response.diagnostics.diagnostics[0];
    assert_eq!(
        diagnostic.code, E_SELECTION_CONFLICT,
        "a policy rejection is a selection conflict, NOT an engine-divergence \
         incident: got '{}'",
        diagnostic.code
    );
    assert_eq!(
        diagnostic.entity_path.as_deref(),
        Some("constraints/prod_forbids_debug")
    );
    assert!(
        diagnostic.message.contains("prod_forbids_debug")
            && diagnostic
                .message
                .contains("environment != 'prod' || log_level != 'debug'"),
        "select must name the constraint and quote it, exactly as resolve does: {}",
        diagnostic.message
    );
}

#[test]
fn interpreter_options_and_select_agree_about_a_constraint() {
    // Three-surface agreement on ONE selection, measured on the seam where the
    // solver and the loader both have a vote: `options` is solver-authoritative
    // and `select` is adjudicated by the solver with the loader rendering the
    // rejection. If they disagreed, `select` could refuse an option `options`
    // had just offered — or offer one it would refuse.
    let cmp_dir = emitted_cmp_dir_with_chunks(
        "constraint-options",
        CONSTRAINT_SOURCE_DEFS,
        CONSTRAINT_SOURCE_COMPONENTS,
        CONSTRAINT_CHUNK_DEFS,
        CONSTRAINT_CHUNK_COMPONENTS,
    );
    let handle = open_handle(&cmp_dir);

    // With nothing selected the policy is undecided, so both arms stand.
    let open_request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: CONSTRAINT_SCOPE.to_string(),
        selection_state: constraint_selection_state(&handle.model_hash, &[]),
        facet: "log_level".to_string(),
        include_pruned_reasons: false,
    };
    let (open_output, open_cli): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &open_request);
    assert_eq!(open_output.exit_code, EXIT_OK);
    assert_eq!(open_cli.status, OperationStatus::Ok);
    assert!(
        open_cli.valid_options.contains(&"debug".to_string()),
        "debug is legal until something makes it illegal: {:?}",
        open_cli.valid_options
    );

    // Under prod it is gone — the solver's per-arm SAT query over the `.ccm`
    // root, whose synthesized intra-facet cardinality (ADR-0054 §5.2) is what
    // makes selecting `environment=prod` exclude the other environments and so
    // makes the policy bite.
    let prod_request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: CONSTRAINT_SCOPE.to_string(),
        selection_state: constraint_selection_state(
            &handle.model_hash,
            &[("environment", "prod")],
        ),
        facet: "log_level".to_string(),
        include_pruned_reasons: false,
    };
    let (prod_output, prod_cli): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &prod_request);
    assert_eq!(prod_output.exit_code, EXIT_OK);
    assert_eq!(
        prod_cli.valid_options,
        vec!["info".to_string()],
        "options must not offer what select would refuse"
    );
}

// ---------------------------------------------------------------------------
// configflux-7uex — the tamper family at the interpreter seam.
//
// INT-004 above drives ONE verb (`explain`) with ONE tampered state. The seam
// is where a hand-authored `SelectionState` actually arrives: `select`,
// `options`, `explain` and `resolve` each deserialize the whole state from
// stdin, so every field of it is caller-supplied and nothing between the JSON
// and the engine re-derives it. The unit-level twins live in
// `session_compose/src/tests.rs`; what these add is the pass over the real CLI
// — argv in, a JSON request on stdin, an exit code and a diagnostic code out.
//
// Two families, because the two refusals are decided by different things.
//
//   A. A forged integrity binding (configflux-q50t). The state does not hash to
//      what it says it does, so `validate_selection_state` refuses it without
//      ever consulting the model. `explain` already returned the compiler's
//      envelope for this; `select` composed its own `Ok` over it and `options`
//      published an option list computed from it, which is why the class went
//      unnoticed until a diff review found it.
//
//   B. An admissible-LOOKING state (configflux-eclx, ADR-0030 Amendment 2
//      Rule 1). Every one of q50t's six bindings passes — the hash is
//      canonical, the model and scope match and are non-empty, the tags do not
//      contradict the choices — and the state still names a facet the model
//      does not have, or gives a facet a value outside its domain. Not one of
//      the six is a model check, so only the model can refuse this, and the
//      replay cannot: it silently skips an assignment it cannot hold, so by the
//      time the solver has answered, the offending entry is gone from the
//      deployment being reasoned about.
//
// Family B is driven on all four verbs; family A adds the two `explain` does
// not cover. Every case asserts the process exit code as well as the envelope,
// because the exit code is the whole contract for a caller that pipes JSON and
// reads `$?` — a refusal that surfaced as exit 0 would be a fail-open however
// correct the diagnostic inside it was.
// ---------------------------------------------------------------------------

/// An S1 state sealed canonically against `model_hash` and `S1_SCOPE`, over
/// caller-authored `choices` — the shape a hand-authored request carries at the
/// seam. `s1_context_tags` rides along unchanged so the state stays realistic:
/// `region` is a condition-only facet no chunk declares, so the screen leaves
/// it alone (ADR-0030 Amendment 2 Rule 1 screens tags only where the model
/// declares a CLOSED domain), and the one planted choice is the only thing the
/// model can object to.
fn s1_authored_selection_state(model_hash: &str, choices: &[(&str, &str)]) -> SelectionState {
    canonical_selection_state(
        model_hash.to_string(),
        S1_SCOPE.to_string(),
        s1_context_tags(),
        choices
            .iter()
            .map(|(facet, option)| (facet.to_string(), option.to_string()))
            .collect(),
    )
    .expect("authored selection state")
}

/// The first diagnostic's code on a report, as a placeholder-bearing accessor
/// rather than an index, so a failing assertion names what actually came back
/// instead of panicking on an empty vector before it can say anything.
fn first_diagnostic_code(report: &DiagnosticsReport) -> &str {
    report
        .diagnostics
        .first()
        .map(|diagnostic| diagnostic.code.as_str())
        .unwrap_or("<no diagnostic>")
}

/// A `select` request over `state`, with a delta the model admits and the state
/// does not already carry. Both halves matter: an out-of-domain delta would let
/// the delta's own refusal stand in for the state's, and a delta on a facet the
/// state already holds short-circuits `session_compose::apply` straight to the
/// compiler, so the wrapper's own screen would never be the thing under test.
fn s1_select_request(handle: &ModelHandle, state: &SelectionState) -> ApplySelectionRequest {
    ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: state.clone(),
        selection_delta: SelectionDelta {
            facet: "cooling_brand".to_string(),
            option: "hydra".to_string(),
        },
    }
}

/// An `options` request over `state` for a facet the model knows, so an
/// `E_SELECTION_UNKNOWN_FACET` that comes back is the STATE's and never the
/// probe's.
fn s1_options_request(handle: &ModelHandle, state: &SelectionState) -> GetSelectionOptionsRequest {
    GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: state.clone(),
        facet: "cooling_brand".to_string(),
        include_pruned_reasons: false,
    }
}

/// An `explain` request over `state` probing an in-domain `(facet, option)`,
/// for the same reason `s1_options_request` probes a known facet.
fn s1_explain_request(handle: &ModelHandle, state: &SelectionState) -> ExplainRejectionRequest {
    ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: state.clone(),
        rejected_option: SelectionDelta {
            facet: "cooling_brand".to_string(),
            option: "aeroflux".to_string(),
        },
    }
}

/// A `resolve` request over `state`, with no implied choices — the
/// caller-supplied map configflux-v93p screens separately, held empty so the
/// only thing under screen here is the state.
fn s1_resolve_request(handle: &ModelHandle, state: &SelectionState) -> ResolveFromSelectionRequest {
    ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: S1_SCOPE.to_string(),
        selection_state: state.clone(),
        implied_choices: Default::default(),
    }
}

/// Every selection verb the CLI exposes, driven over `state` through the real
/// binary, each asserted to fail closed with `code`: exit 2 (command error),
/// nothing on stderr, `status: error` in the envelope, and `code` as the first
/// diagnostic. `explain` reports its refusal in `rejection` rather than
/// `diagnostics` (ADR-0031 D2 makes a request that could not run a command
/// error, not a rejection verdict), so its accessor differs; the contract does
/// not.
fn assert_every_verb_refuses_over_the_cli(
    handle: &ModelHandle,
    state: &SelectionState,
    code: &str,
) {
    let (select_output, selected): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &s1_select_request(handle, state));
    assert_eq!(
        select_output.exit_code, EXIT_COMMAND_ERROR,
        "select exit code"
    );
    assert!(
        select_output.stderr.is_empty(),
        "select stderr must stay empty"
    );
    assert_eq!(selected.status, OperationStatus::Error, "select status");
    assert_eq!(
        first_diagnostic_code(&selected.diagnostics),
        code,
        "select: {:?}",
        selected.diagnostics
    );
    assert!(
        selected.selection_state.is_none(),
        "select must not hand back a re-canonicalized state for one it refused"
    );

    let (options_output, listed): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &s1_options_request(handle, state));
    assert_eq!(
        options_output.exit_code, EXIT_COMMAND_ERROR,
        "options exit code"
    );
    assert!(
        options_output.stderr.is_empty(),
        "options stderr must stay empty"
    );
    assert_eq!(listed.status, OperationStatus::Error, "options status");
    assert_eq!(
        first_diagnostic_code(&listed.diagnostics),
        code,
        "options: {:?}",
        listed.diagnostics
    );
    assert!(
        listed.valid_options.is_empty(),
        "an error envelope carries no options: {:?}",
        listed.valid_options
    );

    let (explain_output, explained): (RunOutput, ExplainRejectionResult) =
        run_json_command(&["explain"], &s1_explain_request(handle, state));
    assert_eq!(
        explain_output.exit_code, EXIT_COMMAND_ERROR,
        "explain exit code"
    );
    assert!(
        explain_output.stderr.is_empty(),
        "explain stderr must stay empty"
    );
    assert_eq!(explained.status, OperationStatus::Error, "explain status");
    assert_eq!(
        explained.rejection.code, code,
        "explain: {:?}",
        explained.rejection
    );

    let (resolve_output, resolved): (RunOutput, ResolveResult) =
        run_json_command(&["resolve"], &s1_resolve_request(handle, state));
    assert_eq!(
        resolve_output.exit_code, EXIT_COMMAND_ERROR,
        "resolve exit code"
    );
    assert!(
        resolve_output.stderr.is_empty(),
        "resolve stderr must stay empty"
    );
    assert_eq!(resolved.status, OperationStatus::Error, "resolve status");
    assert_eq!(
        first_diagnostic_code(&resolved.diagnostics),
        code,
        "resolve: {:?}",
        resolved.diagnostics
    );
}

/// Family A at the seam — the two verbs INT-004 does not reach.
///
/// The state is `s1_empty_selection_state` with its hash overwritten, and the
/// delta names a facet it does not carry, which is precisely the
/// configflux-q50t shape: with the delta absent from both maps,
/// `session_compose::apply` used to take the solver's accept and emit its OWN
/// `Ok` envelope plus a freshly canonicalized hash, never asking the compiler
/// that would have refused the state. `options` had the mirror-image hole —
/// legacy's `Error` status survived its merge, but the solver's `valid_options`
/// replaced the empty list the refusal carried, publishing options enumerated
/// over an environment the engine had just called incoherent.
#[test]
fn interpreter_select_and_options_refuse_a_forged_selection_state_hash() {
    let cmp_dir = emitted_cmp_dir("7uex-forged-hash");
    let handle = open_handle(&cmp_dir);
    let mut forged = s1_empty_selection_state(&handle.model_hash);
    forged.selection_state_hash = "00".repeat(32);

    let (select_output, selected): (RunOutput, ApplySelectionResult) =
        run_json_command(&["select"], &s1_select_request(&handle, &forged));
    assert_eq!(select_output.exit_code, EXIT_COMMAND_ERROR);
    assert!(select_output.stderr.is_empty());
    assert_eq!(selected.status, OperationStatus::Error);
    assert_eq!(
        first_diagnostic_code(&selected.diagnostics),
        E_SELECTION_STATE_INVALID,
        "select: {:?}",
        selected.diagnostics
    );
    assert!(
        selected.selection_state.is_none(),
        "select must not re-canonicalize a state whose seal does not hold"
    );

    let (options_output, listed): (RunOutput, GetSelectionOptionsResult) =
        run_json_command(&["options"], &s1_options_request(&handle, &forged));
    assert_eq!(options_output.exit_code, EXIT_COMMAND_ERROR);
    assert!(options_output.stderr.is_empty());
    assert_eq!(listed.status, OperationStatus::Error);
    assert_eq!(
        first_diagnostic_code(&listed.diagnostics),
        E_SELECTION_STATE_INVALID,
        "options: {:?}",
        listed.diagnostics
    );
    assert!(
        listed.valid_options.is_empty(),
        "options must publish nothing over a state it refused: {:?}",
        listed.valid_options
    );
}

/// Family B at the seam, unknown-facet half — a state naming a facet the model
/// has no symbol for at all. Nothing about it is malformed: it hashes to what
/// it claims, it is sealed against this model and scope, and its tags agree
/// with its choices. The `.ccm` simply has no variable for `nosuch_facet`, so
/// the replay skips it and every verb would otherwise answer about a deployment
/// carrying a facet that does not exist.
#[test]
fn interpreter_every_verb_refuses_a_choice_on_an_unknown_facet() {
    let cmp_dir = emitted_cmp_dir("7uex-unknown-facet");
    let handle = open_handle(&cmp_dir);
    let state = s1_authored_selection_state(&handle.model_hash, &[("nosuch_facet", "x")]);

    assert_every_verb_refuses_over_the_cli(&handle, &state, E_SELECTION_UNKNOWN_FACET);
}

/// Family B at the seam, out-of-domain half — a facet the model does know,
/// carrying a value it does not admit. S1's `cooling_model` is inferred from
/// the `control_driver` override conditions, so its domain is exactly
/// `{a9, x200}` and `x999` is outside it. The caller's own choices never widen
/// that domain — it is read from the model alone — which is what makes this
/// refusable at all.
#[test]
fn interpreter_every_verb_refuses_a_choice_outside_its_facets_domain() {
    let cmp_dir = emitted_cmp_dir("7uex-out-of-domain");
    let handle = open_handle(&cmp_dir);
    let state = s1_authored_selection_state(&handle.model_hash, &[("cooling_model", "x999")]);

    assert_every_verb_refuses_over_the_cli(&handle, &state, E_SELECTION_INVALID_OPTION);
}
