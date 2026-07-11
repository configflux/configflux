// SPDX-License-Identifier: BUSL-1.1

//! configflux-3gk3 / ADR-0047 §5/§6: resolve-time auto-bind of declared-facet
//! defaults, the `defaulted_choices` provenance, the `resolve_hash` fold (with
//! the runtime_api lockstep cross-validation), and the `E_RESOLVE_FACET_UNBOUND`
//! usage error. Black-box, end-to-end through the public product/loader/runtime
//! API — never reaching into module internals.
//!
//! The load-bearing case is **F2**: a facet's DEFAULT arm resolves with NOTHING
//! selected. Before this change an empty-selection resolve of a model whose
//! condition referenced an unbound facet failed with a misleading "unsatisfiable"
//! (`E_RESOLVE_CONTEXT_UNSATISFIED`). After it, a declared default is auto-bound
//! (recorded in `defaulted_choices`), and a declared facet with NO default that
//! an active condition needs produces the precise `E_RESOLVE_FACET_UNBOUND`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use compiler::loader_api::{
    initialize_selection_state, open_model, resolve_from_selection,
    InitializeSelectionStateRequest, ModelHandle, OpenModelRequest, ResolveFromSelectionRequest,
    ResolveResult, SelectionState, E_RESOLVE_FACET_UNBOUND,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use compiler::runtime_api::{runtime_open, RuntimeOpenRequest};

/// Closed facet `region` over `[eu, us, apac]` with `apac` the default arm. NO
/// condition references `region`, so `apac` is the invisible-before-ADR-0047
/// default arm (F2). An empty-selection resolve must auto-bind it.
const CLOSED_DEFAULT_FIXTURE: &str = r#"{
    "package": "facet_f2_resolve",
    "version": "1.0.0",
    "facets": {
        "region": { "values": ["eu", "us", "apac"], "default": "apac", "open": false }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// A facet-free model: proves the skip-if-empty invariant — its resolve emits no
/// `defaulted_choices` key and its `resolve_hash` pre-image is byte-unchanged by
/// this feature.
const FACET_FREE_FIXTURE: &str = r#"{
    "package": "facet_free",
    "version": "1.0.0",
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// Closed facet `region` with NO default, referenced by an ACTIVE condition. An
/// empty-selection resolve leaves `region` unbound, and the condition needs it —
/// the precise `E_RESOLVE_FACET_UNBOUND` (naming the declared domain) fires.
const DEFAULTLESS_NEEDED_FIXTURE: &str = r#"{
    "package": "facet_defaultless",
    "version": "1.0.0",
    "facets": {
        "region": { "values": ["eu", "us"], "open": false }
    },
    "components": {
        "svc": { "type": "service", "condition": "region == 'eu'" }
    }
}"#;

struct Compiled {
    output_dir: PathBuf,
    handle: ModelHandle,
}

fn compile_fixture(label: &str, source: &str) -> Compiled {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "scenarios/facet/00_facets.json".to_string(),
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
    assert_eq!(open_result.status, OperationStatus::Ok, "open_model ok");
    Compiled {
        handle: open_result.model_handle.expect("model handle present"),
        output_dir,
    }
}

fn selection_state(handle: &ModelHandle, scope: &str, context_tags: BTreeMap<String, String>) -> SelectionState {
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        context_tags,
    });
    assert_eq!(
        init.status,
        OperationStatus::Ok,
        "init selection state ok: {:?}",
        init.diagnostics
    );
    init.selection_state.expect("selection_state present")
}

fn resolve(handle: &ModelHandle, scope: &str, state: &SelectionState) -> ResolveResult {
    resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
    })
}

#[test]
fn empty_selection_resolve_auto_binds_declared_default_with_provenance() {
    let compiled = compile_fixture("facet-resolve-default", CLOSED_DEFAULT_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());
    let resolved = resolve(&compiled.handle, "all", &state);

    // (a) The empty-selection resolve SUCCEEDS — the default arm is bound.
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "empty-selection resolve of a defaulted facet must succeed: {:?}",
        resolved.diagnostics
    );

    // (b) Provenance records exactly the auto-bound default.
    let mut expected = BTreeMap::new();
    expected.insert("region".to_string(), "apac".to_string());
    assert_eq!(
        resolved.defaulted_choices, expected,
        "defaulted_choices must record the auto-bound region=apac"
    );

    // (c) `resolve_hash` is present and deterministic across repeat resolves.
    let hash_a = resolved.resolve_hash.clone().expect("resolve_hash present");
    let resolved_b = resolve(&compiled.handle, "all", &state);
    assert_eq!(
        Some(hash_a),
        resolved_b.resolve_hash,
        "resolve_hash must be deterministic across repeat resolves"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

#[test]
fn facet_free_resolve_emits_no_defaulted_choices_key() {
    let compiled = compile_fixture("facet-free-resolve", FACET_FREE_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());
    let resolved = resolve(&compiled.handle, "all", &state);

    assert_eq!(resolved.status, OperationStatus::Ok, "facet-free resolve ok");
    assert!(
        resolved.defaulted_choices.is_empty(),
        "a facet-free model must record no defaulted choices"
    );
    // Skip-if-empty: the serialized envelope carries no `defaulted_choices` key,
    // so the resolve_hash pre-image is byte-unchanged by this feature.
    let json = serde_json::to_value(&resolved).expect("serialize resolve result");
    assert!(
        json.get("defaulted_choices").is_none(),
        "facet-free resolve JSON must omit the defaulted_choices key"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

#[test]
fn defaultless_facet_needed_by_condition_yields_facet_unbound() {
    let compiled = compile_fixture("facet-defaultless", DEFAULTLESS_NEEDED_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());
    let resolved = resolve(&compiled.handle, "all", &state);

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "a defaultless facet an active condition needs must fail resolve"
    );
    let diag = resolved
        .diagnostics
        .diagnostics
        .first()
        .expect("a diagnostic is present");
    assert_eq!(
        diag.code, E_RESOLVE_FACET_UNBOUND,
        "must be the precise facet-unbound code, not the generic unsat fold"
    );
    // The message names the facet and its declared domain.
    assert!(diag.message.contains("region"), "names the facet: {}", diag.message);
    assert!(
        diag.message.contains("eu") && diag.message.contains("us"),
        "names the declared domain: {}",
        diag.message
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// ADR-0047 §5 lockstep. Resolve with an auto-bound defaulted facet AND a
/// non-empty `context_tags`, then hand the returned `resolve_hash` +
/// `defaulted_choices` to `runtime_open`: the runtime's INDEPENDENT resolve-hash
/// recipe must reproduce the same hash and SUCCEED. A tampered `defaulted_choices`
/// must FAIL CLOSED with `E_RUNTIME_HASH_MISMATCH`.
#[test]
fn runtime_open_cross_validates_defaulted_resolve_hash() {
    let compiled = compile_fixture("facet-xval", CLOSED_DEFAULT_FIXTURE);
    let scope = "component:svc";
    let mut context_tags = BTreeMap::new();
    context_tags.insert("deployment".to_string(), "prod".to_string());
    let state = selection_state(&compiled.handle, scope, context_tags);
    let resolved = resolve(&compiled.handle, scope, &state);

    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "component-scoped resolve with an auto-bound default must succeed: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.defaulted_choices.get("region").map(String::as_str),
        Some("apac"),
        "region=apac must be recorded as the auto-bound default"
    );

    // Faithful hand-off: runtime_open reproduces the hash and succeeds.
    let opened = runtime_open(open_request(&resolved, None));
    assert_eq!(
        opened.status,
        OperationStatus::Ok,
        "runtime_open must reproduce the resolve_hash and succeed: {:?}",
        opened.diagnostics
    );

    // Tampered defaulted_choices → the recomputed hash diverges → fail closed.
    let tampered = runtime_open(open_request(&resolved, Some(("region", "eu"))));
    assert_eq!(
        tampered.status,
        OperationStatus::Error,
        "a tampered defaulted_choices must fail closed"
    );
    assert_eq!(
        tampered
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some("E_RUNTIME_HASH_MISMATCH"),
        "tamper must be rejected as a resolve_hash mismatch"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// Build a `RuntimeOpenRequest` from a resolve result via JSON (serde defaults
/// fill every runtime-only field), copying `defaulted_choices` — the lockstep
/// carry the ADR mandates. `tamper` overrides one `defaulted_choices` entry to
/// exercise the fail-closed path.
fn open_request(resolved: &ResolveResult, tamper: Option<(&str, &str)>) -> RuntimeOpenRequest {
    let mut defaulted = resolved.defaulted_choices.clone();
    if let Some((facet, value)) = tamper {
        defaulted.insert(facet.to_string(), value.to_string());
    }
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
        "defaulted_choices": defaulted,
    }))
    .expect("valid runtime open request")
}

fn tempdir_for(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "configflux-compiler-{test_name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}
