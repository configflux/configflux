// SPDX-License-Identifier: BUSL-1.1

use super::*;
use crate::Compiler;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../../scenarios/s1_water_pump/smoke/cue/10_components.json");

fn emitted_cmp_dir() -> (PathBuf, ir::IrIndex) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "configflux-loop2-loader-open-model-{}-{}",
        std::process::id(),
        unique
    ));

    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto(S1_SOURCE_DEFS, S1_CHUNK_DEFS)
        .expect("add defs chunk");
    compiler
        .add_chunk_auto(S1_SOURCE_COMPONENTS, S1_CHUNK_COMPONENTS)
        .expect("add components chunk");

    let index = compiler.emit_ir(&temp_dir).expect("emit ir");
    (temp_dir, index)
}

fn cmp_manifest_path(dir: &Path) -> PathBuf {
    dir.join(ir::CMP_DEFAULT_MANIFEST_FILENAME)
}

fn load_json(path: &Path) -> JsonValue {
    let bytes = std::fs::read(path).expect("read json");
    serde_json::from_slice(&bytes).expect("parse json")
}

fn write_json(path: &Path, value: &JsonValue) {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

#[test]
fn open_model_accepts_valid_cmp_manifest() {
    let (temp_dir, index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Ok);
    assert_eq!(result.model_hash, Some(index.config_hash.clone()));
    let handle = result.model_handle.expect("model_handle");
    assert_eq!(handle.model_hash, index.config_hash);
    assert!(Path::new(&handle.index_ref).exists());
    assert!(Path::new(&handle.chunk_set_ref).exists());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn open_model_rejects_manifest_hash_mismatch() {
    let (temp_dir, _index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);

    let mut manifest_json = load_json(&manifest_path);
    manifest_json["model_hash"] = JsonValue::String("deadbeef".repeat(8));
    write_json(&manifest_path, &manifest_json);

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(result.error_count, 1);
    assert_eq!(
        result.diagnostics.diagnostics[0].code,
        E_LOADER_MANIFEST_INCONSISTENT.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn open_model_rejects_corrupt_index_hash() {
    let (temp_dir, _index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);
    let index_path = temp_dir.join("index.cfir.json");

    let mut index_json = load_json(&index_path);
    index_json["config_hash"] = JsonValue::String("00".repeat(32));
    write_json(&index_path, &index_json);

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(result.error_count, 1);
    assert_eq!(
        result.diagnostics.diagnostics[0].code,
        E_LOADER_INDEX_INVALID.to_string()
    );
    assert!(result.diagnostics.diagnostics[0]
        .message
        .contains("does not match computed hash"));

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn open_model_rejects_missing_chunk_file() {
    let (temp_dir, index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);

    let chunk_hash = index.chunks.first().expect("chunk").chunk_hash.clone();
    let chunk_path = temp_dir.join(format!("chunk-{chunk_hash}.cfir"));
    std::fs::remove_file(&chunk_path).expect("remove chunk");

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(result.error_count, 1);
    assert_eq!(
        result.diagnostics.diagnostics[0].code,
        E_LOADER_INDEX_INVALID.to_string()
    );
    assert!(result.diagnostics.diagnostics[0]
        .message
        .contains("Missing IR chunk"));

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn open_model_rejects_invalid_manifest_json() {
    let (temp_dir, _index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);
    std::fs::write(&manifest_path, b"{ this is not valid json ]").expect("write corrupt manifest");

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(result.error_count, 1);
    assert_eq!(
        result.diagnostics.diagnostics[0].code,
        E_LOADER_MANIFEST_INVALID.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn open_model_rejection_is_deterministic_for_same_corruption() {
    let (temp_dir, _index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);

    let mut manifest_json = load_json(&manifest_path);
    manifest_json["schema_version"] = JsonValue::Number(999_u64.into());
    write_json(&manifest_path, &manifest_json);

    let request = OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    };
    let first = open_model(request.clone());
    let second = open_model(request);

    assert_eq!(first.status, OperationStatus::Error);
    assert_eq!(first, second);
    assert_eq!(
        first.diagnostics.diagnostics[0].code,
        E_LOADER_MANIFEST_INCONSISTENT.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn initialize_selection_state_returns_canonical_empty_state() {
    let (temp_dir, _index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);
    let open_result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });
    let handle = open_result.model_handle.expect("model_handle");
    let context_tags = BTreeMap::from([("region".to_string(), "us".to_string())]);

    let result = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "component:thermal_control".to_string(),
        context_tags: context_tags.clone(),
    });

    assert_eq!(result.status, OperationStatus::Ok);
    let selection_state = result.selection_state.expect("selection_state");
    assert_eq!(
        selection_state,
        canonical_selection_state(
            handle.model_hash,
            "component:thermal_control".to_string(),
            context_tags,
            BTreeMap::new(),
        )
        .expect("canonical selection state")
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn initialize_selection_state_rejects_invalid_model_handle_artifacts() {
    let (temp_dir, index) = emitted_cmp_dir();
    let manifest_path = cmp_manifest_path(&temp_dir);
    let open_result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });
    let handle = open_result.model_handle.expect("model_handle");

    let chunk_hash = index.chunks.first().expect("chunk").chunk_hash.clone();
    let chunk_path = temp_dir.join(format!("chunk-{chunk_hash}.cfir"));
    std::fs::remove_file(&chunk_path).expect("remove chunk");

    let result = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle,
        scope: "component:thermal_control".to_string(),
        context_tags: BTreeMap::new(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(result.error_count, 1);
    assert_eq!(
        result.diagnostics.diagnostics[0].code,
        E_LOADER_INDEX_INVALID.to_string()
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

// --- configflux-47ni: UnsatCore type + unsat_core field on RejectionReason ---
// These are pure serde/contract tests for the ADR-0031 D3 unsat-core schema.
// They exercise the additive `RejectionReason.unsat_core` field shape only; the
// solver-decided content (the labeled MUS) is produced under separate issues.

fn empty_diagnostics() -> DiagnosticsReport {
    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    }
}

/// A fully populated `ExplainRejectionResult` carrying a solver-decided unsat
/// core with one `selection` constraint and one `model_rule` constraint —
/// enough to exercise every field and both `ConstraintKind` variants.
fn explain_result_with_core() -> ExplainRejectionResult {
    ExplainRejectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: "model-hash-abc".to_string(),
        scope: "component:thermal_control".to_string(),
        facet: "database".to_string(),
        option: "postgres".to_string(),
        rejection: RejectionReason {
            code: E_SELECTION_CONFLICT.to_string(),
            message: "Selection 'database'='postgres' is unsatisfiable".to_string(),
            blocking_choices: BTreeMap::from([("storage".to_string(), "local".to_string())]),
            hint: Some("Choose a compatible option".to_string()),
            unsat_core: Some(UnsatCore {
                rejected: ConstraintFacet {
                    facet: "database".to_string(),
                    option: "postgres".to_string(),
                },
                conflicting_constraints: vec![
                    ConflictingConstraint {
                        kind: ConstraintKind::Selection,
                        facets: vec![ConstraintFacet {
                            facet: "storage".to_string(),
                            option: "local".to_string(),
                        }],
                        summary: "blocked by your earlier choice: storage.local".to_string(),
                    },
                    ConflictingConstraint {
                        kind: ConstraintKind::ModelRule,
                        facets: vec![
                            ConstraintFacet {
                                facet: "database".to_string(),
                                option: "postgres".to_string(),
                            },
                            ConstraintFacet {
                                facet: "storage".to_string(),
                                option: "remote".to_string(),
                            },
                        ],
                        summary: "database.postgres requires storage.remote".to_string(),
                    },
                ],
                minimal: true,
                note: "one minimal explanation; other minimal cores may exist".to_string(),
            }),
        },
        error_count: 0,
        warning_count: 0,
        diagnostics_ref: None,
        diagnostics: empty_diagnostics(),
    }
}

#[test]
fn unsat_core_serializes_to_adr0031_d3_schema() {
    let result = explain_result_with_core();
    let json: JsonValue = serde_json::to_value(&result).expect("serialize result");

    let core = &json["rejection"]["unsat_core"];
    assert!(core.is_object(), "unsat_core must be present on a conflict");

    // `rejected` — the (facet, option) being explained, echoed for self-containment.
    assert_eq!(core["rejected"]["facet"], JsonValue::from("database"));
    assert_eq!(core["rejected"]["option"], JsonValue::from("postgres"));

    // `conflicting_constraints` — the labeled MUS. `kind` is snake_case per D3.
    let constraints = core["conflicting_constraints"]
        .as_array()
        .expect("conflicting_constraints array");
    assert_eq!(constraints.len(), 2);

    assert_eq!(constraints[0]["kind"], JsonValue::from("selection"));
    assert_eq!(
        constraints[0]["facets"][0]["facet"],
        JsonValue::from("storage")
    );
    assert_eq!(
        constraints[0]["facets"][0]["option"],
        JsonValue::from("local")
    );
    assert!(constraints[0]["summary"].is_string());

    assert_eq!(constraints[1]["kind"], JsonValue::from("model_rule"));
    assert_eq!(constraints[1]["facets"].as_array().expect("facets").len(), 2);

    // `minimal` is a bool; `note` is the fixed advisory string.
    assert_eq!(core["minimal"], JsonValue::Bool(true));
    assert!(core["note"].is_string());

    // The labeled-MUS invariant (ADR-0031 D3): no raw BDD variable indices.
    // Every facet entry under conflicting_constraints is a labeled string pair.
    for c in constraints {
        for f in c["facets"].as_array().expect("facets") {
            assert!(f["facet"].is_string());
            assert!(f["option"].is_string());
        }
    }
}

#[test]
fn explain_result_with_core_round_trips_identically() {
    let original = explain_result_with_core();
    let text = serde_json::to_string(&original).expect("serialize");
    let parsed: ExplainRejectionResult = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(original, parsed, "round-trip must yield an identical struct");
}

#[test]
fn unsat_core_none_omits_the_key() {
    let mut result = explain_result_with_core();
    result.rejection.unsat_core = None;

    let json: JsonValue = serde_json::to_value(&result).expect("serialize result");
    let rejection = json["rejection"].as_object().expect("rejection object");
    assert!(
        !rejection.contains_key("unsat_core"),
        "unsat_core must be omitted when None (skip_serializing_if)"
    );

    // The result still round-trips with the field absent.
    let text = serde_json::to_string(&result).expect("serialize");
    let parsed: ExplainRejectionResult = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(result, parsed);
    assert_eq!(parsed.rejection.unsat_core, None);
}

#[test]
fn rejection_reason_unchanged_fields_survive_the_additive_field() {
    // The pre-existing RejectionReason fields keep their names, shapes, and
    // serialization. blocking_choices is still a map; hint is still optional.
    let result = explain_result_with_core();
    let json: JsonValue = serde_json::to_value(&result).expect("serialize");
    let rejection = &json["rejection"];

    assert_eq!(rejection["code"], JsonValue::from(E_SELECTION_CONFLICT));
    assert!(rejection["message"].is_string());
    assert_eq!(
        rejection["blocking_choices"]["storage"],
        JsonValue::from("local")
    );
    assert!(rejection["hint"].is_string());

    // A RejectionReason absent `unsat_core` and `hint` deserializes via defaults
    // (the existing `explain_rejection_failed` path passes such reasons through).
    let minimal_json = r#"{"code":"E_SELECTION_UNKNOWN_FACET","message":"x"}"#;
    let reason: RejectionReason = serde_json::from_str(minimal_json).expect("deserialize");
    assert_eq!(reason.blocking_choices, BTreeMap::new());
    assert_eq!(reason.hint, None);
    assert_eq!(reason.unsat_core, None);
}
