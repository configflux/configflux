// SPDX-License-Identifier: BUSL-1.1

//! configflux-y2ai — the resolve-hash DRIFT TRIPWIRE.
//!
//! `resolve_hash` is produced on two paths: the loader emits it when a resolve
//! succeeds, and the runtime recomputes it when a caller bridges that resolve
//! into a session, refusing the open with `E_RUNTIME_HASH_MISMATCH` if the two
//! disagree. Until configflux-y2ai each path carried its own transcribed copy of
//! the pre-image struct and the hashing, kept identical by a comment asking the
//! next editor to keep them identical. One edit to one copy would have produced
//! `E_RUNTIME_HASH_MISMATCH` on every selection-bearing open in the product,
//! wearing the shape of a caller mistake.
//!
//! These tests are what stands in place of that comment. They feed IDENTICAL
//! inputs through both public paths and assert the two `resolve_hash` values are
//! the same bytes. If someone re-forks the recipe — a second pre-image struct, a
//! reordered field, a dropped `skip_serializing_if`, a different hash function on
//! one side — these fail, here, instead of in a user's runtime open.
//!
//! They are a TRIPWIRE, NOT A VALUE PIN. No hash literal appears below, and none
//! should be added: a deliberate, agreed rotation of the recipe must be free to
//! move the value, and must NOT be free to move it on only one of the two paths.
//! The pinned literals live in the scenario goldens and the byte-stability
//! baselines, which is the right place for that separate job.
//!
//! Black-box: everything goes through the public product, loader and runtime
//! APIs. Neither test can see which module the recipe lives in, so a future
//! relocation of it needs no edit here.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use compiler::loader_api::{
    apply_selection, initialize_selection_state, open_model, resolve_from_selection,
    ApplySelectionRequest, InitializeSelectionStateRequest, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, ResolveResult, SelectionDelta, SelectionState,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use compiler::runtime_api::{expected_resolve_hash, runtime_open, RuntimeOpenRequest};

/// Four declared closed facets, each with a default and each referenced by its
/// own probe parameter so all four are live during resolve.
///
/// Four is the point: it is the only shape that binds one facet at every
/// provenance level at once, so a single resolve carries a NON-EMPTY
/// `context_tags`, `choices`, `defaulted_choices` AND `implied_choices`. Every
/// field of the pre-image is then present in the serialized bytes, including the
/// two that skip themselves when empty — which is exactly the coverage a
/// re-forked recipe has to fail.
const FOUR_LEVEL_FIXTURE: &str = r#"{
    "package": "resolve_hash_parity",
    "version": "1.0.0",
    "definitions": {
        "mark": {
            "type": "string",
            "doc": "Provenance probe",
            "lifecycle": "runtime",
            "safety": "q_m",
            "access": "technician"
        }
    },
    "facets": {
        "bychoice":  { "values": ["c1", "c2"], "default": "c1" },
        "bydefault": { "values": ["d1", "d2"], "default": "d1" },
        "byimplied": { "values": ["i1", "i2"], "default": "i1" },
        "bytag":     { "values": ["t1", "t2"], "default": "t1" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "p_choice": {
                    "inherits": "mark", "type": "string", "lifecycle": "runtime",
                    "safety": "q_m", "access": "technician", "value": "unset",
                    "overrides": [
                        {"condition": "bychoice == 'c1'", "value": "C1"},
                        {"condition": "bychoice == 'c2'", "value": "C2"}
                    ]
                },
                "p_default": {
                    "inherits": "mark", "type": "string", "lifecycle": "runtime",
                    "safety": "q_m", "access": "technician", "value": "unset",
                    "overrides": [
                        {"condition": "bydefault == 'd1'", "value": "D1"},
                        {"condition": "bydefault == 'd2'", "value": "D2"}
                    ]
                },
                "p_implied": {
                    "inherits": "mark", "type": "string", "lifecycle": "runtime",
                    "safety": "q_m", "access": "technician", "value": "unset",
                    "overrides": [
                        {"condition": "byimplied == 'i1'", "value": "I1"},
                        {"condition": "byimplied == 'i2'", "value": "I2"}
                    ]
                },
                "p_tag": {
                    "inherits": "mark", "type": "string", "lifecycle": "runtime",
                    "safety": "q_m", "access": "technician", "value": "unset",
                    "overrides": [
                        {"condition": "bytag == 't1'", "value": "T1"},
                        {"condition": "bytag == 't2'", "value": "T2"}
                    ]
                }
            }
        }
    }
}"#;

/// A model with no facets at all: every provenance map on its resolve is empty.
///
/// The other half of the coverage. `defaulted_choices` and `implied_choices`
/// carry `skip_serializing_if`, so on this model they contribute NO bytes to the
/// pre-image. A fork that dropped the attribute on one side only would still
/// agree on the four-level fixture (where both maps are non-empty and serialize
/// either way) and diverge here.
const FACET_FREE_FIXTURE: &str = r#"{
    "package": "resolve_hash_parity_facet_free",
    "version": "1.0.0",
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// The scope both tests resolve and open at. `runtime_open` accepts only
/// `component:<component_id>`, so the loader must be asked for the same scope
/// the runtime will be handed — otherwise the bridge cannot be faithful and the
/// comparison would be testing the wrong thing.
const SCOPE: &str = "component:svc";

struct Compiled {
    output_dir: PathBuf,
    handle: ModelHandle,
}

fn compile_fixture(label: &str, source: &str) -> Compiled {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "scenarios/resolve_hash_parity/00_model.json".to_string(),
            inline_content: source.to_string(),
        }],
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "compile must succeed: {:?}",
        result.verify_report
    );
    let cmp_manifest_ref = result
        .compiled_model_package_ref
        .clone()
        .expect("cmp manifest ref present");
    let open_result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref,
    });
    assert_eq!(
        open_result.status,
        OperationStatus::Ok,
        "open_model must succeed: {:?}",
        open_result.diagnostics
    );
    Compiled {
        handle: open_result.model_handle.expect("model handle present"),
        output_dir,
    }
}

fn selection_state(handle: &ModelHandle, context_tags: BTreeMap<String, String>) -> SelectionState {
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: SCOPE.to_string(),
        context_tags,
    });
    assert_eq!(
        init.status,
        OperationStatus::Ok,
        "initialize_selection_state must succeed: {:?}",
        init.diagnostics
    );
    init.selection_state.expect("selection_state present")
}

/// The faithful bridge: every field `runtime_open` recomputes over, copied from
/// the resolve result unmodified.
///
/// Faithfulness is the whole precondition of these tests. `runtime_open` is
/// designed to reject an UNFAITHFUL bridge — that is the guard working, and
/// configflux-9991 was a false bug report produced by exactly such a bridge. A
/// dropped field here would make these tests fail for a reason that has nothing
/// to do with recipe drift, so the copy below is deliberately total.
fn faithful_open_request(resolved: &ResolveResult) -> RuntimeOpenRequest {
    serde_json::from_value(serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": resolved.model_hash,
        "resolve_hash": resolved.resolve_hash.clone().expect("resolve_hash present"),
        "scope": resolved.scope,
        "resolved_output": resolved.resolved_output.clone().expect("resolved_output present"),
        "resolved_component_dependencies": resolved.resolved_component_dependencies,
        "resolved_artifacts": resolved.resolved_artifacts,
        "context_tags": resolved.context_tags,
        "choices": resolved.choices,
        "defaulted_choices": resolved.defaulted_choices,
        "implied_choices": resolved.implied_choices,
    }))
    .expect("valid runtime open request")
}

/// Assert the two paths produce the SAME `resolve_hash` bytes for the resolve
/// they were both handed, and that the open they gate actually succeeds.
///
/// Two assertions rather than one because they fail differently and a reader
/// deserves to know which happened. `expected_resolve_hash` compares the values
/// directly and reports both when they differ; `runtime_open` proves the value
/// is the one the real gate consults, so a hash that matched but was computed
/// somewhere the open does not look could not pass unnoticed.
fn assert_paths_agree(resolved: &ResolveResult, case: &str) {
    let loader_hash = resolved
        .resolve_hash
        .clone()
        .expect("a successful resolve emits a resolve_hash");
    let request = faithful_open_request(resolved);

    let runtime_hash =
        expected_resolve_hash(&request).expect("the runtime can canonicalize a faithful payload");
    assert_eq!(
        loader_hash, runtime_hash,
        "[{case}] the loader and the runtime disagree on resolve_hash for IDENTICAL inputs. \
         The two paths must share one recipe (crate::resolve_hash); a second copy of the \
         pre-image, a reordered field, a dropped skip_serializing_if, or a different hash \
         function on one side is what this test exists to catch. Fix the fork — do NOT \
         update an expected value here, there isn't one."
    );

    let opened = runtime_open(request);
    assert_eq!(
        opened.status,
        OperationStatus::Ok,
        "[{case}] a faithfully bridged resolve must open: {:?}",
        opened.diagnostics
    );
}

/// Every field of the pre-image present at once: `context_tags`, `choices`,
/// `defaulted_choices` and `implied_choices` are all non-empty, one per
/// provenance level, so the serialized bytes carry the complete struct.
#[test]
fn loader_and_runtime_agree_on_the_resolve_hash_for_identical_inputs() {
    let compiled = compile_fixture("resolve-hash-parity-four-level", FOUR_LEVEL_FIXTURE);

    let mut context_tags = BTreeMap::new();
    context_tags.insert("bytag".to_string(), "t2".to_string());
    let state = selection_state(&compiled.handle, context_tags);

    let applied = apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: SCOPE.to_string(),
        selection_state: state,
        selection_delta: SelectionDelta {
            facet: "bychoice".to_string(),
            option: "c2".to_string(),
        },
    });
    assert_eq!(
        applied.status,
        OperationStatus::Ok,
        "apply bychoice=c2 must succeed: {:?}",
        applied.diagnostics
    );
    let state = applied.selection_state.expect("selection_state present");

    let mut implied = BTreeMap::new();
    implied.insert("byimplied".to_string(), "i2".to_string());
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: SCOPE.to_string(),
        selection_state: state,
        implied_choices: implied,
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "resolve must succeed: {:?}",
        resolved.diagnostics
    );

    // The precondition this case exists for. Asserted rather than assumed: if a
    // future change to defaulting or precedence emptied one of these maps, the
    // parity assertion below would silently stop covering that field and the
    // test would keep passing while testing less.
    assert!(
        !resolved.context_tags.is_empty(),
        "context_tags must be non-empty: {:?}",
        resolved.context_tags
    );
    assert!(
        !resolved.choices.is_empty(),
        "choices must be non-empty: {:?}",
        resolved.choices
    );
    assert!(
        !resolved.defaulted_choices.is_empty(),
        "defaulted_choices must be non-empty: {:?}",
        resolved.defaulted_choices
    );
    assert!(
        !resolved.implied_choices.is_empty(),
        "implied_choices must be non-empty: {:?}",
        resolved.implied_choices
    );

    assert_paths_agree(&resolved, "all four provenance maps non-empty");

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// The skip-if-empty arm: nothing was tagged, chosen, defaulted or implied, so
/// both maps that carry `skip_serializing_if` contribute no bytes. A fork that
/// dropped the attribute on one side would agree on the four-level case above
/// and diverge here.
#[test]
fn loader_and_runtime_agree_on_the_resolve_hash_when_every_provenance_map_is_empty() {
    let compiled = compile_fixture("resolve-hash-parity-facet-free", FACET_FREE_FIXTURE);
    let state = selection_state(&compiled.handle, BTreeMap::new());

    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: SCOPE.to_string(),
        selection_state: state,
        implied_choices: BTreeMap::new(),
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "facet-free resolve must succeed: {:?}",
        resolved.diagnostics
    );

    // The precondition: this case is only the skip-if-empty case while all four
    // maps really are empty.
    assert!(
        resolved.context_tags.is_empty()
            && resolved.choices.is_empty()
            && resolved.defaulted_choices.is_empty()
            && resolved.implied_choices.is_empty(),
        "a facet-free resolve must record no provenance at all: tags={:?} choices={:?} \
         defaulted={:?} implied={:?}",
        resolved.context_tags,
        resolved.choices,
        resolved.defaulted_choices,
        resolved.implied_choices
    );

    assert_paths_agree(&resolved, "every provenance map empty");

    fs::remove_dir_all(&compiled.output_dir).ok();
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
