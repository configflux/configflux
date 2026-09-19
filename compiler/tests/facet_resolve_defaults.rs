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

use compiler::loader_api::{
    apply_selection, initialize_selection_state, open_model, resolve_from_selection,
    ApplySelectionRequest, InitializeSelectionStateRequest, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, ResolveResult, SelectionDelta, SelectionState,
    E_RESOLVE_FACET_UNBOUND, RESOLVE_HASH_SELECTION_FIELDS,
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
        implied_choices: Default::default(),
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
/// fill every runtime-only field), copying BOTH provenance maps — the lockstep
/// carry ADR-0047 §5 and ADR-0057 §D6 mandate. `tamper` overrides one
/// `defaulted_choices` entry to exercise the fail-closed path.
fn open_request(resolved: &ResolveResult, tamper: Option<(&str, &str)>) -> RuntimeOpenRequest {
    let mut defaulted = resolved.defaulted_choices.clone();
    if let Some((facet, value)) = tamper {
        defaulted.insert(facet.to_string(), value.to_string());
    }
    open_request_with(resolved, defaulted, resolved.implied_choices.clone())
}

/// The same bridge with BOTH provenance maps supplied explicitly, so a test can
/// drop or edit either one. `drop_implied` is what spec T7 needs: a caller that
/// omits `implied_choices` must be rejected, not silently accepted.
fn open_request_with(
    resolved: &ResolveResult,
    defaulted: BTreeMap<String, String>,
    implied: BTreeMap<String, String>,
) -> RuntimeOpenRequest {
    let hash = resolved.resolve_hash.clone().expect("resolve_hash present");
    open_request_parts(resolved, defaulted, implied, hash)
}

/// The same bridge with the `resolve_hash` supplied explicitly, so a test can
/// present a hash that is well-formed but does NOT belong to the payload it
/// rides on (configflux-zr6m). Every other bridge above forwards the loader's
/// own hash, which is exactly why none of them can reach the forgery case.
fn open_request_parts(
    resolved: &ResolveResult,
    defaulted: BTreeMap<String, String>,
    implied: BTreeMap<String, String>,
    resolve_hash: String,
) -> RuntimeOpenRequest {
    serde_json::from_value(serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": resolved.model_hash,
        "resolve_hash": resolve_hash,
        "scope": resolved.scope,
        "resolved_output": resolved.resolved_output.clone().expect("resolved_output present"),
        "resolved_component_dependencies": resolved.resolved_component_dependencies,
        "resolved_artifacts": resolved.resolved_artifacts,
        "context_tags": resolved.context_tags,
        "choices": resolved.choices,
        "defaulted_choices": defaulted,
        "implied_choices": implied,
    }))
    .expect("valid runtime open request")
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}

// ---------------------------------------------------------------------------
// configflux-secb.3 (ADR-0057 §D6) — T7: the runtime lockstep, and the
// fail-open hole a guard that forgot `implied_choices` would leave behind
// ---------------------------------------------------------------------------

/// T7 — `implied_choices` rides to `runtime_open` and is cross-validated there.
///
/// The selection is EMPTY and `region` is implied rather than defaulted, so
/// `implied_choices` is the ONLY non-empty provenance on the envelope. That is
/// the shape that matters: `runtime_open` skips hash cross-validation entirely
/// when every provenance map is empty, so a guard that tested only
/// `context_tags` / `choices` / `defaulted_choices` would treat this snapshot as
/// having nothing to check and open FAIL-OPEN — accepting a dropped or edited
/// `implied_choices` in silence. This test is that hole's regression.
#[test]
fn runtime_open_cross_validates_implied_choices_as_sole_provenance() {
    let compiled = compile_fixture("facet-implied-lockstep", CLOSED_DEFAULT_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());

    let mut implied = BTreeMap::new();
    implied.insert("region".to_string(), "us".to_string());
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        implied_choices: implied.clone(),
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "resolve must succeed: {:?}",
        resolved.diagnostics
    );

    // The precondition the rest of the test depends on: implied is the ONLY
    // non-empty provenance, which is the shape a guard naming only the other
    // three would wave through.
    assert_eq!(resolved.implied_choices, implied);
    assert!(
        resolved.defaulted_choices.is_empty(),
        "an implied facet must not also be defaulted: {:?}",
        resolved.defaulted_choices
    );
    assert!(resolved.context_tags.is_empty() && resolved.choices.is_empty());

    // Carried faithfully: the runtime reproduces the loader's hash.
    let ok = runtime_open(open_request(&resolved, None));
    assert_eq!(
        ok.status,
        OperationStatus::Ok,
        "a faithfully bridged snapshot must open: {:?}",
        ok.diagnostics
    );

    // EDITED: `implied_choices` is the only non-empty provenance, so this is
    // precisely the payload a guard that tested only context_tags / choices /
    // defaulted_choices would wave through. The recomputed hash diverges and
    // the open fails closed.
    //
    // Dropping every provenance map instead is no longer an escape either:
    // configflux-zr6m removed the all-empty fast path, so that payload is
    // recomputed like any other and rejected. It has its own test below
    // (`runtime_open_rejects_a_provenance_downgrade_that_zeroes_every_map`).
    let mut edited = implied.clone();
    edited.insert("region".to_string(), "eu".to_string());
    let tampered = runtime_open(open_request_with(
        &resolved,
        resolved.defaulted_choices.clone(),
        edited,
    ));
    assert_eq!(
        tampered.status,
        OperationStatus::Error,
        "an edited implied_choices must fail closed; without the implied term in \
         the open-time guard this payload would skip validation and open fail-open"
    );
    let diagnostic = tampered
        .diagnostics
        .diagnostics
        .first()
        .expect("a rejection carries a diagnostic");
    assert_eq!(diagnostic.code, "E_RUNTIME_HASH_MISMATCH");
    assert!(
        diagnostic
            .hint
            .as_deref()
            .unwrap_or_default()
            .contains("implied_choices"),
        "the remediation must NAME the field that was dropped, or it points the \
         user at the wrong one — the drift configflux-j2jj records; got {:?}",
        diagnostic.hint
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// T7, second arm — the realistic DROP. With a context tag present the guard is
/// live regardless, so a bridge that simply forgets to copy `implied_choices`
/// is rejected rather than silently opening a snapshot whose hash it cannot
/// account for. This is the case a caller actually hits: real deployments carry
/// a selection.
#[test]
fn runtime_open_rejects_a_bridge_that_forgets_implied_choices() {
    let compiled = compile_fixture("facet-implied-dropped", CLOSED_DEFAULT_FIXTURE);
    let mut tags = BTreeMap::new();
    tags.insert("region".to_string(), "eu".to_string());
    let state = selection_state(&compiled.handle, "all", tags);

    let mut implied = BTreeMap::new();
    // configflux-v93p: a DECLARED, in-domain facet. This arm is about the
    // RUNTIME bridge, not about the resolve-surface screen that now rejects an
    // undeclared implied facet, so the payload has to be one the screen
    // accepts. `region` deliberately repeats the context tag above, which
    // leaves the post-default assignment byte-identical to what this test
    // resolved before — only the provenance map the bridge must carry changed.
    implied.insert("region".to_string(), "eu".to_string());
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        implied_choices: implied,
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "resolve must succeed: {:?}",
        resolved.diagnostics
    );
    assert!(!resolved.implied_choices.is_empty());

    let dropped = runtime_open(open_request_with(
        &resolved,
        resolved.defaulted_choices.clone(),
        BTreeMap::new(),
    ));
    assert_eq!(
        dropped.status,
        OperationStatus::Error,
        "a bridge that drops implied_choices must fail closed: {:?}",
        dropped.diagnostics
    );
    assert_eq!(
        dropped
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some("E_RUNTIME_HASH_MISMATCH")
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

// ---------------------------------------------------------------------------
// configflux-zr6m — `runtime_open` cross-validates EVERY snapshot, including
// the one whose provenance is entirely empty
// ---------------------------------------------------------------------------

/// A well-formed sha256-hex that is not the hash of anything in this suite.
/// Well-formed matters: `runtime_open` rejects a malformed `resolve_hash` with
/// `E_RUNTIME_OPEN_INVALID` long before the recompute, so a junk string would
/// pass these tests for the wrong reason.
const FOREIGN_RESOLVE_HASH: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The regression guard for the fix: a legitimate provenance-free snapshot —
/// no context tags, no choices, nothing defaulted, nothing implied — must still
/// open once the recompute is unconditional.
///
/// This is the case the removed fast path existed to spare, and the reason it
/// is safe to remove: with all four maps empty both `resolve_hash` recipes
/// skip-serialize the two provenance fields (ADR-0047 §5, ADR-0057 §D6) and
/// canonicalize `resolved_output` through byte-identical implementations, so
/// the runtime reproduces exactly the bytes the loader hashed.
#[test]
fn runtime_open_accepts_a_provenance_free_snapshot_with_its_own_hash() {
    let compiled = compile_fixture("facet-free-open", FACET_FREE_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());
    let resolved = resolve(&compiled.handle, "all", &state);
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "facet-free resolve must succeed: {:?}",
        resolved.diagnostics
    );

    // The precondition that makes this test the one it claims to be.
    assert!(
        resolved.context_tags.is_empty()
            && resolved.choices.is_empty()
            && resolved.defaulted_choices.is_empty()
            && resolved.implied_choices.is_empty(),
        "every provenance map must be empty for this fixture"
    );

    let opened = runtime_open(open_request(&resolved, None));
    assert_eq!(
        opened.status,
        OperationStatus::Ok,
        "an unconditional recompute must still admit a faithful provenance-free \
         snapshot: {:?}",
        opened.diagnostics
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// The hole itself. A provenance-free snapshot carrying a `resolve_hash` that
/// belongs to some OTHER payload must fail closed.
///
/// Before the fix `runtime_open` skipped the recompute whenever all four
/// provenance maps were empty, so this open SUCCEEDED and the snapshot went on
/// to carry a hash it could not account for — and `resolve_hash` is read
/// downstream as an identity token, not as a hint.
#[test]
fn runtime_open_rejects_a_provenance_free_snapshot_bearing_a_foreign_hash() {
    let compiled = compile_fixture("facet-free-foreign-hash", FACET_FREE_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());
    let resolved = resolve(&compiled.handle, "all", &state);
    assert_eq!(resolved.status, OperationStatus::Ok, "resolve ok");
    assert_ne!(
        resolved.resolve_hash.as_deref(),
        Some(FOREIGN_RESOLVE_HASH),
        "the foreign hash must not collide with the real one"
    );

    let forged = runtime_open(open_request_parts(
        &resolved,
        BTreeMap::new(),
        BTreeMap::new(),
        FOREIGN_RESOLVE_HASH.to_string(),
    ));
    assert_eq!(
        forged.status,
        OperationStatus::Error,
        "a hash that belongs to no payload must be rejected even when there is \
         no provenance to check it against: {:?}",
        forged.diagnostics
    );
    assert_eq!(
        forged
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some("E_RUNTIME_HASH_MISMATCH"),
        "the rejection is a hash mismatch, not a shape error"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// The downgrade arm. Take a snapshot whose provenance is genuine — `region`
/// was IMPLIED — and zero all four maps while keeping the loader's real
/// `resolve_hash`. The payload now claims a hash computed over provenance it no
/// longer carries.
///
/// Before the fix this was the cheapest forgery in the contract: emptying every
/// map disarmed the guard outright, so a legitimate hash could be bound to a
/// stripped snapshot without computing a single preimage. Tampering with a map
/// while leaving one non-empty was always caught; erasing them all was not.
#[test]
fn runtime_open_rejects_a_provenance_downgrade_that_zeroes_every_map() {
    let compiled = compile_fixture("facet-downgrade", CLOSED_DEFAULT_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());

    let mut implied = BTreeMap::new();
    implied.insert("region".to_string(), "us".to_string());
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        implied_choices: implied.clone(),
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "resolve must succeed: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.implied_choices, implied,
        "the snapshot's provenance must be genuine before it is stripped"
    );

    let downgraded = runtime_open(open_request_with(
        &resolved,
        BTreeMap::new(),
        BTreeMap::new(),
    ));
    assert_eq!(
        downgraded.status,
        OperationStatus::Error,
        "stripping every provenance map must not buy a caller an unchecked \
         open: {:?}",
        downgraded.diagnostics
    );
    let diagnostic = downgraded
        .diagnostics
        .diagnostics
        .first()
        .expect("a rejection carries a diagnostic");
    assert_eq!(diagnostic.code, "E_RUNTIME_HASH_MISMATCH");
    // Held against the const the hint is BUILT from, not against a list copied
    // out of it. Naming every field the pre-image folds in is the hint's whole
    // job, and `RESOLVE_HASH_SELECTION_FIELDS` is the single definition of that
    // set (configflux-j2jj); asserting one field would leave the hint free to
    // narrow back to exactly the drift that const exists to prevent.
    let hint = diagnostic.hint.as_deref().unwrap_or_default();
    for field in RESOLVE_HASH_SELECTION_FIELDS.split(", ") {
        assert!(
            hint.contains(field),
            "the remediation must name every provenance field the resolve-hash \
             pre-image folds in; `{field}` is missing from {hint:?}"
        );
    }

    fs::remove_dir_all(&compiled.output_dir).ok();
}

// ---------------------------------------------------------------------------
// configflux-secb.3 (ADR-0057 §D6) — T1: the four-level precedence ladder
// ---------------------------------------------------------------------------

/// One probe parameter per precedence level, each overridden on its own facet's
/// values, so the resolved value NAMES which level won. Four declared closed
/// facets, all with defaults, so every level has a default to beat.
const PRECEDENCE_FIXTURE: &str = r#"{
    "package": "facet_precedence",
    "version": "1.0.0",
    "definitions": {
        "mark": {
            "type": "string",
            "doc": "Precedence probe",
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

/// The resolved value of `svc.<param>`, found without hard-coding the scope
/// root — the root key is the scope's, and this test is about precedence, not
/// about the shape of the envelope around it.
fn probe(resolved: &ResolveResult, param: &str) -> String {
    let output = resolved
        .resolved_output
        .as_ref()
        .expect("a successful resolve delivers a payload");
    let root = output
        .as_object()
        .expect("resolved_output is an object")
        .values()
        .next()
        .expect("resolved_output carries one scope root");
    root["components"]["svc"]["params"][param]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("svc.{param} resolved to no string value"))
        .to_string()
}

/// T1 — the compiler seeds the tag environment in the order declared defaults,
/// `implied_choices`, `context_tags`, `choices`, so each level beats every level
/// below it. This is the compiler's HALF of ADR-0057 §D6: it never consults the
/// solver (ADR-0003 §2), it only honours an inference someone else made and
/// places it correctly in the ladder.
///
/// `implied_choices` deliberately names three facets here, two of which are ALSO
/// bound higher up. A seeding order that appended implied last — the natural
/// mistake — would silently overwrite a user's explicit choice with a value the
/// model merely permits, which is the one thing precedence exists to forbid.
///
/// configflux-v93p: all three entries name a DECLARED facet with an in-domain
/// value, so this is also the accepted arm of the declared-ness + domain screen
/// — the ladder is unchanged for every input the screen lets through. The
/// rejected arms sit at the end of this file.
#[test]
fn implied_choices_sit_above_defaults_and_below_tags_and_choices() {
    let compiled = compile_fixture("facet-precedence", PRECEDENCE_FIXTURE);

    let mut context_tags = BTreeMap::new();
    context_tags.insert("bytag".to_string(), "t2".to_string());
    let state = selection_state(&compiled.handle, "all", context_tags);

    let applied = apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        selection_delta: SelectionDelta {
            facet: "bychoice".to_string(),
            option: "c2".to_string(),
        },
    });
    assert_eq!(
        applied.status,
        OperationStatus::Ok,
        "apply bychoice=c2: {:?}",
        applied.diagnostics
    );
    let state = applied.selection_state.expect("next selection_state");

    let mut implied = BTreeMap::new();
    implied.insert("byimplied".to_string(), "i2".to_string());
    // Both of these are outranked and must lose.
    implied.insert("bytag".to_string(), "t1".to_string());
    implied.insert("bychoice".to_string(), "c1".to_string());

    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        implied_choices: implied,
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "precedence resolve must succeed: {:?}",
        resolved.diagnostics
    );

    assert_eq!(
        probe(&resolved, "p_default"),
        "D1",
        "the declared default applies when nothing above it binds"
    );
    assert_eq!(
        probe(&resolved, "p_implied"),
        "I2",
        "implied beats the declared default"
    );
    assert_eq!(probe(&resolved, "p_tag"), "T2", "a context tag beats implied");
    assert_eq!(
        probe(&resolved, "p_choice"),
        "C2",
        "an explicit choice beats implied"
    );

    let mut expected_defaulted = BTreeMap::new();
    expected_defaulted.insert("bydefault".to_string(), "d1".to_string());
    assert_eq!(
        resolved.defaulted_choices, expected_defaulted,
        "only the facet nothing else bound is recorded as defaulted — an implied \
         facet must NEVER also appear here, or one binding would be counted twice \
         in the resolve-hash pre-image"
    );
}

// ---------------------------------------------------------------------------
// configflux-secb.6 (ADR-0057 §D7) — the requirement block in the lockstep
// ---------------------------------------------------------------------------

/// A single chunk carrying the whole ADR-0057 §D7 shape: a catalogue, a binding
/// with a declared default, and a component that REQUIRES it. The binding is
/// defaulted rather than derived so the compiler-direct resolve path binds it
/// without a solver round, which keeps this test about the HASH rather than
/// about inference.
const REQUIRES_DELIVERY_FIXTURE: &str = r#"{
    "package": "requires_lockstep",
    "version": "1.0.0",
    "catalogues": {
        "containers": {
            "fields": {
                "width_mm": { "type": "integer", "unit": "mm" }
            },
            "entries": {
                "c1": { "width_mm": 800 },
                "c2": { "width_mm": 600 }
            }
        }
    },
    "bindings": {
        "line_container": { "catalogue": "containers", "default": "c1" }
    },
    "components": {
        "svc": {
            "type": "service",
            "requires": { "container": "line_container" }
        }
    }
}"#;

/// ADR-0057 §D7 lockstep, and the ONE case that proves the two `resolve_hash`
/// recipes agree about a delivered requirement.
///
/// The distinction that makes this test worth its lines: the hash handed to
/// `runtime_open` here is the LOADER's, taken off the `ResolveResult` exactly as
/// a real caller would forward it. The runtime then recomputes it with its own
/// independent recipe. A sibling unit test in `runtime_api/tests.rs` stamps its
/// fixture with `expected_resolve_hash`, the runtime's own helper — which is the
/// right tool for building a synthetic snapshot, but makes the two recipes agree
/// by construction and therefore cannot witness a disagreement. Only a
/// loader-produced hash can.
///
/// The tamper arm edits a FIELD VALUE inside the delivered entry and keeps the
/// loader's hash. That is what proves the `requires` block is really inside the
/// runtime's pre-image rather than merely tolerated beside it: if the runtime
/// hashed everything except that block, the edit would sail through.
#[test]
fn runtime_open_cross_validates_a_loader_produced_requires_hash() {
    let compiled = compile_fixture("requires-xval", REQUIRES_DELIVERY_FIXTURE);
    let scope = "component:svc";
    let state = selection_state(&compiled.handle, scope, BTreeMap::new());
    let resolved = resolve(&compiled.handle, scope, &state);

    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "a requirement-bearing resolve must succeed: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved
            .defaulted_choices
            .get("line_container")
            .map(String::as_str),
        Some("c1"),
        "the binding's declared default must be recorded like any facet's"
    );

    let delivered = &resolved
        .resolved_output
        .as_ref()
        .expect("resolved_output present")["svc"]["components"]["svc"]["requires"]["container"];
    assert_eq!(delivered["binding"], serde_json::json!("line_container"));
    assert_eq!(delivered["entry"], serde_json::json!("c1"));
    assert_eq!(delivered["fields"]["width_mm"], serde_json::json!(800));

    // Faithful hand-off: the runtime's own recipe reproduces the LOADER's hash
    // over a payload that carries a requires block.
    let opened = runtime_open(open_request(&resolved, None));
    assert_eq!(
        opened.status,
        OperationStatus::Ok,
        "runtime_open must reproduce the loader's resolve_hash and succeed: {:?}",
        opened.diagnostics
    );

    // Tampered delivered VALUE, loader's hash unchanged → the recomputed hash
    // diverges → fail closed.
    let tampered = runtime_open(open_request_with_output(&resolved, |output| {
        output["svc"]["components"]["svc"]["requires"]["container"]["fields"]["width_mm"] =
            serde_json::json!(801);
    }));
    assert_eq!(
        tampered.status,
        OperationStatus::Error,
        "an edited requirement value must fail closed"
    );
    assert_eq!(
        tampered
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some("E_RUNTIME_HASH_MISMATCH"),
        "the edit must be rejected as a resolve_hash mismatch"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// The `open_request` bridge with the delivered PAYLOAD mutable, so a test can
/// edit `resolved_output` while keeping the loader's `resolve_hash`. Every other
/// bridge forwards the payload verbatim, which is exactly why none of them can
/// reach an edited-payload case.
fn open_request_with_output(
    resolved: &ResolveResult,
    edit: impl FnOnce(&mut serde_json::Value),
) -> RuntimeOpenRequest {
    let mut output = resolved
        .resolved_output
        .clone()
        .expect("resolved_output present");
    edit(&mut output);
    serde_json::from_value(serde_json::json!({
        "schema_version": PRODUCT_SCHEMA_VERSION,
        "model_hash": resolved.model_hash,
        "resolve_hash": resolved.resolve_hash.clone().expect("resolve_hash present"),
        "scope": resolved.scope,
        "resolved_output": output,
        "resolved_component_dependencies": resolved.resolved_component_dependencies,
        "resolved_artifacts": resolved.resolved_artifacts,
        "context_tags": resolved.context_tags,
        "choices": resolved.choices,
        "defaulted_choices": resolved.defaulted_choices,
        "implied_choices": resolved.implied_choices,
    }))
    .expect("valid runtime open request")
}

// ---------------------------------------------------------------------------
// configflux-v93p — the declared-ness + domain screen on `implied_choices`
// ---------------------------------------------------------------------------

/// A catalogue and the binding that draws from it, with NO derive table and no
/// requirement — the minimum shape that DECLARES a binding. ADR-0057 §D3 makes
/// a binding one more declared closed facet whose domain is the catalogue's
/// entry ids, so `container` is a legitimate `implied_choices` key even though
/// it appears under `bindings:` rather than `facets:`.
const BINDING_FIXTURE: &str = r#"{
    "package": "binding_implied",
    "version": "1.0.0",
    "catalogues": {
        "containers": {
            "doc": "The containers this line runs.",
            "fields": {
                "width_mm": { "type": "integer", "unit": "mm", "doc": "Outside width" }
            },
            "entries": {
                "c1": { "width_mm": 800 },
                "c2": { "width_mm": 600 }
            }
        }
    },
    "bindings": {
        "container": {
            "catalogue": "containers",
            "default": "c1",
            "doc": "The container every service on this line draws from."
        }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// Resolve `CLOSED_DEFAULT_FIXTURE` with an empty selection and the supplied
/// `implied_choices`. Every screen case below differs only in that map, so the
/// rest of the request is built once here.
fn resolve_with_implied(label: &str, implied: &[(&str, &str)]) -> (Compiled, ResolveResult) {
    let compiled = compile_fixture(label, CLOSED_DEFAULT_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());
    let implied_choices: BTreeMap<String, String> = implied
        .iter()
        .map(|(facet, option)| ((*facet).to_string(), (*option).to_string()))
        .collect();
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        implied_choices,
    });
    (compiled, resolved)
}

/// V1 — an `implied_choices` entry naming a facet the model never declared is
/// rejected.
///
/// `implied_choices` used to be the ONE resolve input that reached the
/// constraint-evaluation assignment unscreened: `context_tags` and `choices`
/// go through `validate_selection_state`, the declared defaults come from the
/// model itself, and implied arrived with neither a declared-ness check nor a
/// hash binding. It is not a privilege bypass — implied is overlaid by the
/// assignments below it, so it can never outrank a tag or a choice — but a
/// value it injects does land in the `resolve_hash` pre-image, which is reason
/// enough for the surface to fail closed on it.
#[test]
fn resolve_rejects_an_implied_choice_on_an_undeclared_facet() {
    let (compiled, resolved) = resolve_with_implied("implied-unknown", &[("unrelated", "x")]);

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "an undeclared implied facet must fail closed: {:?}",
        resolved.diagnostics
    );
    let diagnostic = resolved
        .diagnostics
        .diagnostics
        .first()
        .expect("a rejection carries a diagnostic");
    assert_eq!(diagnostic.code, "E_SELECTION_UNKNOWN_FACET");
    assert!(
        diagnostic.message.contains("unrelated") && diagnostic.message.contains("implied_choices"),
        "the rejection must name the facet AND the input it came from — three \
         maps reach this surface and a caller cannot fix the right one \
         otherwise; got {:?}",
        diagnostic.message
    );
    assert!(
        resolved.resolved_output.is_none() && resolved.resolve_hash.is_none(),
        "a rejected resolve delivers no payload and no identity for one"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// V2 — the facet IS declared but the implied value is outside its declared
/// domain. Same screen, the second of the two reused codes.
#[test]
fn resolve_rejects_an_implied_choice_outside_the_declared_domain() {
    let (compiled, resolved) = resolve_with_implied("implied-out-of-domain", &[("region", "moon")]);

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "an out-of-domain implied value must fail closed: {:?}",
        resolved.diagnostics
    );
    let diagnostic = resolved
        .diagnostics
        .diagnostics
        .first()
        .expect("a rejection carries a diagnostic");
    assert_eq!(diagnostic.code, "E_SELECTION_INVALID_OPTION");
    assert!(
        diagnostic.message.contains("moon") && diagnostic.message.contains("implied_choices"),
        "the rejection must name the value AND the input it came from; got {:?}",
        diagnostic.message
    );
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some("Valid options: apac, eu, us"),
        "the hint lists the declared domain so the caller can pick from it"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// V3 — the determinism guard. Two offending entries produce two diagnostics,
/// in `implied_choices`' own sorted order, and the run is byte-stable.
///
/// `implied_choices` is a `BTreeMap`, so iterating it IS the sorted order; the
/// test exists because a screen that collected into a `HashMap` first, or that
/// returned on the first offender, would still pass V1 and V2. Reporting one
/// entry at a time would also make a caller with two bad entries discover the
/// second only after fixing the first.
#[test]
fn resolve_reports_every_offending_implied_choice_in_sorted_order() {
    let (compiled, resolved) = resolve_with_implied(
        "implied-two-offenders",
        // Deliberately supplied in an order the sorted walk must correct, and
        // one of each kind so the pairing of code to entry is pinned too.
        &[("region", "moon"), ("alpha_unrelated", "x")],
    );

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "two offending entries must fail closed: {:?}",
        resolved.diagnostics
    );
    let codes: Vec<&str> = resolved
        .diagnostics
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec!["E_SELECTION_UNKNOWN_FACET", "E_SELECTION_INVALID_OPTION"],
        "one diagnostic per offending entry, in sorted facet order \
         (alpha_unrelated before region)"
    );
    assert!(
        resolved.diagnostics.diagnostics[0]
            .message
            .contains("alpha_unrelated")
            && resolved.diagnostics.diagnostics[1].message.contains("region"),
        "each diagnostic names its own entry: {:?}",
        resolved.diagnostics.diagnostics
    );
    assert_eq!(resolved.error_count, 2);

    fs::remove_dir_all(&compiled.output_dir).ok();
}

/// V4 — a BINDING id is an accepted `implied_choices` key.
///
/// This is the case a screen written against `facets:` alone would break, and
/// it would break it in the shipped path rather than in a corner: ADR-0057 §D6
/// has `session_compose::resolve` infer over "each still-unbound declared
/// closed facet (**bindings included**)", drawing its roster from
/// `closed_facet_domains` — which is seeded from the declared facets AND from
/// the bindings projected onto the closed facets they are. So a binding
/// decision is the ordinary output of inference, not an exotic one, and the
/// screen's roster has to be the same union.
///
/// The second arm pins the other half: a value outside the CATALOGUE's entry
/// ids is still rejected, so widening the roster to bindings did not buy them
/// an exemption from the domain check.
#[test]
fn resolve_accepts_an_implied_binding_and_still_screens_its_domain() {
    let compiled = compile_fixture("implied-binding", BINDING_FIXTURE);
    let state = selection_state(&compiled.handle, "all", BTreeMap::new());

    let mut implied = BTreeMap::new();
    implied.insert("container".to_string(), "c2".to_string());
    let resolved = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        implied_choices: implied.clone(),
    });
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "a binding is a declared closed facet — implying one of its catalogue \
         entries must resolve: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.implied_choices, implied,
        "the accepted binding is carried as provenance like any implied facet"
    );

    let mut off_domain = BTreeMap::new();
    off_domain.insert("container".to_string(), "c9".to_string());
    let rejected = resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        implied_choices: off_domain,
    });
    assert_eq!(
        rejected.status,
        OperationStatus::Error,
        "an entry id the catalogue does not hold must still fail closed: {:?}",
        rejected.diagnostics
    );
    let diagnostic = rejected
        .diagnostics
        .diagnostics
        .first()
        .expect("a rejection carries a diagnostic");
    assert_eq!(diagnostic.code, "E_SELECTION_INVALID_OPTION");
    assert_eq!(
        diagnostic.hint.as_deref(),
        Some("Valid options: c1, c2"),
        "a binding's domain is its catalogue's entry ids"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}
