// SPDX-License-Identifier: BUSL-1.1

use super::*;
// configflux-y2ai: the resolve-hash pre-image moved to the leaf
// `crate::resolve_hash` module so the loader and the runtime share one copy.
// `resolve_hash_selection_fields_const_matches_preimage` below still asserts
// the const against the struct itself, now at its new home.
use crate::resolve_hash::ResolveHashCanonical;
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
        implied_choices: Default::default(),
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

// ---------------------------------------------------------------------------
// configflux-secb.2 / ADR-0057 §D5 — resolve-side evaluation of a
// facet-to-facet equality constraint.
//
// Two facets stay independently bindable; one constraint ties them together
// for this deployment. Three cases decide the contract: bound differently is a
// violation, bound equal resolves, and ONE side bound is Unknown — which is
// not a violation (ADR-0054 §2), because a selection that has not yet said
// what the other facet is has not broken an agreement rule about both.
// ---------------------------------------------------------------------------

const FACET_EQUALITY_CHUNK: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "line_container": {"values": ["c1", "c2"], "default": "c1"},
    "sorter_container": {"values": ["c1", "c2"], "default": "c1"}
  },
  "constraints": {
    "groups_equal": {
      "condition": "sorter_container == line_container",
      "doc": "The sorter and the line must draw from the same container."
    }
  },
  "components": {
    "sorter": {"type": "service", "params": {}}
  }
}"#;

#[test]
fn resolve_rejects_a_selection_that_binds_two_equated_facets_differently() {
    let (temp_dir, handle) = single_chunk_model("facet-equality-conflict", FACET_EQUALITY_CHUNK);
    let result = resolve_with_choices(
        &handle,
        &[("line_container", "c1"), ("sorter_container", "c2")],
    );

    assert_eq!(
        result.status,
        OperationStatus::Error,
        "c1 and c2 disagree, so the equality constraint must fail closed"
    );
    assert!(result.resolve_hash.is_none());
    assert!(result.resolved_output.is_none());

    let diagnostic = &result.diagnostics.diagnostics[0];
    assert_eq!(diagnostic.code, E_SELECTION_CONFLICT);
    assert_eq!(
        diagnostic.entity_path.as_deref(),
        Some("constraints/groups_equal")
    );
    assert!(
        diagnostic.message.contains("groups_equal")
            && diagnostic
                .message
                .contains("sorter_container == line_container"),
        "the message must name the constraint and quote the AUTHORED condition, \
         unquoted right-hand side and all: {}",
        diagnostic.message
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn resolve_accepts_a_selection_that_binds_two_equated_facets_alike() {
    let (temp_dir, handle) = single_chunk_model("facet-equality-agree", FACET_EQUALITY_CHUNK);
    let result = resolve_with_choices(
        &handle,
        &[("line_container", "c2"), ("sorter_container", "c2")],
    );

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "both bound to c2 satisfies the equality: {:?}",
        result.diagnostics.diagnostics
    );
    assert!(result.resolve_hash.is_some());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn one_unbound_side_of_an_equality_is_unknown_not_a_violation() {
    // ADR-0054 §2. Only `line_container` is chosen; the other side is left to
    // its declared default, and nothing about the pair is yet contradicted.
    let (temp_dir, handle) = single_chunk_model("facet-equality-partial", FACET_EQUALITY_CHUNK);
    let result = resolve_with_choices(&handle, &[("line_container", "c1")]);

    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "a half-bound equality must not be reported as a violation: {:?}",
        result.diagnostics.diagnostics
    );
    assert!(
        !result
            .diagnostics
            .diagnostics
            .iter()
            .any(|d| d.code == E_SELECTION_CONFLICT),
        "no conflict may be raised: {:?}",
        result.diagnostics.diagnostics
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
    //
    // Every choice is one S1's own `apply_selection` accepts as a delta —
    // `pump_type=dual` is the value its condition names and its sbom golden
    // carries. `resolve_with_choices` seals the state directly rather than
    // building it through `apply_selection`, so a value no surface would accept
    // could sit here unnoticed; since ADR-0030 Amendment 2 the state's
    // assignments are screened against the model on the way in, and one would
    // be refused as `E_SELECTION_INVALID_OPTION` — a true verdict, but not the
    // one this case is about.
    let (temp_dir, index) = emitted_cmp_dir();
    let handle = model_handle_for(&temp_dir, &index);
    let result = resolve_with_choices(
        &handle,
        &[
            ("cooling_brand", "hydra"),
            ("cooling_model", "x200"),
            ("pump_type", "dual"),
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

/// Every non-empty `{string: string}` object in a serialized pre-image, by field
/// name — the shape a selection-provenance map takes on the wire. Recurses, so a
/// provenance field nested inside `selection_state` counts the same as a
/// top-level one. `resolved_output` is skipped BY NAME: it is the resolved
/// payload, and a model whose output happened to be flat strings is not
/// provenance about the selection.
fn collect_string_map_fields(value: &JsonValue, out: &mut BTreeSet<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, child) in object {
        if key == "resolved_output" {
            continue;
        }
        let Some(entries) = child.as_object() else {
            continue;
        };
        if !entries.is_empty() && entries.values().all(JsonValue::is_string) {
            out.insert(key.clone());
        } else {
            collect_string_map_fields(child, out);
        }
    }
}

/// `RESOLVE_HASH_SELECTION_FIELDS` names what the resolve-hash pre-image folds
/// in — and nothing but a test can hold it to that, because it is a hand-written
/// literal sitting next to the struct rather than derived from it.
///
/// The const is the SINGLE source of the `E_RUNTIME_HASH_MISMATCH` remediation
/// hint, so a field added to `ResolveHashCanonical` (or to the nested
/// `SelectionStateCanonical`) and not added here would put the hint back exactly
/// where configflux-j2jj found it: naming a narrower set than the hash covers,
/// pointing users away from the one field they actually dropped. That is the
/// drift this test exists to fail on.
///
/// It serializes a pre-image whose every provenance map is NON-EMPTY, which is
/// the only way the skip-if-empty rule lets them all appear at once, then
/// requires the map fields present to be exactly the const's set — and to appear
/// in the const's order, which is the other half of its claim.
#[test]
fn resolve_hash_selection_fields_const_matches_preimage() {
    let context_tags = BTreeMap::from([("region".to_string(), "eu".to_string())]);
    let choices = BTreeMap::from([("pump_type".to_string(), "dual".to_string())]);
    let defaulted = BTreeMap::from([("cooling_brand".to_string(), "hydra".to_string())]);
    let implied = BTreeMap::from([("cooling_model".to_string(), "x200".to_string())]);
    // Nested and not all-strings, so the payload could not be mistaken for a
    // provenance map even without the by-name exclusion above.
    let resolved_output = serde_json::json!({"components": {"thermal": {"setpoint": 21}}});

    let canonical = ResolveHashCanonical {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash: "model",
        scope: "all",
        selection_state: SelectionStateCanonical {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_hash: "model",
            scope: "all",
            context_tags: &context_tags,
            choices: &choices,
        },
        resolved_output: &resolved_output,
        defaulted_choices: &defaulted,
        implied_choices: &implied,
    };
    // The SAME serializer `compute_resolve_hash` hashes, so what is asserted
    // below is the pre-image itself and not a restatement of it.
    let preimage = serde_json::to_string(&canonical).expect("canonicalize pre-image");
    let parsed: JsonValue = serde_json::from_str(&preimage).expect("reparse pre-image");

    let mut present: BTreeSet<String> = BTreeSet::new();
    collect_string_map_fields(&parsed, &mut present);
    let named: BTreeSet<String> = RESOLVE_HASH_SELECTION_FIELDS
        .split(", ")
        .map(str::to_string)
        .collect();
    assert_eq!(
        present, named,
        "RESOLVE_HASH_SELECTION_FIELDS names {named:?} but the resolve-hash pre-image folds \
         in {present:?}. Update the const: E_RUNTIME_HASH_MISMATCH's hint is built from it, \
         so a stale list sends users to the wrong field."
    );

    // ...in pre-image order. Each field must appear AFTER the previous one; the
    // leading quote in the needle keeps `choices` from matching inside
    // `defaulted_choices`.
    let mut cursor = 0usize;
    for field in RESOLVE_HASH_SELECTION_FIELDS.split(", ") {
        let needle = format!("\"{field}\":");
        let at = preimage[cursor..]
            .find(&needle)
            .map(|offset| cursor + offset)
            .unwrap_or_else(|| {
                panic!("pre-image carries no `{field}` after byte {cursor}: {preimage}")
            });
        cursor = at + 1;
    }
}

// ---------------------------------------------------------------------------
// configflux-eclx (security screen): the Rule 1 screen on the resolve path must
// not fail OPEN when the model it reads will not load.
// ---------------------------------------------------------------------------

/// Rewrite every constraint-bearing chunk of `dir` so its declared constraints
/// carry an expression the condition parser refuses, and RE-ADDRESS the package
/// around the edit: each edited chunk is renamed to the hash of its new content
/// and the index follows it. Returns the handle for the rewritten package.
///
/// The re-addressing is what makes this a fixture rather than a corruption.
/// `verify_index_integrity` recomputes each chunk's content address from the
/// chunk's own entity maps (ADR-0056 Amendment 1), so an edit that left the
/// name and the `chunk_hash` field standing would be refused on load by BOTH
/// loaders and could no longer separate them. What separates them is the
/// constraint expression itself: `load_selection_constraint_model` parses every
/// constraint on the way in and refuses, while `load_resolve_model` carries
/// them verbatim and does not — a divergence that survives only over a package
/// whose integrity is beyond question.
fn break_one_constraint_expression(dir: &Path, index: &ir::IrIndex) -> ModelHandle {
    let mut edited = 0usize;
    let mut readdressed: Vec<(String, String)> = Vec::new();
    for chunk_ref in &index.chunks {
        let path = dir.join(format!("chunk-{}.cfir", chunk_ref.chunk_hash));
        let mut chunk = load_json(&path);
        let Some(constraints) = chunk["constraints"].as_object_mut() else {
            continue;
        };
        if constraints.is_empty() {
            continue;
        }
        for (_, constraint) in constraints.iter_mut() {
            constraint["condition"] = JsonValue::String("(".to_string());
            edited += 1;
        }
        write_json(&path, &chunk);

        let address = ir::chunk_hash_of_chunk(&ir::load_chunk(&path).expect("parse edited chunk"))
            .expect("recompute the edited chunk's address");
        chunk["chunk_hash"] = JsonValue::String(address.clone());
        let new_path = dir.join(format!("chunk-{address}.cfir"));
        write_json(&new_path, &chunk);
        if new_path != path {
            std::fs::remove_file(&path).expect("remove the pre-edit chunk file");
        }
        readdressed.push((chunk_ref.chunk_hash.clone(), address));
    }
    assert!(edited > 0, "the fixture must declare a constraint to break");

    let index_path = dir.join(ir::CMP_DEFAULT_INDEX_REF);
    let mut index_json = load_json(&index_path);
    for (before, after) in &readdressed {
        let entries = index_json["chunks"]
            .as_array_mut()
            .expect("index chunks array");
        for entry in entries.iter_mut() {
            if entry["chunk_hash"].as_str() == Some(before.as_str()) {
                entry["chunk_hash"] = JsonValue::String(after.clone());
            }
        }
        for namespace in [
            "definition_index",
            "component_index",
            "artifact_index",
            "facet_index",
            "catalogue_index",
            "binding_index",
        ] {
            let Some(map) = index_json[namespace].as_object_mut() else {
                continue;
            };
            for (_, chunk_hash) in map.iter_mut() {
                if chunk_hash.as_str() == Some(before.as_str()) {
                    *chunk_hash = JsonValue::String(after.clone());
                }
            }
        }
    }
    // An emitted index orders its chunk vector by `chunk_hash` (ADR-0056 §2),
    // and re-addressing can move a chunk within that order.
    index_json["chunks"]
        .as_array_mut()
        .expect("index chunks array")
        .sort_by(|left, right| {
            left["chunk_hash"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["chunk_hash"].as_str().unwrap_or_default())
        });
    write_json(&index_path, &index_json);

    // `config_hash` is a function of the index content, and the addresses in it
    // just moved.
    let rewritten = ir::load_index(&index_path).expect("reload rewritten index");
    index_json["config_hash"] =
        JsonValue::String(rewritten.compute_config_hash().expect("recompute config_hash"));
    write_json(&index_path, &index_json);

    model_handle_for(dir, &ir::load_index(&index_path).expect("reload rewritten index"))
}

/// A `SelectionState` naming a facet the model does not have — the input the
/// screen exists to refuse, used here only to prove the screen ran at all.
fn inadmissible_state(handle: &ModelHandle) -> SelectionState {
    canonical_selection_state(
        handle.model_hash.clone(),
        "all".to_string(),
        BTreeMap::new(),
        BTreeMap::from([("nosuch".to_string(), "x".to_string())]),
    )
    .expect("canonical selection state")
}

#[test]
fn the_state_screen_refuses_a_package_only_its_own_loader_rejects() {
    let (temp_dir, handle) = constraint_model_handle();
    let index = ir::load_index(&handle.index_ref).expect("load index");
    let handle = break_one_constraint_expression(&temp_dir, &index);

    // The divergence this case is about, stated as a precondition: one loader
    // refuses the package and the other accepts it.
    assert!(
        load_selection_constraint_model(&handle).is_err(),
        "the selection loader must refuse an unparseable constraint expression"
    );
    assert!(
        load_resolve_model(&handle).is_ok(),
        "the resolve loader must still accept it — that is what makes the \
         screen's silence a fail-open rather than a duplicate report"
    );

    let screened = screen_selection_state(&handle, &inadmissible_state(&handle));

    assert_eq!(
        screened.len(),
        1,
        "a model the screen cannot read must yield a refusal, not silence: {screened:?}"
    );
    assert_eq!(screened[0].code, E_RESOLVE_MODEL_INVALID);
    assert_eq!(screened[0].severity, DiagnosticSeverity::Error);
    assert_eq!(
        screened[0].source_id.as_deref(),
        Some(handle.index_ref.as_str()),
        "the refusal must name the package it could not read"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn the_state_screen_leaves_a_package_its_caller_also_rejects_to_the_caller() {
    let (temp_dir, mut handle) = constraint_model_handle();
    handle.index_ref = temp_dir
        .join("no-such-index.json")
        .to_string_lossy()
        .into_owned();

    // Both loaders refuse this one, so the screen stays silent and `resolve`
    // renders its own canonical `E_RESOLVE_MODEL_INVALID` — byte-for-byte what
    // it rendered before the screen existed.
    assert!(load_selection_constraint_model(&handle).is_err());
    assert!(load_resolve_model(&handle).is_err());

    assert!(
        screen_selection_state(&handle, &inadmissible_state(&handle)).is_empty(),
        "pre-empting the caller's own load failure would replace a specific \
         report of the corruption with a second one"
    );

    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: inadmissible_state(&handle),
        implied_choices: Default::default(),
    });
    assert_eq!(resolved.status, OperationStatus::Error);
    assert_eq!(resolved.diagnostics.diagnostics.len(), 1);
    assert_eq!(
        resolved.diagnostics.diagnostics[0].code,
        E_RESOLVE_MODEL_INVALID
    );
    assert!(
        resolved.diagnostics.diagnostics[0]
            .message
            .contains("Failed to load index"),
        "the caller's own load failure must still be the one reported: {:?}",
        resolved.diagnostics.diagnostics[0]
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

// ---------------------------------------------------------------------------
// ADR-0063 Amendment 1 / configflux-h3rm: the authored-symbol rule on the
// CONSUMPTION path.
//
// Every package below is written by the crate's own writers over a `Config`
// that never went through `Compiler::add_chunk_auto`, so `link_verify` never
// saw it — which is the threat exactly: package hashes are self-consistent and
// unkeyed, and ADR-0063 kept `PRODUCT_SCHEMA_VERSION` at 5, so a package built
// by a pre-amendment compiler (or rewritten with recomputed hashes) passes
// every integrity check `open_model` makes. What the crafted symbols reach if
// nothing re-checks them is `lowering::lowered_root_conjuncts`, whose text
// `facet_eq_predicate` interpolates BARE and this loader re-parses as a root
// conjunct — the configflux-mrm6 class, on the consumption side.
// ---------------------------------------------------------------------------

/// The clean model every crafted fixture below is one edited field away from.
///
/// It carries all four symbol classes — declared facet keys with declared
/// values, a catalogue with entry ids, and a binding whose `derive` table is
/// what the lowering turns into root conjuncts — so each case can edit exactly
/// one and leave the rest a working control.
const H3RM_CLEAN_SOURCE: &str = r#"{
    "package": "h3rm_loader_symbols",
    "version": "1.0.0",
    "facets": {
        "environment": { "values": ["dev", "prod"], "default": "dev" },
        "replica_class": { "values": ["single", "pair"], "default": "single" }
    },
    "catalogues": {
        "containers": {
            "fields": { "length_mm": { "type": "integer" } },
            "entries": { "c1": { "length_mm": 1200 }, "c2": { "length_mm": 2400 } }
        }
    },
    "bindings": {
        "line_container": {
            "catalogue": "containers",
            "derive": { "environment": { "dev": "c1", "prod": "c2" } }
        }
    },
    "components": {
        "svc": { "type": "service", "condition": "environment == 'prod'" }
    }
}"#;

/// Write a one-chunk package around `config`, BYPASSING ingest validation.
///
/// `Compiler::add_chunk_auto` runs `link_verify`, so a crafted symbol cannot
/// reach a chunk file through it — and a fixture that hand-edited an emitted
/// chunk would have to re-address the package around the edit anyway
/// (`break_one_constraint_expression` above is what that costs). Going through
/// the real writers instead — `ir::IrChunk::from_config`, the `LinkChunk` shape
/// `link_emit::link_chunks_of` builds, `link_emit::build_package_index` and
/// `link_emit::write_package` — produces a package whose every hash is correct
/// by construction, which is precisely the package this rule exists to refuse.
fn h3rm_package(label: &str, config: &crate::schema::Config) -> (PathBuf, ModelHandle) {
    let temp_dir = unique_temp_path("cfx-loader-symbol", label);
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let source_id = "00_model.json";
    let chunk_hash = ir::chunk_hash_from_config(config).expect("chunk content address");
    let ir_chunk = ir::IrChunk::from_config(source_id, &chunk_hash, config);
    let chunks = vec![crate::link_emit::LinkChunk {
        chunk_hash,
        source_id: source_id.to_string(),
        exports: crate::interface_summary::summarize(config, source_id).exports,
        bytes: ir::chunk_file_bytes(&ir_chunk).expect("canonical chunk bytes"),
    }];
    let index = crate::link_emit::build_package_index(&chunks).expect("build package index");
    // `write_package` runs `ir::verify_index_integrity` itself, so reaching the
    // end of this helper already proves the fixture is internally consistent.
    crate::link_emit::write_package(&temp_dir, &chunks, &index).expect("write package");

    let handle = model_handle_for(&temp_dir, &index);
    (temp_dir, handle)
}

fn h3rm_clean_config() -> crate::schema::Config {
    serde_json::from_str(H3RM_CLEAN_SOURCE).expect("parse the clean source")
}

fn h3rm_empty_state(handle: &ModelHandle) -> SelectionState {
    canonical_selection_state(
        handle.model_hash.clone(),
        "all".to_string(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("canonical selection state")
}

/// The three public operations R1 names, driven over `handle`, each asserted to
/// refuse under the code it already reports for a package it cannot read.
///
/// One helper rather than three tests because the claim is that all three AGREE
/// — a per-operation assertion can pass while the other two answer from a model
/// the loader should never have built.
fn h3rm_assert_every_operation_refuses(handle: &ModelHandle, symbol: &str) {
    // `open_model` is integrity-only and MUST still accept: the fixture's whole
    // point is that nothing about its bytes is wrong.
    let opened = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: handle.cmp_manifest_ref.clone(),
    });
    assert_eq!(
        opened.status,
        OperationStatus::Ok,
        "the package must pass every integrity check: {:?}",
        opened.diagnostics.diagnostics
    );

    let initialized = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        context_tags: BTreeMap::new(),
    });
    assert_eq!(initialized.status, OperationStatus::Error);
    let from_initialize = initialized.diagnostics.diagnostics[0].clone();
    assert_eq!(from_initialize.code, E_LOADER_INDEX_INVALID);

    let options = get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        facet: "environment".to_string(),
        selection_state: h3rm_empty_state(handle),
        include_pruned_reasons: false,
    });
    assert_eq!(options.status, OperationStatus::Error);
    assert!(
        options.valid_options.is_empty(),
        "a refused package must publish no options: {:?}",
        options.valid_options
    );
    let from_options = options.diagnostics.diagnostics[0].clone();
    assert_eq!(from_options.code, E_LOADER_INDEX_INVALID);

    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: h3rm_empty_state(handle),
        implied_choices: Default::default(),
    });
    assert_eq!(resolved.status, OperationStatus::Error);
    let from_resolve = resolved.diagnostics.diagnostics[0].clone();
    // R1: the resolve funnel reports through `E_RESOLVE_MODEL_INVALID`, the code
    // this operation already uses for a package it cannot read. No new code
    // enters the frozen registry for this rule.
    assert_eq!(from_resolve.code, E_RESOLVE_MODEL_INVALID);

    for diagnostic in [&from_initialize, &from_options, &from_resolve] {
        assert!(
            diagnostic.message.contains(symbol),
            "the refusal must name the offending symbol, got: {}",
            diagnostic.message
        );
        assert!(
            diagnostic.message.contains("must be recompiled"),
            "the refusal must say the package has to be recompiled, got: {}",
            diagnostic.message
        );
    }
}

/// The control the cases below are read against: the same shape with every
/// symbol legal loads and answers exactly as it does today.
#[test]
fn h3rm_the_clean_package_still_loads_and_offers_its_options() {
    let (temp_dir, handle) = h3rm_package("clean", &h3rm_clean_config());

    let options = get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        facet: "environment".to_string(),
        selection_state: h3rm_empty_state(&handle),
        include_pruned_reasons: false,
    });
    assert_eq!(
        options.status,
        OperationStatus::Ok,
        "the clean package must still load: {:?}",
        options.diagnostics.diagnostics
    );
    assert_eq!(
        options.valid_options,
        vec!["dev".to_string(), "prod".to_string()]
    );

    // A binding's own domain is its catalogue's entry ids (ADR-0057 §D3), so
    // this is the surface a rewritten entry id would move.
    let bound = get_selection_options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        facet: "line_container".to_string(),
        selection_state: h3rm_empty_state(&handle),
        include_pruned_reasons: false,
    });
    assert_eq!(bound.status, OperationStatus::Ok);
    assert_eq!(
        bound.valid_options,
        vec!["c1".to_string(), "c2".to_string()]
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

/// T1. A catalogue entry id crafted to close its own literal and continue with
/// valid grammar — the symbol both `accepts_disjunction` and the `derive`
/// lowering hand to `facet_eq_predicate`.
#[test]
fn h3rm_a_package_declaring_an_injected_catalogue_entry_id_is_refused_at_load() {
    let mut config = h3rm_clean_config();
    let injected = "c1' || site == 'x";
    let catalogue = config
        .catalogues
        .get_mut("containers")
        .expect("the clean catalogue");
    let entry = catalogue.entries.remove("c1").expect("the clean entry");
    catalogue.entries.insert(injected.to_string(), entry);
    let derive = config
        .bindings
        .get_mut("line_container")
        .expect("the clean binding")
        .derive
        .as_mut()
        .expect("the clean derive table");
    derive
        .get_mut("environment")
        .expect("the derive source")
        .insert("dev".to_string(), injected.to_string());

    let (temp_dir, handle) = h3rm_package("entry-id", &config);
    h3rm_assert_every_operation_refuses(&handle, injected);
    std::fs::remove_dir_all(&temp_dir).ok();
}

/// T2a. configflux-mrm6 case 3 on the consumption path: a declared facet value
/// holding BOTH quote characters, which no quote choice in
/// `facet_eq_predicate` can contain.
#[test]
fn h3rm_a_package_declaring_an_injected_facet_value_is_refused_at_load() {
    let mut config = h3rm_clean_config();
    let injected = "a'b\" || environment == \"zz";
    config
        .facets
        .get_mut("replica_class")
        .expect("the clean facet")
        .values
        .push(injected.to_string());

    let (temp_dir, handle) = h3rm_package("facet-value", &config);
    h3rm_assert_every_operation_refuses(&handle, injected);
    std::fs::remove_dir_all(&temp_dir).ok();
}

/// T2b. configflux-mrm6 case 2 on the consumption path: a facet KEY that
/// injects a phantom value into the closed `environment` domain.
#[test]
fn h3rm_a_package_declaring_an_injected_facet_key_is_refused_at_load() {
    let mut config = h3rm_clean_config();
    let injected = "environment != 'zz' || replica_class";
    let facet = config
        .facets
        .remove("replica_class")
        .expect("the clean facet");
    config.facets.insert(injected.to_string(), facet);
    // The binding's derive table names `environment`, not `replica_class`, so
    // nothing else in the model moves with the rename.

    let (temp_dir, handle) = h3rm_package("facet-key", &config);
    h3rm_assert_every_operation_refuses(&handle, injected);
    std::fs::remove_dir_all(&temp_dir).ok();
}

/// A binding id is refused as a BINDING id, not under the wider facet wording
/// it would collect from the ADR-0057 §D3 projection, and a catalogue entry id
/// is refused as an ENTRY id rather than as the facet value that projection
/// makes of it. The class order inside `validate_symbol_charset` is the whole
/// content of this case — nothing else can distinguish it.
#[test]
fn h3rm_each_refused_symbol_is_named_under_its_narrowest_class() {
    let mut config = h3rm_clean_config();
    let injected_binding = "line_container' || environment == 'zz";
    let binding = config
        .bindings
        .remove("line_container")
        .expect("the clean binding");
    config
        .bindings
        .insert(injected_binding.to_string(), binding);

    let (temp_dir, handle) = h3rm_package("binding-id", &config);
    let refusal = load_selection_constraint_model(&handle)
        .expect_err("a binding id violating the rule must be refused")
        .to_string();
    assert!(
        refusal.contains("Binding id") && refusal.contains(injected_binding),
        "the refusal must name the class and the symbol, got: {refusal}"
    );
    std::fs::remove_dir_all(&temp_dir).ok();

    let mut config = h3rm_clean_config();
    let injected_entry = "c1' || site == 'x";
    let catalogue = config
        .catalogues
        .get_mut("containers")
        .expect("the clean catalogue");
    let entry = catalogue.entries.remove("c1").expect("the clean entry");
    catalogue.entries.insert(injected_entry.to_string(), entry);

    let (temp_dir, handle) = h3rm_package("entry-id-class", &config);
    let refusal = load_resolve_model(&handle)
        .expect_err("an entry id violating the rule must be refused")
        .to_string();
    assert!(
        refusal.contains("entry id") && refusal.contains(injected_entry),
        "an entry id must not be reported as a facet value, got: {refusal}"
    );
    std::fs::remove_dir_all(&temp_dir).ok();
}

// ---------------------------------------------------------------------------
// configflux-8nhr — how many times one operation reads its package.
//
// `resolve_from_selection` builds three models out of one package: the
// admissibility screen's `SelectionConstraintModel`, the `ResolveModel`, and
// the closed-facet table's `SelectionConstraintModel` again. Each builder used
// to read the index and open every chunk file for itself, and each was preceded
// by an integrity walk that opens every chunk file too — three index reads and
// six opens of every chunk per resolve, every one of them returning bytes an
// earlier pass had already read, since a package cannot change under one call.
//
// The two pins below fix the count at one index read and one open per chunk, so
// a future load added back to this path fails a test instead of silently
// doubling the I/O again.
// ---------------------------------------------------------------------------

/// Run `op` and report it with the package reads it performed, as
/// `(value, index reads, chunk opens)`.
fn measure_package_reads<T>(op: impl FnOnce() -> T) -> (T, u64, u64) {
    let index_before = ir::INDEX_LOADS.with(|count| count.get());
    let chunks_before = ir::CHUNK_LOADS.with(|count| count.get());
    let value = op();
    (
        value,
        ir::INDEX_LOADS.with(|count| count.get()) - index_before,
        ir::CHUNK_LOADS.with(|count| count.get()) - chunks_before,
    )
}

/// The `chunk-*.cfir` files a package directory actually holds.
///
/// Read off the filesystem rather than taken from `index.chunks` so the pins
/// below are stated against the files on disk — the thing being opened — and an
/// index that disagreed with its own chunk set could not satisfy them by
/// agreeing with the count it supplied.
fn chunk_file_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .expect("read the package directory")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("chunk-") && name.ends_with(".cfir")
        })
        .count()
}

const S1_SMOKE_SCOPE: &str = "component:thermal_control";

fn s1_smoke_context() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("cooling_brand".to_string(), "hydra".to_string()),
        ("cooling_model".to_string(), "x200".to_string()),
        ("pump_type".to_string(), "dual".to_string()),
        ("region".to_string(), "us".to_string()),
    ])
}

#[test]
fn one_resolve_reads_the_index_once_and_opens_each_chunk_once() {
    let (temp_dir, index) = emitted_cmp_dir();
    let handle = model_handle_for(&temp_dir, &index);
    let chunk_files = chunk_file_count(&temp_dir);
    assert_eq!(
        chunk_files,
        index.chunks.len(),
        "the fixture's index must name every chunk file it ships"
    );

    let state = canonical_selection_state(
        handle.model_hash.clone(),
        S1_SMOKE_SCOPE.to_string(),
        s1_smoke_context(),
        BTreeMap::new(),
    )
    .expect("canonical selection state");

    let (resolved, index_reads, chunk_opens) = measure_package_reads(|| {
        resolve_from_selection(ResolveFromSelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: S1_SMOKE_SCOPE.to_string(),
            selection_state: state,
            implied_choices: Default::default(),
        })
    });

    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "the pin must be measured over a resolve that runs the whole path, \
         including the closed-facet table at the end: {:?}",
        resolved.diagnostics.diagnostics
    );
    assert_eq!(
        index_reads, 1,
        "one resolve must read the index exactly once"
    );
    assert_eq!(
        chunk_opens, chunk_files as u64,
        "one resolve must open each of the {chunk_files} chunk files exactly once"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

const FOUR_CHUNK_FACETS: &str = r#"{
  "package": "p",
  "version": "1.0",
  "facets": {
    "environment": {"values": ["dev", "prod"], "default": "dev"},
    "log_level": {"values": ["info", "debug"], "default": "info"}
  },
  "constraints": {
    "zeta_forbids_debug": {"condition": "environment != 'prod' || log_level != 'debug'"}
  }
}"#;

const FOUR_CHUNK_WEBAPP: &str = r#"{
  "package": "p",
  "version": "1.0",
  "components": {
    "webapp": {"type": "service", "condition": "environment == 'dev'", "params": {}}
  }
}"#;

const FOUR_CHUNK_WORKER: &str = r#"{
  "package": "p",
  "version": "1.0",
  "components": {
    "worker": {"type": "service", "condition": "log_level == 'info'", "params": {}}
  }
}"#;

const FOUR_CHUNK_REPORTER: &str = r#"{
  "package": "p",
  "version": "1.0",
  "components": {
    "reporter": {"type": "service", "params": {}}
  }
}"#;

/// A package of FOUR chunks, so the pin scales with the chunk count rather than
/// holding only for the two-chunk shape every other fixture here has.
fn four_chunk_model() -> (PathBuf, ModelHandle) {
    let temp_dir = unique_temp_path("cfx-loader-four-chunk", "fixture");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let mut compiler = Compiler::new();
    for (source_id, chunk) in [
        ("00_facets.json", FOUR_CHUNK_FACETS),
        ("10_webapp.json", FOUR_CHUNK_WEBAPP),
        ("20_worker.json", FOUR_CHUNK_WORKER),
        ("30_reporter.json", FOUR_CHUNK_REPORTER),
    ] {
        compiler
            .add_chunk_auto(source_id, chunk)
            .expect("add chunk");
    }
    let index = compiler.emit_ir(&temp_dir).expect("emit ir");

    let handle = model_handle_for(&temp_dir, &index);
    (temp_dir, handle)
}

#[test]
fn the_one_open_per_chunk_pin_holds_on_a_four_chunk_package() {
    let (temp_dir, handle) = four_chunk_model();
    let chunk_files = chunk_file_count(&temp_dir);
    assert_eq!(
        chunk_files, 4,
        "the fixture must ship four chunk files for this pin to say anything \
         the two-chunk case does not"
    );

    let (resolved, index_reads, chunk_opens) = measure_package_reads(|| {
        resolve_with_choices(&handle, &[("environment", "dev"), ("log_level", "info")])
    });

    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "dev + info breaks no policy: {:?}",
        resolved.diagnostics.diagnostics
    );
    assert_eq!(index_reads, 1, "one resolve reads the index exactly once");
    assert_eq!(
        chunk_opens, 4,
        "one resolve opens each of the four chunk files exactly once"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

// ---------------------------------------------------------------------------
// configflux-8nhr — error parity across the shared read.
//
// Sharing one read between the funnels can only be invisible if each funnel
// still refuses in its own words. The three cases below pin the whole message a
// package fault reaches the caller as: the integrity refusal (whose wording
// names the funnel, and so is the one the sharing could blur), the missing
// index, and the package only the selection loader refuses.
// ---------------------------------------------------------------------------

#[test]
fn an_integrity_failure_is_reported_in_the_words_of_the_funnel_that_hit_it() {
    let (temp_dir, handle) = constraint_model_handle();
    let index = ir::load_index(&handle.index_ref).expect("load index");
    let missing = temp_dir.join(format!(
        "chunk-{}.cfir",
        index.chunks.first().expect("a chunk").chunk_hash
    ));
    std::fs::remove_file(&missing).expect("remove one chunk file");

    // The RESOLVE funnel. This is the fault both loaders share, which is the
    // case the screen stays silent for, so what reaches the envelope is the
    // refusal the resolve loader rendered — in the resolve model's words.
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: inadmissible_state(&handle),
        implied_choices: Default::default(),
    });
    assert_eq!(resolved.status, OperationStatus::Error);
    assert_eq!(resolved.diagnostics.diagnostics.len(), 1);
    assert_eq!(
        resolved.diagnostics.diagnostics[0].code,
        E_RESOLVE_MODEL_INVALID
    );
    assert_eq!(
        resolved.diagnostics.diagnostics[0].message,
        format!(
            "Resolve model integrity check failed for chunk directory '{}'",
            handle.chunk_set_ref
        )
    );

    // The SELECTION funnel over the same package, which must NOT have picked up
    // the resolve funnel's wording from the read the two now share.
    let initialized = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        context_tags: BTreeMap::new(),
    });
    assert_eq!(initialized.status, OperationStatus::Error);
    assert_eq!(initialized.diagnostics.diagnostics.len(), 1);
    assert_eq!(
        initialized.diagnostics.diagnostics[0].code,
        E_LOADER_INDEX_INVALID
    );
    assert_eq!(
        initialized.diagnostics.diagnostics[0].message,
        format!(
            "Selection model integrity check failed for chunk directory '{}'",
            handle.chunk_set_ref
        )
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_package_with_no_index_resolves_to_the_index_read_failure_verbatim() {
    let (temp_dir, mut handle) = constraint_model_handle();
    handle.index_ref = temp_dir
        .join("no-such-index.json")
        .to_string_lossy()
        .into_owned();

    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: inadmissible_state(&handle),
        implied_choices: Default::default(),
    });

    assert_eq!(resolved.status, OperationStatus::Error);
    assert_eq!(resolved.diagnostics.diagnostics.len(), 1);
    assert_eq!(
        resolved.diagnostics.diagnostics[0].code,
        E_RESOLVE_MODEL_INVALID
    );
    assert_eq!(
        resolved.diagnostics.diagnostics[0].message,
        format!("Failed to load index '{}'", handle.index_ref)
    );
    assert_eq!(
        resolved.diagnostics.diagnostics[0].source_id.as_deref(),
        Some(handle.index_ref.as_str())
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn a_package_only_the_selection_loader_refuses_resolves_in_that_loader_s_words() {
    let (temp_dir, handle) = constraint_model_handle();
    let index = ir::load_index(&handle.index_ref).expect("load index");
    let handle = break_one_constraint_expression(&temp_dir, &index);

    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: inadmissible_state(&handle),
        implied_choices: Default::default(),
    });

    assert_eq!(resolved.status, OperationStatus::Error);
    assert_eq!(resolved.diagnostics.diagnostics.len(), 1);
    let diagnostic = &resolved.diagnostics.diagnostics[0];
    assert_eq!(diagnostic.code, E_RESOLVE_MODEL_INVALID);
    // The whole frame is pinned; only the constraint id and the chunk address
    // it was declared in are left free, and both are the fixture's own.
    assert!(
        diagnostic.message.starts_with("Constraint '")
            && diagnostic.message.contains("' in chunk '")
            && diagnostic
                .message
                .ends_with("' has an unparseable expression '('"),
        "the selection loader's refusal must reach the caller verbatim, got: {}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.source_id.as_deref(),
        Some(handle.index_ref.as_str())
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}
