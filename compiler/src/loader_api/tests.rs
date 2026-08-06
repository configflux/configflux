// SPDX-License-Identifier: BUSL-1.1

use super::*;
use crate::scenario_test_support::unique_temp_path;
use crate::Compiler;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

const S1_SOURCE_DEFS: &str = "scenarios/s1_water_pump/smoke/chunks/00_definitions.toml";
const S1_SOURCE_COMPONENTS: &str = "scenarios/s1_water_pump/smoke/chunks/10_components.toml";
const S1_CHUNK_DEFS: &str =
    include_str!("../../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const S1_CHUNK_COMPONENTS: &str =
    include_str!("../../scenarios/s1_water_pump/smoke/cue/10_components.json");

fn emitted_cmp_dir() -> (PathBuf, ir::IrIndex) {
    let temp_dir = unique_temp_path("cfx-loader-open-model", "fixture");

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
                        constraint_id: None,
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
                        constraint_id: None,
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

#[test]
fn unsupported_schema_version_hint_names_the_required_version() {
    // configflux-8u92: the rejection hint must name the version the binary
    // requires (PRODUCT_SCHEMA_VERSION), never a stale literal echoing the
    // value just rejected. The schema gate fires before any manifest I/O, so
    // the request ref is never read.
    let result = open_model(OpenModelRequest {
        schema_version: 1,
        cmp_manifest_ref: "unused-schema-gate-fires-first".to_string(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    let diagnostic = result
        .diagnostics
        .diagnostics
        .iter()
        .find(|d| d.code == E_LOADER_UNSUPPORTED_SCHEMA_VERSION)
        .expect("schema version rejection diagnostic");
    let hint = diagnostic.hint.as_deref().expect("rejection carries a hint");
    assert!(
        hint.contains(&PRODUCT_SCHEMA_VERSION.to_string()),
        "hint must name required schema_version {PRODUCT_SCHEMA_VERSION}, got: {hint}"
    );
    assert!(
        !hint.contains("to 1"),
        "hint must not echo the rejected value: {hint}"
    );
}

// ---------------------------------------------------------------------------
// ADR-0054 §4 — `constraints` ingestion into the selection model.
// ---------------------------------------------------------------------------

const CONSTRAINT_DEFS_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },
  "constraints": {
    "zeta_forbids_debug": {
      "condition": "environment != 'prod' || log_level != 'debug'",
      "doc": "Debug logging is not permitted in production."
    }
  }
}"#;

const CONSTRAINT_COMPONENTS_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "components": {
    "webapp": {
      "type": "service",
      "condition": "environment == 'dev'",
      "params": {}
    }
  },
  "constraints": {
    "alpha_requires_info": {"condition": "log_level == 'info' || environment != 'prod'"}
  }
}"#;

fn model_handle_for(dir: &Path, index: &ir::IrIndex) -> ModelHandle {
    ModelHandle {
        model_hash: index.config_hash.clone(),
        cmp_manifest_ref: cmp_manifest_path(dir).to_string_lossy().into_owned(),
        index_ref: dir
            .join(ir::CMP_DEFAULT_INDEX_REF)
            .to_string_lossy()
            .into_owned(),
        chunk_set_ref: dir.to_string_lossy().into_owned(),
        ccm_ref: String::new(),
    }
}

/// Compile ONE authored chunk into a throwaway CMP and hand back its handle.
/// Used by the ADR-0054 §2 cases that need a purpose-built facet/default shape
/// the shared two-chunk fixture cannot express.
fn single_chunk_model(label: &str, chunk: &str) -> (PathBuf, ModelHandle) {
    let temp_dir = unique_temp_path("cfx-loader", label);
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", chunk)
        .expect("add chunk");
    let index = compiler.emit_ir(&temp_dir).expect("emit ir");

    let handle = model_handle_for(&temp_dir, &index);
    (temp_dir, handle)
}

fn constraint_model_handle() -> (PathBuf, ModelHandle) {
    let temp_dir = unique_temp_path("cfx-loader-constraints", "fixture");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let mut compiler = Compiler::new();
    compiler
        .add_chunk_auto("00_definitions.json", CONSTRAINT_DEFS_CHUNK)
        .expect("add defs chunk");
    compiler
        .add_chunk_auto("10_components.json", CONSTRAINT_COMPONENTS_CHUNK)
        .expect("add components chunk");
    let index = compiler.emit_ir(&temp_dir).expect("emit ir");

    let handle = model_handle_for(&temp_dir, &index);
    (temp_dir, handle)
}

#[test]
fn selection_model_carries_constraints_in_id_ascending_order() {
    let (temp_dir, handle) = constraint_model_handle();
    let model = load_selection_constraint_model(&handle).expect("load selection model");

    let ids: Vec<&str> = model
        .constraints
        .iter()
        .map(|constraint| constraint.id.as_str())
        .collect();
    // Ascending ACROSS chunks: `alpha_requires_info` is authored in the second
    // chunk and `zeta_forbids_debug` in the first, so index order alone would
    // give the wrong answer.
    assert_eq!(ids, vec!["alpha_requires_info", "zeta_forbids_debug"]);

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn constraints_are_kept_separate_from_selector_conditions() {
    // ADR-0054 §3: the two lists have different semantics and must never be
    // merged again. A constraint must NOT appear in `conditions`, and the
    // component's selector must NOT appear in `constraints`.
    let (temp_dir, handle) = constraint_model_handle();
    let model = load_selection_constraint_model(&handle).expect("load selection model");

    let selector = parse_condition_expr("environment == 'dev'").expect("selector parses");
    assert!(
        model.conditions.contains(&selector),
        "the component selector must still reach `conditions`"
    );
    assert!(
        !model
            .constraints
            .iter()
            .any(|constraint| constraint.expr == selector),
        "a selector must never be recorded as a policy constraint"
    );

    for constraint in &model.constraints {
        assert!(
            !model.conditions.contains(&constraint.expr),
            "constraint '{}' leaked into the selector `conditions` list",
            constraint.id
        );
    }

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_constraint_does_not_widen_a_facet_domain() {
    // A policy asserts over a domain; it never creates or extends one. Both
    // facets here are fully declared, so their domains must be exactly the
    // declared values even though the constraints name those same values.
    let (temp_dir, handle) = constraint_model_handle();
    let model = load_selection_constraint_model(&handle).expect("load selection model");

    let environment = model
        .facet_domains
        .get("environment")
        .expect("declared facet seeded");
    assert_eq!(
        environment.iter().cloned().collect::<Vec<_>>(),
        vec!["dev".to_string(), "prod".to_string()]
    );
    let log_level = model
        .facet_domains
        .get("log_level")
        .expect("declared facet seeded");
    assert_eq!(
        log_level.iter().cloned().collect::<Vec<_>>(),
        vec!["debug".to_string(), "info".to_string()]
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn the_resolve_model_carries_constraints_verbatim() {
    // ADR-0054 §1: constraints pass through the resolve layer verbatim, like
    // facets. Nothing in THIS task reads them from the resolve-time `Config` —
    // enforcement is ADR-0054 §6 (configflux-4sjk) — so without this test the
    // carry is unpinned plumbing that a later refactor could quietly drop,
    // leaving 4sjk to discover the policy was never there.
    let (temp_dir, handle) = constraint_model_handle();
    let config = load_resolve_model(&handle).expect("load resolve model").config;

    let mut ids: Vec<&str> = config.constraints.keys().map(String::as_str).collect();
    ids.sort();
    assert_eq!(ids, vec!["alpha_requires_info", "zeta_forbids_debug"]);
    assert_eq!(
        config.constraints["zeta_forbids_debug"].condition,
        "environment != 'prod' || log_level != 'debug'",
        "the condition text must survive verbatim — no rewriting at the resolve layer"
    );
    assert_eq!(
        config.constraints["zeta_forbids_debug"].doc.as_deref(),
        Some("Debug logging is not permitted in production.")
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_model_without_constraints_carries_an_empty_constraint_list() {
    let (temp_dir, index) = emitted_cmp_dir();
    let handle = model_handle_for(&temp_dir, &index);
    let model = load_selection_constraint_model(&handle).expect("load selection model");

    assert!(model.constraints.is_empty());
    assert!(
        !model.conditions.is_empty(),
        "S1 authors selector conditions; only `constraints` should be empty"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn the_resolve_model_records_which_chunk_declared_each_constraint() {
    // ADR-0054 §6 / configflux-emmg: the rejection diagnostic's `source_id` is
    // "the chunk that declared the constraint". The two constraints in this
    // fixture are authored in DIFFERENT chunks, so a walk that merged them
    // without provenance — or one that stamped every constraint with the last
    // chunk visited — fails here.
    let (temp_dir, handle) = constraint_model_handle();
    let model = load_resolve_model(&handle).expect("load resolve model");

    assert_eq!(
        model.constraint_sources.get("zeta_forbids_debug").map(String::as_str),
        Some("00_definitions.json")
    );
    assert_eq!(
        model.constraint_sources.get("alpha_requires_info").map(String::as_str),
        Some("10_components.json")
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

// ---------------------------------------------------------------------------
// ADR-0054 §2/§6 — resolve FAILS CLOSED on a constraint-violating selection
// (configflux-4sjk). The contract: exit-class `E_SELECTION_CONFLICT`, one
// diagnostic per violated constraint in id-ascending order, no `resolve_hash`
// and no `resolved_output` (so there is nothing for a caller to export).
// ---------------------------------------------------------------------------

/// Resolve `choices` against `handle` at scope `all`, building the canonical
/// selection state the API requires.
fn resolve_with_choices(handle: &ModelHandle, choices: &[(&str, &str)]) -> ResolveResult {
    let choices: BTreeMap<String, String> = choices
        .iter()
        .map(|(facet, option)| (facet.to_string(), option.to_string()))
        .collect();
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        "all".to_string(),
        BTreeMap::new(),
        choices,
    )
    .expect("canonical selection state");

    resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state,
    })
}

#[test]
fn resolve_rejects_a_selection_that_violates_a_constraint() {
    let (temp_dir, handle) = constraint_model_handle();
    let result = resolve_with_choices(&handle, &[("environment", "prod"), ("log_level", "debug")]);

    assert_eq!(
        result.status,
        OperationStatus::Error,
        "prod + debug violates the declared policy and must not resolve"
    );
    // No partial output: nothing to hash, nothing to export.
    assert!(result.resolve_hash.is_none());
    assert!(result.resolved_output.is_none());

    let diagnostic = &result.diagnostics.diagnostics[0];
    assert_eq!(diagnostic.code, E_SELECTION_CONFLICT);
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    assert_eq!(
        diagnostic.entity_path.as_deref(),
        Some("constraints/alpha_requires_info"),
        "the `constraints/` prefix is how a machine consumer tells a policy \
         violation from the other E_SELECTION_CONFLICT causes (ADR-0054 §6)"
    );
    assert!(
        diagnostic.message.contains("alpha_requires_info")
            && diagnostic
                .message
                .contains("log_level == 'info' || environment != 'prod'"),
        "the message must name the constraint and quote its condition verbatim: {}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some("Run 'cfx explain' with the same selection to see the minimal conflicting set.")
    );
    assert_eq!(
        diagnostic.source_id.as_deref(),
        Some("10_components.json"),
        "ADR-0054 §6: source_id is the chunk that declared the constraint"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn resolve_reports_every_violated_constraint_in_id_ascending_order() {
    // Both fixture constraints forbid prod + debug, and they are authored in
    // different chunks in the opposite order — so index order alone would get
    // this wrong. ADR-0054 §6: one diagnostic per violated constraint, all
    // carrying the same code, emitted id-ascending.
    let (temp_dir, handle) = constraint_model_handle();
    let result = resolve_with_choices(&handle, &[("environment", "prod"), ("log_level", "debug")]);

    let entity_paths: Vec<&str> = result
        .diagnostics
        .diagnostics
        .iter()
        .map(|d| d.entity_path.as_deref().expect("entity_path"))
        .collect();
    assert_eq!(
        entity_paths,
        vec![
            "constraints/alpha_requires_info",
            "constraints/zeta_forbids_debug"
        ]
    );
    assert!(result
        .diagnostics
        .diagnostics
        .iter()
        .all(|d| d.code == E_SELECTION_CONFLICT));
    assert_eq!(result.diagnostics.error_count, 2);

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn resolve_accepts_a_selection_every_constraint_permits() {
    // The same model, the same facets, a legal combination: dev + debug. The
    // enforcement must reject policy violations, not selections in general.
    let (temp_dir, handle) = constraint_model_handle();
    let result = resolve_with_choices(&handle, &[("environment", "dev"), ("log_level", "debug")]);

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "dev + debug breaks no policy: {:?}",
        result.diagnostics.diagnostics
    );
    assert!(result.resolve_hash.is_some());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_constraint_violated_only_by_a_declared_default_is_still_rejected() {
    // ADR-0054 §2 evaluates constraints under the TOTAL POST-DEFAULT
    // assignment, not under the user's choices. Here the user types NOTHING:
    // both offending values arrive from ADR-0047 §5 default auto-bind. A check
    // that only looked at `choices` would resolve this happily — and would ship
    // a configuration no surface ever screened.
    const DEFAULTS_VIOLATE_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "prod"},
    "log_level": {"values": ["info", "debug"], "default": "debug"}
  },
  "constraints": {
    "prod_forbids_debug": {"condition": "environment != 'prod' || log_level != 'debug'"}
  }
}"#;
    let (temp_dir, handle) = single_chunk_model("defaults-violate", DEFAULTS_VIOLATE_CHUNK);
    let result = resolve_with_choices(&handle, &[]);

    assert_eq!(result.status, OperationStatus::Error);
    assert_eq!(
        result.diagnostics.diagnostics[0].entity_path.as_deref(),
        Some("constraints/prod_forbids_debug")
    );
    assert_eq!(
        result.defaulted_choices,
        BTreeMap::from([
            ("environment".to_string(), "prod".to_string()),
            ("log_level".to_string(), "debug".to_string()),
        ]),
        "the rejection must show where the offending values came from — the \
         user typed neither"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_constraint_over_an_unbound_facet_is_not_a_violation() {
    // ADR-0054 §2: `Ternary::Unknown` is not a violation. `region` is declared
    // with no default and nothing binds it, so the policy is not decided —
    // nothing was chosen, so nothing was violated. Rejecting here would make
    // every under-specified selection fail closed for no reason.
    const UNBOUND_FACET_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "region": {"values": ["eu", "us"]}
  },
  "constraints": {
    "eu_forbids_prod": {"condition": "region != 'eu' || environment != 'prod'"}
  }
}"#;
    let (temp_dir, handle) = single_chunk_model("unbound-facet", UNBOUND_FACET_CHUNK);
    let result = resolve_with_choices(&handle, &[("environment", "prod")]);

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "an undecided constraint must not reject: {:?}",
        result.diagnostics.diagnostics
    );

    // Binding the other half DOES decide it, and then it is a violation.
    let decided = resolve_with_choices(&handle, &[("environment", "prod"), ("region", "eu")]);
    assert_eq!(decided.status, OperationStatus::Error);
    assert_eq!(
        decided.diagnostics.diagnostics[0].entity_path.as_deref(),
        Some("constraints/eu_forbids_prod")
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_model_that_declares_no_constraint_resolves_unaffected() {
    // The enforcement is inert for every model authored before ADR-0054: with
    // an empty constraint list there is nothing to evaluate and nothing to
    // reject. S1 is the byte-stability anchor, so this also guards the goldens.
    let (temp_dir, index) = emitted_cmp_dir();
    let handle = model_handle_for(&temp_dir, &index);
    let result = resolve_with_choices(
        &handle,
        &[
            ("cooling_brand", "hydra"),
            ("cooling_model", "x200"),
            ("pump_type", "centrifugal"),
            ("region", "eu"),
        ],
    );

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "S1 declares no constraints and must resolve exactly as before: {:?}",
        result.diagnostics.diagnostics
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

// ---------------------------------------------------------------------------
// ADR-0054 §2 on the SELECTION surfaces — options and apply (configflux-p571.9,
// configflux-narb). The same one rule resolve enforces, asked one choice at a
// time: an option no constraint-satisfying assignment contains is not offered,
// and applying it is rejected with the SAME `E_SELECTION_CONFLICT` diagnostic
// resolve raises. That agreement is what stops `session_compose` from having to
// report a correctly-modelled policy rejection as engine divergence.
// ---------------------------------------------------------------------------

/// Apply `facet`=`option` on top of `prior` choices at scope `all`.
fn apply_choice(
    handle: &ModelHandle,
    prior: &[(&str, &str)],
    facet: &str,
    option: &str,
) -> ApplySelectionResult {
    let choices: BTreeMap<String, String> = prior
        .iter()
        .map(|(facet, option)| (facet.to_string(), option.to_string()))
        .collect();
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        "all".to_string(),
        BTreeMap::new(),
        choices,
    )
    .expect("canonical selection state");

    apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state,
        selection_delta: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    })
}

/// List `facet`'s still-valid options under `choices` at scope `all`.
fn options_for(
    handle: &ModelHandle,
    choices: &[(&str, &str)],
    facet: &str,
    include_pruned_reasons: bool,
) -> GetSelectionOptionsResult {
    let choices: BTreeMap<String, String> = choices
        .iter()
        .map(|(facet, option)| (facet.to_string(), option.to_string()))
        .collect();
    let selection_state = canonical_selection_state(
        handle.model_hash.clone(),
        "all".to_string(),
        BTreeMap::new(),
        choices,
    )
    .expect("canonical selection state");

    get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state,
        facet: facet.to_string(),
        include_pruned_reasons,
    })
}

#[test]
fn apply_rejects_a_choice_that_violates_a_constraint() {
    // configflux-narb, the whole point: this apply used to SUCCEED. The solver
    // rejected the same choice (the policy is a `.ccm` root conjunct), so
    // `session_compose` saw solver-REJECT vs loader-ACCEPT and reported the
    // model's own flagship policy as `E_SELECTION_ENGINE_DIVERGENCE` — an
    // internal-fault code whose hint told the user to recompile.
    let (temp_dir, handle) = constraint_model_handle();
    let result = apply_choice(&handle, &[("environment", "prod")], "log_level", "debug");

    assert_eq!(
        result.status,
        OperationStatus::Error,
        "prod + debug violates the declared policy and must not apply"
    );
    assert!(
        result.selection_state.is_none(),
        "a rejected apply must not hand back a next state"
    );

    let diagnostic = &result.diagnostics.diagnostics[0];
    assert_eq!(
        diagnostic.code, E_SELECTION_CONFLICT,
        "apply must reject with the SAME code resolve uses, not a new one and \
         not the internal-fault family"
    );
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    // Both constraints in this fixture forbid prod+debug; id-ascending picks
    // `alpha_requires_info`, which is also the constraint resolve names first.
    assert_eq!(
        diagnostic.entity_path.as_deref(),
        Some("constraints/alpha_requires_info")
    );
    assert!(
        diagnostic.message.contains("alpha_requires_info")
            && diagnostic
                .message
                .contains("log_level == 'info' || environment != 'prod'"),
        "the message must name the constraint and quote its condition: {}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.source_id.as_deref(),
        Some("10_components.json"),
        "ADR-0054 §6: source_id is the chunk that declared the constraint"
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some("Run 'cfx explain' with the same selection to see the minimal conflicting set."),
        "select and resolve must send the user to the same next command"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn apply_and_resolve_raise_the_identical_diagnostic_for_one_policy() {
    // The two surfaces are separate code paths reading separate models (the
    // selection-side `SelectionConstraintModel` vs the resolve-side
    // `ResolveModel`). One policy, one message: if these ever drift the product
    // is telling a user two different stories about one rule.
    let (temp_dir, handle) = constraint_model_handle();

    let applied = apply_choice(&handle, &[("environment", "prod")], "log_level", "debug");
    let resolved = resolve_with_choices(&handle, &[("environment", "prod"), ("log_level", "debug")]);

    let from_apply = &applied.diagnostics.diagnostics[0];
    let from_resolve = &resolved.diagnostics.diagnostics[0];
    assert_eq!(from_apply.code, from_resolve.code);
    assert_eq!(from_apply.message, from_resolve.message);
    assert_eq!(from_apply.entity_path, from_resolve.entity_path);
    assert_eq!(from_apply.source_id, from_resolve.source_id);
    assert_eq!(from_apply.hint, from_resolve.hint);

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn apply_accepts_a_choice_that_decides_no_constraint_yet() {
    // ADR-0054 §2: `Ternary::Unknown` is not a violation. Selecting prod alone
    // leaves both policies undecided — `log_level` is unbound — so the guided
    // walk must keep walking. Rejecting here would make the first half of every
    // legal two-step selection fail.
    let (temp_dir, handle) = constraint_model_handle();
    let result = apply_choice(&handle, &[], "environment", "prod");

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "an undecided policy must not reject: {:?}",
        result.diagnostics.diagnostics
    );
    assert_eq!(
        result
            .selection_state
            .expect("next state")
            .choices
            .get("environment"),
        Some(&"prod".to_string())
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn apply_accepts_a_choice_every_constraint_permits() {
    // The control. Fail-closed must mean "closed on violations", not "closed".
    let (temp_dir, handle) = constraint_model_handle();
    let result = apply_choice(&handle, &[("environment", "dev")], "log_level", "debug");

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "dev + debug breaks no policy: {:?}",
        result.diagnostics.diagnostics
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn options_prune_an_option_a_constraint_forbids_under_the_current_selection() {
    // The options half of the same rule: once prod is chosen, `debug` appears in
    // no assignment the policy admits, so it is not offered. This is the loader
    // reaching the answer the solver's per-arm SAT query already reached — the
    // two must agree, or one envelope reports an option as both valid and gone.
    let (temp_dir, handle) = constraint_model_handle();

    let unselected = options_for(&handle, &[], "log_level", false);
    assert_eq!(
        unselected.valid_options,
        vec!["debug".to_string(), "info".to_string()],
        "with nothing selected the policy is undecided and both arms stand"
    );

    let under_prod = options_for(&handle, &[("environment", "prod")], "log_level", false);
    assert_eq!(
        under_prod.valid_options,
        vec!["info".to_string()],
        "prod forbids debug, so debug is not an option under prod"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn options_explain_a_constraint_pruned_option_in_pruned_reasons() {
    // `pruned_options` is the "why is it gone" list. An option dropped by a
    // policy has to appear in it, or a caller asking for reasons is told an
    // option vanished for no reason at all.
    let (temp_dir, handle) = constraint_model_handle();
    let result = options_for(&handle, &[("environment", "prod")], "log_level", true);

    let pruned = result.pruned_options.expect("pruned reasons requested");
    assert_eq!(
        pruned.iter().map(|p| p.option.as_str()).collect::<Vec<_>>(),
        vec!["debug"],
        "the policy-pruned option must be reported, not silently absent"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_positive_equality_constraint_prunes_the_sibling_option() {
    // ADR-0054 §5.2's UNDER-pruning class, stated as a regression: a constraint
    // that positively equates a facet to a value (`log_level == 'info'`) leaves
    // every sibling forbidden. Concretely the loader sees `log_level == 'info'`
    // evaluate FALSE under `{log_level: debug}`; on the solver side the same
    // answer needs the ADR-0054 §5.2 intra-facet cardinality in the `.ccm` root
    // — without at-most-one, `log_level.info ∧ log_level.debug` is satisfiable,
    // so the existential per-arm query would keep offering `debug` while
    // resolve rejected it. This case is why that synthesis exists.
    const PINNED_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },
  "constraints": {
    "logging_is_pinned_to_info": {"condition": "log_level == 'info'"}
  }
}"#;
    let (temp_dir, handle) = single_chunk_model("positive-equality", PINNED_CHUNK);

    let listed = options_for(&handle, &[], "log_level", false);
    assert_eq!(
        listed.valid_options,
        vec!["info".to_string()],
        "a positively-equated facet offers exactly the value it is equated to"
    );

    // ... and the surfaces agree about it: apply refuses the sibling, and
    // resolve — which evaluates the constraint concretely under the total
    // post-default assignment — refuses the same selection.
    let applied = apply_choice(&handle, &[], "log_level", "debug");
    assert_eq!(applied.status, OperationStatus::Error);
    assert_eq!(
        applied.diagnostics.diagnostics[0].entity_path.as_deref(),
        Some("constraints/logging_is_pinned_to_info")
    );

    let resolved = resolve_with_choices(&handle, &[("log_level", "debug")]);
    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "options and resolve must agree: {:?}",
        resolved.diagnostics.diagnostics
    );

    // The permitted value still resolves — the policy pins a value, it does not
    // empty the domain.
    assert_eq!(
        resolve_with_choices(&handle, &[("log_level", "info")]).status,
        OperationStatus::Ok
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn declaring_the_facet_is_what_makes_the_three_surfaces_agree() {
    // configflux-6j91, the paired half of the compiler-side rejection. This is
    // the SAME model the compiler now refuses — a component condition on `arch`
    // and a constraint pinning `arch` — with the one difference that makes it
    // legal: `arch` is declared. Declaration is what earns the ADR-0054 §5.2
    // at-most-one clauses, and those clauses are what make the existential
    // per-arm query agree with the concrete evaluation resolve performs. Without
    // the declaration, `root AND arch.x86 AND arch.arm` is SAT over independent
    // variables, so options and select would keep offering `arm` while resolve
    // rejected it. That fail-open corner is why the compiler refuses the
    // undeclared form outright instead of trying to enforce it here.
    const DECLARED_ARCH_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "arch": {"values": ["x86", "arm"], "default": "x86"}
  },
  "constraints": {
    "pinned_arch": {"condition": "arch == 'x86'"}
  },
  "components": {
    "agent": {"type": "service", "condition": "arch == 'x86'", "params": {}}
  }
}"#;
    let (temp_dir, handle) = single_chunk_model("declared-arch", DECLARED_ARCH_CHUNK);

    // Surface 1 — options WITHHOLDS the value no valid configuration can hold.
    let listed = options_for(&handle, &[], "arch", false);
    assert_eq!(
        listed.status,
        OperationStatus::Ok,
        "{:?}",
        listed.diagnostics.diagnostics
    );
    assert_eq!(
        listed.valid_options,
        vec!["x86".to_string()],
        "options must withhold the sibling a positive equality forbids"
    );

    // Surface 2 — select REJECTS it, naming the policy that forbade it.
    let applied = apply_choice(&handle, &[], "arch", "arm");
    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "select must refuse what options withheld: {:?}",
        applied.diagnostics.diagnostics
    );
    assert_eq!(
        applied.diagnostics.diagnostics[0].entity_path.as_deref(),
        Some("constraints/pinned_arch")
    );

    // Surface 3 — resolve REJECTS the same selection, evaluating the constraint
    // concretely under the total post-default assignment.
    let resolved = resolve_with_choices(&handle, &[("arch", "arm")]);
    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "resolve must refuse what the other two surfaces refused: {:?}",
        resolved.diagnostics.diagnostics
    );
    assert_eq!(
        resolved.diagnostics.diagnostics[0].entity_path.as_deref(),
        Some("constraints/pinned_arch")
    );

    // ... and the permitted value is still accepted by all three, so the
    // agreement is on the policy, not on refusing everything.
    assert_eq!(
        apply_choice(&handle, &[], "arch", "x86").status,
        OperationStatus::Ok
    );
    assert_eq!(
        resolve_with_choices(&handle, &[("arch", "x86")]).status,
        OperationStatus::Ok
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_model_that_declares_no_constraint_selects_unaffected() {
    // Byte-stability anchor for the selection surfaces, matching the
    // resolve-side guard above: S1 declares no constraints, so the new screen is
    // inert and every existing options/apply byte is untouched.
    let (temp_dir, index) = emitted_cmp_dir();
    let handle = model_handle_for(&temp_dir, &index);

    let listed = options_for(&handle, &[], "cooling_brand", false);
    assert_eq!(
        listed.status,
        OperationStatus::Ok,
        "{:?}",
        listed.diagnostics.diagnostics
    );
    assert!(
        listed.valid_options.contains(&"hydra".to_string()),
        "S1's options must be unchanged: {:?}",
        listed.valid_options
    );

    let applied = apply_choice(&handle, &[], "cooling_brand", "hydra");
    assert_eq!(
        applied.status,
        OperationStatus::Ok,
        "S1's applies must be unchanged: {:?}",
        applied.diagnostics.diagnostics
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}
