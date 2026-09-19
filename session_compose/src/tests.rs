// SPDX-License-Identifier: BUSL-1.1

//! configflux-secb.3 / ADR-0057 §D6: solver-inferred binding at resolve.
//!
//! The contract these pin: after context tags and explicit choices are applied
//! to the solver session, every still-unbound DECLARED CLOSED facet whose
//! domain has collapsed to a single value is bound to it and recorded in
//! `implied_choices`. Precedence, highest first: explicit choice > context tag
//! > implied > declared default. Declared defaults are applied AFTER inference
//! and only to what inference left open, which is what makes a default mean
//! "the model has no opinion" rather than "the model's opinion, overwritten".
//!
//! Black-box through the crate's own public seam (`resolve`) over a REAL
//! compiled model — the in-process `compile_model` fixture pattern the `cfx`
//! and `runtime` suites use. Nothing reaches into module internals, and the
//! `.ccm` is the one the product compiler emits, so the solver these tests
//! consult is the solver the product consults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use solver::{CuddBackend, Session};

use compiler::loader_api::{
    apply_selection, canonical_selection_state, compute_selection_state_hash, explain_rejection,
    get_selection_options, initialize_selection_state, open_model, resolve_from_selection,
    ApplySelectionRequest, ApplySelectionResult, ExplainRejectionRequest, ExplainRejectionResult,
    GetSelectionOptionsRequest, GetSelectionOptionsResult, InitializeSelectionStateRequest,
    ModelHandle, OpenModelRequest, PrunedOptionReason, RejectionReason,
    ResolveFromSelectionRequest, ResolveResult, SelectionDelta,
    SelectionState, E_RESOLVE_FACET_UNBOUND, E_SELECTION_CONFLICT, E_SELECTION_ENGINE_DIVERGENCE,
    E_SELECTION_INVALID_OPTION, E_SELECTION_STATE_INVALID, E_SELECTION_UNKNOWN_FACET,
    E_SELECTION_UNSATISFIABLE, MODEL_OVER_CONSTRAINED_SUMMARY,
};
use compiler::product_api::{
    compile_model, CompileModelRequest, Diagnostic, DiagnosticSeverity, DiagnosticsReport,
    OperationStatus, SourceManifestEntry, PRODUCT_SCHEMA_VERSION,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The spec's two-facet model (T2/T2b). `site=factory_b` forces `container` to
/// `c2` and nothing else does; with nothing selected, BOTH facets keep two
/// valid values so neither is implied and both fall through to their defaults.
///
/// This is the assessment's failing case in miniature: before ADR-0057 §D6,
/// selecting only `site=factory_b` defaulted `container` to `c1`, violated
/// `factory_b_uses_c2`, and exited with `E_SELECTION_CONFLICT` — while
/// `cfx options` already reported `c2` as the only valid value.
const SITE_CONTAINER_FIXTURE: &str = r#"{
    "package": "secb3_site_container",
    "version": "1.0.0",
    "facets": {
        "site":      { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "container": { "values": ["c1", "c2"], "default": "c1" }
    },
    "constraints": {
        "factory_b_uses_c2": {
            "condition": "site != 'factory_b' || container == 'c2'",
            "doc": "Factory B only has c2 containers on the line."
        }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// A two-constraint chain (T3): choosing `stage=s2` forces `mid=m2`, which in
/// turn forces `tail=t2`. One resolve must bind BOTH, which is what makes this
/// a fixpoint rather than a single pass over the roster.
const CHAIN_FIXTURE: &str = r#"{
    "package": "secb3_chain",
    "version": "1.0.0",
    "facets": {
        "stage": { "values": ["s1", "s2"], "default": "s1" },
        "mid":   { "values": ["m1", "m2"], "default": "m1" },
        "tail":  { "values": ["t1", "t2"], "default": "t1" }
    },
    "constraints": {
        "stage_forces_mid": { "condition": "stage != 's2' || mid == 'm2'" },
        "mid_forces_tail":  { "condition": "mid != 'm2' || tail == 't2'" }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// Zero-remaining (T4): under `site=factory_b` the first constraint forces
/// `container == 'c2'` and the second forbids it, so the formula is already
/// unsatisfiable and NO facet has a valid option left. The model itself is
/// satisfiable (`site=factory_a, container=c1`), so it compiles and links.
const CONTRADICTION_FIXTURE: &str = r#"{
    "package": "secb3_contradiction",
    "version": "1.0.0",
    "facets": {
        "site":      { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "container": { "values": ["c1", "c2"], "default": "c1" }
    },
    "constraints": {
        "factory_b_uses_c2": { "condition": "site != 'factory_b' || container == 'c2'" },
        "c2_withdrawn":      { "condition": "container != 'c2'" }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// Open and undeclared facets (T5). `mode` is declared OPEN and the constraint
/// forces it to `m2` under `site=factory_b`; it must STILL not be inferred,
/// because an open domain is extensible and nothing may ever be entailed about
/// it. `tier` is never declared at all — it exists only because a component
/// condition mentions it — so it is outside the roster for a different reason.
const OPEN_AND_UNDECLARED_FIXTURE: &str = r#"{
    "package": "secb3_open_undeclared",
    "version": "1.0.0",
    "facets": {
        "site": { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "mode": { "values": ["m1", "m2"], "default": "m2", "open": true }
    },
    "constraints": {
        "factory_b_uses_m2": { "condition": "site != 'factory_b' || mode == 'm2'" }
    },
    "components": {
        "svc": { "type": "service", "condition": "tier == 'gold'" }
    }
}"#;

/// A one-value closed domain (T5b). ADR-0054 §5.2 lowers it to the bare root
/// assertion `only == 'v1'` rather than an `exactly_one_of`, so the solver
/// holds exactly one valid option for it from the start — it is IMPLIED, not
/// defaulted, and it has no default to fall back on in any case. `other` keeps
/// two values and is untouched by any rule, so it defaults.
const SINGLE_VALUE_FIXTURE: &str = r#"{
    "package": "secb3_single_value",
    "version": "1.0.0",
    "facets": {
        "only":  { "values": ["v1"] },
        "other": { "values": ["o1", "o2"], "default": "o1" }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

static FIXTURE_SEQ: AtomicU64 = AtomicU64::new(0);

struct Compiled {
    handle: ModelHandle,
    _dir: PathBuf,
}

/// Compile one inline chunk into a real CMP + `.ccm` and open it.
///
/// The `.ccm` this emits is the one `//compiler:compiler` itself emits — the
/// binary is built against the same cudd-free `compiler_lib` this crate depends
/// on — so the session the tests below drive is the product's session.
fn compile_fixture(label: &str, source: &str) -> Compiled {
    let seq = FIXTURE_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "session-compose-{label}-{}-{seq}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create fixture dir");

    let compiled = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: vec![SourceManifestEntry {
            source_id: "scenarios/secb3/00_definitions.json".to_string(),
            inline_content: source.to_string(),
        }],
        output_dir: Some(dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        compiled.status,
        OperationStatus::Ok,
        "fixture compile must succeed: {:?}",
        compiled.verify_report.diagnostics.diagnostics
    );
    let cmp_manifest_ref = compiled
        .compiled_model_package_ref
        .expect("compile emitted no compiled_model_package_ref");

    let opened = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref,
    });
    assert_eq!(
        opened.status,
        OperationStatus::Ok,
        "open_model must succeed: {:?}",
        opened.diagnostics
    );
    Compiled {
        handle: opened.model_handle.expect("model handle present"),
        _dir: dir,
    }
}

/// Build a canonical `SelectionState` carrying `context_tags`, then apply each
/// entry of `choices` through the compiler's own `apply_selection` so the
/// state's hash stays canonical.
fn selection(
    handle: &ModelHandle,
    context_tags: &[(&str, &str)],
    choices: &[(&str, &str)],
) -> SelectionState {
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        context_tags: map(context_tags),
    });
    assert_eq!(
        init.status,
        OperationStatus::Ok,
        "initialize_selection_state must succeed: {:?}",
        init.diagnostics
    );
    let mut state = init.selection_state.expect("selection_state present");

    for (facet, option) in choices {
        let applied = apply_selection(ApplySelectionRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            model_handle: handle.clone(),
            scope: "all".to_string(),
            selection_state: state,
            selection_delta: SelectionDelta {
                facet: (*facet).to_string(),
                option: (*option).to_string(),
            },
        });
        assert_eq!(
            applied.status,
            OperationStatus::Ok,
            "apply_selection {facet}={option} must succeed: {:?}",
            applied.diagnostics
        );
        state = applied.selection_state.expect("next selection_state present");
    }
    state
}

fn request(handle: &ModelHandle, state: &SelectionState) -> ResolveFromSelectionRequest {
    ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        implied_choices: BTreeMap::new(),
    }
}

/// `session_compose::resolve` over the fixture, asserting it succeeded.
fn resolve_ok(handle: &ModelHandle, state: &SelectionState) -> ResolveResult {
    let resolved = super::resolve(request(handle, state));
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "resolve must succeed: {:?}",
        resolved.diagnostics
    );
    resolved
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// The still-valid options `session_compose::options` reports for one facet —
/// the seam `cfx options` and the interpreter's `options` envelope both call.
fn options_for(handle: &ModelHandle, state: &SelectionState, facet: &str) -> Vec<String> {
    let listed = super::options(GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        facet: facet.to_string(),
        include_pruned_reasons: false,
    });
    assert_eq!(
        listed.status,
        OperationStatus::Ok,
        "options must succeed: {:?}",
        listed.diagnostics
    );
    listed.valid_options
}

/// `session_compose::explain` for one `(facet, option)` — the seam `cfx explain`
/// calls. A rejection is the SUCCESS path here (ADR-0031 D2), so the caller
/// reads `rejection`, not `status`.
fn explain_for(
    handle: &ModelHandle,
    state: &SelectionState,
    facet: &str,
    option: &str,
) -> ExplainRejectionResult {
    super::explain(ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        rejected_option: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    })
}

/// An `apply_selection` request for one `(facet, option)` delta over `state`.
fn apply_request(
    handle: &ModelHandle,
    state: &SelectionState,
    facet: &str,
    option: &str,
) -> ApplySelectionRequest {
    apply_request_scoped(handle, state, "all", facet, option)
}

/// The same request under an explicit `scope`. Only the two scope bindings
/// (`scope` non-empty, and `selection_state.scope == request.scope`) can be
/// stated with the request's scope held apart from the state's, so those cases
/// need a seam `apply_request`'s hardcoded `"all"` cannot give them.
fn apply_request_scoped(
    handle: &ModelHandle,
    state: &SelectionState,
    scope: &str,
    facet: &str,
    option: &str,
) -> ApplySelectionRequest {
    ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: scope.to_string(),
        selection_state: state.clone(),
        selection_delta: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    }
}

/// A bare `Session<CuddBackend>` over the fixture's real `.ccm` — the same
/// artifact `session_from_handle` loads, with no environment replayed onto it.
fn fixture_session(handle: &ModelHandle) -> Session<CuddBackend> {
    let ccm = Session::<CuddBackend>::load_ccm(Path::new(handle.ccm_ref.trim()))
        .expect("fixture .ccm loads");
    Session::<CuddBackend>::new(ccm).expect("session over the fixture .ccm")
}

/// The declared-constraint ids an explain result blames.
fn blamed_constraints(explained: &ExplainRejectionResult) -> Vec<String> {
    explained
        .rejection
        .unsat_core
        .as_ref()
        .map(|core| {
            core.conflicting_constraints
                .iter()
                .filter_map(|clause| clause.constraint_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// T2 / T2b — the precedence pin
// ---------------------------------------------------------------------------

/// T2: an explicit choice that leaves exactly one valid value for another
/// declared closed facet binds it, and records it as IMPLIED rather than
/// defaulted. `container` has a declared default of `c1`; the constraints say
/// `c2`; the constraints win, which is the whole point of ADR-0057 §D6.
#[test]
fn a_forced_facet_is_inferred_and_recorded_as_implied() {
    let compiled = compile_fixture("site-container", SITE_CONTAINER_FIXTURE);
    let state = selection(&compiled.handle, &[], &[("site", "factory_b")]);

    let resolved = resolve_ok(&compiled.handle, &state);

    assert_eq!(
        resolved.implied_choices,
        map(&[("container", "c2")]),
        "container's domain collapses to c2 under site=factory_b, so it must be implied"
    );
    assert!(
        resolved.defaulted_choices.is_empty(),
        "an implied facet must NOT also be recorded as defaulted, and site came \
         from an explicit choice: {:?}",
        resolved.defaulted_choices
    );
}

/// T2b — the PINNED SEMANTICS the spec calls out by name. With nothing
/// selected, `site` still has two valid values, so it is not implied; it takes
/// its default. Crucially `container` is NOT then implied from that default:
/// defaults are applied AFTER inference, so they never feed it. `container`
/// takes its own default `c1`.
///
/// The alternative — seeding defaults into the inference session — would make a
/// default silently propagate through the constraint graph, so a value nobody
/// chose could force a value nobody chose. That is exactly the coupling the
/// precedence order exists to prevent.
#[test]
fn defaults_are_applied_after_inference_and_never_feed_it() {
    let compiled = compile_fixture("site-container-empty", SITE_CONTAINER_FIXTURE);
    let state = selection(&compiled.handle, &[], &[]);

    let resolved = resolve_ok(&compiled.handle, &state);

    assert!(
        resolved.implied_choices.is_empty(),
        "nothing is forced when nothing is selected: {:?}",
        resolved.implied_choices
    );
    assert_eq!(
        resolved.defaulted_choices,
        map(&[("container", "c1"), ("site", "factory_a")]),
        "both facets fall through to their declared defaults"
    );
}

// ---------------------------------------------------------------------------
// T3 — fixpoint
// ---------------------------------------------------------------------------

/// T3: one choice propagates through TWO constraints, and a single resolve
/// binds both downstream facets. A single non-iterating pass would bind `mid`
/// and stop.
#[test]
fn inference_reaches_a_fixpoint_across_chained_constraints() {
    let compiled = compile_fixture("chain", CHAIN_FIXTURE);
    let state = selection(&compiled.handle, &[], &[("stage", "s2")]);

    let resolved = resolve_ok(&compiled.handle, &state);

    assert_eq!(
        resolved.implied_choices,
        map(&[("mid", "m2"), ("tail", "t2")]),
        "stage=s2 forces mid=m2, which forces tail=t2; one resolve binds both"
    );
    assert!(
        resolved.defaulted_choices.is_empty(),
        "every facet is either chosen or implied: {:?}",
        resolved.defaulted_choices
    );
}

// ---------------------------------------------------------------------------
// T4 — zero remaining
// ---------------------------------------------------------------------------

/// T4: when the selection leaves a declared closed facet with NO valid value,
/// inference implies nothing and the resolve fails with the EXISTING
/// `E_SELECTION_CONFLICT` — byte-identically to what the compiler produces
/// today for the same input.
///
/// The expected bytes are computed by calling `resolve_from_selection` directly
/// rather than hard-coded, so this is an oracle rather than a transcription: it
/// cannot go stale, and it states the real contract, which is "inference must
/// not change this rejection" rather than "this rejection has this wording".
///
/// It also pins the choice between aborting inference wholesale and skipping
/// just the exhausted facet. Those are observationally identical here and in
/// general: **ADR-0054 §5.2** gives every declared CLOSED facet an
/// `exactly_one_of` conjunct — at-least-one plus pairwise at-most-one — so a
/// satisfiable formula always leaves such a facet at least one option, and zero
/// options anywhere implies the formula is already unsatisfiable and EVERY
/// facet returns zero. Either policy therefore records nothing, which is what
/// the empty `implied_choices` below asserts.
///
/// Cite §5.2 and NOT ADR-0047 §4 for that premise: the earlier synthesis was
/// REMOVED by ADR-0047 Amendment 1 and reinstated by §5.2/§5.3, so anyone
/// re-deriving it from ADR-0047 reaches the opposite conclusion. Aborting is
/// also strictly SAFER than skipping, not merely equivalent — see
/// `infer_forced_bindings` for the two holes where zero options does not imply
/// unsatisfiability.
#[test]
fn a_selection_with_no_remaining_option_fails_exactly_as_it_does_today() {
    let compiled = compile_fixture("contradiction", CONTRADICTION_FIXTURE);
    let state = selection(&compiled.handle, &[("site", "factory_b")], &[]);

    let expected = resolve_from_selection(request(&compiled.handle, &state));
    let resolved = super::resolve(request(&compiled.handle, &state));

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "an unsatisfiable selection must still be rejected"
    );
    assert_eq!(
        resolved.diagnostics.diagnostics.first().map(|d| d.code.as_str()),
        Some(E_SELECTION_CONFLICT),
        "the rejection keeps the existing code: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.diagnostics, expected.diagnostics,
        "inference must not perturb the rejection the compiler already produces"
    );
    assert!(
        resolved.implied_choices.is_empty(),
        "nothing may be implied from an already-unsatisfiable assignment: {:?}",
        resolved.implied_choices
    );
}

// ---------------------------------------------------------------------------
// T5 / T5b — the roster boundary
// ---------------------------------------------------------------------------

/// T5: an OPEN facet is never inferred even when the constraints force it, and
/// an UNDECLARED facet is never inferred because it was never declared. The two
/// are outside the roster for different reasons and both must stay outside it.
///
/// `mode` is forced to `m2` here and still arrives through its DEFAULT, which
/// is the observable difference: the provenance bucket, and therefore the
/// resolve-hash pre-image, must record it as defaulted rather than implied.
#[test]
fn open_and_undeclared_facets_are_never_inferred() {
    let compiled = compile_fixture("open-undeclared", OPEN_AND_UNDECLARED_FIXTURE);
    let state = selection(&compiled.handle, &[("tier", "gold")], &[("site", "factory_b")]);

    let resolved = resolve_ok(&compiled.handle, &state);

    assert!(
        resolved.implied_choices.is_empty(),
        "an open facet is extensible so nothing may be entailed about it, and an \
         undeclared facet has no declaration to entail from: {:?}",
        resolved.implied_choices
    );
    assert_eq!(
        resolved.defaulted_choices,
        map(&[("mode", "m2")]),
        "the forced-but-open facet arrives through its default instead"
    );
}

/// T5b: a one-value closed domain is IMPLIED, not defaulted. ADR-0054 §5.2
/// lowers it to the bare root assertion `only == 'v1'`, so the solver has held
/// exactly one valid option for it from the start — the model has an opinion,
/// which is precisely the line between implied and defaulted.
#[test]
fn a_single_value_closed_facet_is_implied_not_defaulted() {
    let compiled = compile_fixture("single-value", SINGLE_VALUE_FIXTURE);
    let state = selection(&compiled.handle, &[], &[]);

    let resolved = resolve_ok(&compiled.handle, &state);

    assert_eq!(
        resolved.implied_choices,
        map(&[("only", "v1")]),
        "a one-value closed domain leaves exactly one valid option, so it is implied"
    );
    assert_eq!(
        resolved.defaulted_choices,
        map(&[("other", "o1")]),
        "a facet the model says nothing about still takes its declared default"
    );
}

// ---------------------------------------------------------------------------
// configflux-bmjt — one environment, three surfaces
// ---------------------------------------------------------------------------

/// A context tag is part of the deployment environment, so every surface must
/// reason over it. Under `site=factory_b` the model has already decided
/// `container`: `options` must offer only `c2`, `explain` must refuse `c1` and
/// blame the tag that refuses it, and `resolve` must bind the same `c2`.
///
/// Before configflux-bmjt only `resolve` saw the tag. `options` listed both
/// values and `explain` called `c1` currently valid — the surface disagreement
/// ADR-0054 was written to eliminate, reached through the one channel ADR-0057
/// §D6 ranks directly below an explicit choice.
#[test]
fn options_explain_and_resolve_agree_on_a_tag_bound_facet() {
    let compiled = compile_fixture("tag-bound", SITE_CONTAINER_FIXTURE);
    let state = selection(&compiled.handle, &[("site", "factory_b")], &[]);

    assert_eq!(
        options_for(&compiled.handle, &state, "container"),
        vec!["c2".to_string()],
        "the tag has already excluded c1, so options must not offer it"
    );

    let explained = explain_for(&compiled.handle, &state, "container", "c1");
    assert_eq!(
        explained.rejection.code, E_SELECTION_UNSATISFIABLE,
        "the value the tag excludes must be explained as a rejection: {:?}",
        explained.rejection
    );
    assert_eq!(
        explained.rejection.blocking_choices,
        map(&[("site", "factory_b")]),
        "the rejection must be attributed to the context tag that caused it"
    );
    assert!(
        blamed_constraints(&explained).contains(&"factory_b_uses_c2".to_string()),
        "the core must name the authored rule the tag activates: {:?}",
        explained.rejection.unsat_core
    );

    let resolved = resolve_ok(&compiled.handle, &state);
    assert_eq!(
        resolved.implied_choices,
        map(&[("container", "c2")]),
        "resolve must bind the one value the other two surfaces left standing"
    );
}

/// The same environment, delivered through the other channel, must produce the
/// same answers. `site=factory_b` as an immutable context tag and
/// `site=factory_b` as an explicit choice are one deployment environment
/// (ADR-0057 §D6 ranks the two, it does not make one invisible), so the three
/// surfaces may not tell them apart.
///
/// This is the case that catches a partial fix: teaching `options` about tags
/// while leaving `explain` on choices only still passes the assertions above.
#[test]
fn a_tag_and_a_choice_carrying_one_environment_answer_alike() {
    let compiled = compile_fixture("tag-vs-choice", SITE_CONTAINER_FIXTURE);
    let tagged = selection(&compiled.handle, &[("site", "factory_b")], &[]);
    let chosen = selection(&compiled.handle, &[], &[("site", "factory_b")]);

    assert_eq!(
        options_for(&compiled.handle, &tagged, "container"),
        options_for(&compiled.handle, &chosen, "container"),
        "options must not depend on which channel carried the environment"
    );

    let by_tag = explain_for(&compiled.handle, &tagged, "container", "c1");
    let by_choice = explain_for(&compiled.handle, &chosen, "container", "c1");
    assert_eq!(
        by_tag.rejection.code, by_choice.rejection.code,
        "explain must reach the same verdict either way"
    );
    assert_eq!(
        blamed_constraints(&by_tag),
        blamed_constraints(&by_choice),
        "explain must blame the same authored rule either way"
    );

    assert_eq!(
        resolve_ok(&compiled.handle, &tagged).implied_choices,
        resolve_ok(&compiled.handle, &chosen).implied_choices,
        "resolve must bind the same value either way"
    );
}

/// `select` joins the agreement too. Under `site=factory_b` the compiler's own
/// `apply_selection` already refuses `container=c1` by name
/// (`E_SELECTION_CONFLICT` quoting `factory_b_uses_c2`) because it evaluates
/// the merged tags-and-choices assignment. The solver session did not, so the
/// wrapper built its own OK envelope over the compiler's rejection and `select`
/// accepted a choice `resolve` then refused.
///
/// The oracle is `apply_selection` itself rather than transcribed bytes, so
/// this states the contract — the two engines answer alike — instead of a
/// wording that can go stale.
#[test]
fn select_refuses_what_a_context_tag_excludes_with_the_compilers_bytes() {
    let compiled = compile_fixture("tag-select", SITE_CONTAINER_FIXTURE);
    let state = selection(&compiled.handle, &[("site", "factory_b")], &[]);
    let delta = apply_request(&compiled.handle, &state, "container", "c1");

    let expected = apply_selection(delta.clone());
    let applied = super::apply(delta);

    assert_eq!(
        expected.status,
        OperationStatus::Error,
        "premise: the compiler already refuses this choice: {:?}",
        expected.diagnostics
    );
    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "select must not accept a choice a context tag excludes"
    );
    assert_eq!(
        applied.diagnostics, expected.diagnostics,
        "the refusal must be the compiler's canonical bytes, not a wrapper-invented one"
    );
}

/// ADR-0057 §D6, in the one place the ORDER of the replay is observable.
/// Applying an environment to a session is a conjunction and conjunction
/// commutes, so the order can only matter when one assignment CONTRADICTS
/// another — and then the one applied FIRST is the one that survives, because
/// the second is refused and skipped.
///
/// The immutable tag `site=factory_b` and the explicit choice `container=c1`
/// cannot both hold under `factory_b_uses_c2`, and they name DIFFERENT facets,
/// so nothing rejects the pair before it reaches the session: the same-facet
/// guard does not fire and the state validates. Tags first means the CHOICE is
/// the assignment refused. Choices first would refuse the TAG, leaving the
/// session on `c1` and on a site nobody named.
///
/// **What this pins changed with ADR-0030 Amendment 2, and the witness did
/// not.** `options` used to answer over the weakened session, so the order was
/// visible in the option lists — `site: [factory_b]`, `container: [c2]`. It no
/// longer answers at all: the pair is unsatisfiable, and Rule 3 forbids any
/// surface from enumerating over a state the solver refuses. The order is
/// visible in the same place it always mattered, one step earlier: the conflict
/// names the FIRST refused assignment, and which one that is *is* the order.
///
/// Reversing the chain in `apply_environment` turns the naming assertion below
/// red, which is what keeps this a pin on the order rather than on the fact
/// that tags are applied at all.
#[test]
fn a_context_tag_binds_before_a_choice_that_contradicts_it() {
    let compiled = compile_fixture("tag-precedence", SITE_CONTAINER_FIXTURE);
    let state = canonical_selection_state(
        compiled.handle.model_hash.clone(),
        "all",
        map(&[("site", "factory_b")]),
        map(&[("container", "c1")]),
    )
    .expect("canonical selection state");

    for facet in ["site", "container"] {
        let listed = super::options(options_request(&compiled.handle, &state, facet));
        assert_eq!(
            listed.status,
            OperationStatus::Error,
            "options over an unsatisfiable pair must refuse, not enumerate"
        );
        assert_eq!(options_code(&listed), E_SELECTION_CONFLICT);
        assert!(
            listed.valid_options.is_empty(),
            "an error envelope carries no payload: {:?}",
            listed.valid_options
        );
        assert!(
            listed.diagnostics.diagnostics[0]
                .message
                .contains("'container' = 'c1'"),
            "the tag binds FIRST, so the CHOICE is what gets refused; naming \
             'site' here would mean the chain had been reversed: {:?}",
            listed.diagnostics
        );
    }
}

/// The three replay verdicts, and which one withholds the session. Only a
/// solver FAULT does: `apply_environment` stops at a Backend/Invariant/Ccm
/// error, so every assignment after it is missing and the session is strictly
/// LESS constrained than the deployment asked about — answering `options`,
/// `select` or `explain` over it would offer values the deployment excludes,
/// which is failing open (ADR-0030 D4).
///
/// A typed rejection must NOT withhold it: re-deriving a previously-accepted
/// state has to keep working, and the compiler owns the canonical rejection for
/// a state that is invalid as a whole. Both halves are asserted, because a fix
/// that failed closed on `Rejected` too would break every surface under an
/// unmodelled tag value while looking safer.
///
/// A real Backend/Invariant fault needs a malformed artifact no fixture can
/// produce, so the verdict is supplied directly and the SESSION is the real one
/// the product loads.
#[test]
fn only_a_solver_fault_withholds_the_session() {
    let compiled = compile_fixture("replay-verdict", SITE_CONTAINER_FIXTURE);

    assert!(
        matches!(
            super::model_from_replay(fixture_session(&compiled.handle), super::EnvReplay::Accepted),
            super::SolverModel::Usable {
                replay: super::EnvReplay::Accepted,
                ..
            }
        ),
        "an accepted environment yields the session"
    );
    assert!(
        matches!(
            super::model_from_replay(
                fixture_session(&compiled.handle),
                // The refused assignment the verdict now carries
                // (configflux-rzyd) is irrelevant HERE — this asserts which
                // verdicts withhold the session, and only the variant decides
                // that. It is named so the case reads as the same refusal the
                // fixture's own constraint would produce.
                super::EnvReplay::Rejected {
                    facet: "container".to_string(),
                    option: "c1".to_string(),
                },
            ),
            super::SolverModel::Usable {
                replay: super::EnvReplay::Rejected { .. },
                ..
            }
        ),
        "a refused environment still yields the session — and the verdict rides \
         with it, so the caller can refuse the state rather than answer over it \
         (ADR-0030 Amendment 2 Rule 3)"
    );
    assert!(
        matches!(
            super::model_from_replay(
                fixture_session(&compiled.handle),
                super::EnvReplay::Skew {
                    facet: "container".to_string(),
                    option: "c9".to_string(),
                },
            ),
            super::SolverModel::Skew { .. }
        ),
        "an artifact that cannot hold a declared value describes some other \
         model; it must fail closed"
    );
    assert!(
        matches!(
            super::model_from_replay(fixture_session(&compiled.handle), super::EnvReplay::Fault),
            super::SolverModel::Unavailable
        ),
        "a faulted replay leaves a partially-applied session; it must fail closed"
    );
}

// ---------------------------------------------------------------------------
// configflux-kbue — the pre-existing contradiction rejections, unmoved
// ---------------------------------------------------------------------------

/// A choice contradicting a context tag on the SAME facet is refused, and
/// applying the environment to a session must not change how. The state is
/// built through `canonical_selection_state` because the canonical builders
/// refuse to produce it: `apply_selection` rejects the choice first.
///
/// The expected bytes come from `resolve_from_selection`, so this is an oracle
/// rather than a transcription of today's wording.
#[test]
fn a_choice_contradicting_its_context_tag_is_rejected_exactly_as_it_is_today() {
    let compiled = compile_fixture("kbue-resolve", SITE_CONTAINER_FIXTURE);
    let state = canonical_selection_state(
        compiled.handle.model_hash.clone(),
        "all",
        map(&[("site", "factory_a")]),
        map(&[("site", "factory_b")]),
    )
    .expect("canonical selection state");

    let expected = resolve_from_selection(request(&compiled.handle, &state));
    let resolved = super::resolve(request(&compiled.handle, &state));

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "a state whose tag and choice contradict each other must stay rejected"
    );
    assert_eq!(
        resolved
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some(E_SELECTION_STATE_INVALID),
        "the rejection keeps the existing code: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.diagnostics, expected.diagnostics,
        "applying the environment must not perturb the rejection the compiler produces"
    );
}

/// Selecting against an immutable context tag keeps its own rejection. The
/// facet is tag-pinned, so `apply` delegates before it ever builds a session
/// (ADR-0030 D5) — this pins that the delegation still happens and still
/// carries the compiler's `E_SELECTION_CONFLICT` bytes.
#[test]
fn selecting_against_an_immutable_context_tag_keeps_its_bytes() {
    let compiled = compile_fixture("kbue-select", SITE_CONTAINER_FIXTURE);
    let state = selection(&compiled.handle, &[("site", "factory_b")], &[]);
    let delta = apply_request(&compiled.handle, &state, "site", "factory_a");

    let expected = apply_selection(delta.clone());
    let applied = super::apply(delta);

    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "a tag-pinned facet may not be re-bound by a choice"
    );
    assert_eq!(
        applied
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some(E_SELECTION_CONFLICT),
        "the rejection keeps the existing code: {:?}",
        applied.diagnostics
    );
    assert_eq!(
        applied.diagnostics, expected.diagnostics,
        "the compiler's canonical immutable-context-tag bytes are unchanged"
    );
}

// ---------------------------------------------------------------------------
// configflux-q50t — a selection state the compiler calls invalid is refused on
// the solver-ACCEPT path too.
//
// The whole `SelectionState` is caller-supplied at the SDK seam: `interpreter
// select` deserializes one straight from stdin. The compiler binds six
// integrity properties to it before it will adjudicate anything; the wrapper
// used to check only `request.schema_version` and whether the DELTA facet was
// already assigned, so a state failing any of the six reached the solver, was
// accepted, and came back as an Ok envelope carrying a freshly canonicalized
// hash for a state the engine refuses. That is an engine refusal overridden by
// a wrapper-composed OK — the class configflux-bmjt closed for the
// tag-versus-constraint case, here for state integrity.
//
// Each case below names ONE binding and holds the other five valid, so a
// regression points at the property it broke rather than at "something in the
// state". The oracle is `apply_selection` itself, and the assertion is on the
// WHOLE envelope rather than transcribed bytes: the contract is "select IS the
// compiler's answer here", which no wording can go stale against.
//
// Every delta is one the solver ACCEPTS (`container=c2` is what `site=
// factory_b` forces). That is load-bearing. A delta the solver rejects would
// reach the compiler through the pre-existing typed-rejection delegation and
// the case would pass without ever exercising the accept path it exists for.
// ---------------------------------------------------------------------------

/// A state mutated after the fact and RE-SEALED with the canonical hash of its
/// new contents.
///
/// Sealing is what isolates the binding. `compute_selection_state_hash` covers
/// `schema_version`, `model_hash` and `scope`, so mutating any of them and
/// leaving the old hash in place would trip the HASH check — which the
/// validator reaches first — and the case would go green for the wrong reason.
fn resealed(
    mut state: SelectionState,
    mutate: impl FnOnce(&mut SelectionState),
) -> SelectionState {
    mutate(&mut state);
    state.selection_state_hash =
        compute_selection_state_hash(&state).expect("canonical hash over the mutated state");
    state
}

/// The contract every case below states, in the two halves that make it one:
/// the compiler must ALREADY refuse this state as invalid (the premise, so a
/// case cannot pass by refusing for some unrelated reason), and
/// `session_compose::apply` must return that refusal envelope entire.
fn assert_select_is_the_compilers_refusal(request: ApplySelectionRequest, binding: &str) {
    let expected = apply_selection(request.clone());
    assert_eq!(
        expected.status,
        OperationStatus::Error,
        "premise ({binding}): the compiler must already refuse this state"
    );
    assert_eq!(
        expected
            .diagnostics
            .diagnostics
            .first()
            .map(|d| d.code.as_str()),
        Some(E_SELECTION_STATE_INVALID),
        "premise ({binding}): the refusal must be the state-integrity code: {:?}",
        expected.diagnostics
    );

    let applied = super::apply(request);
    assert_eq!(
        applied, expected,
        "select must return the compiler's whole envelope for {binding}, not an \
         Ok it composed itself"
    );
}

/// The headline case: one facet carrying DIFFERENT values in `context_tags` and
/// `choices`, with the delta on an unrelated facet.
///
/// Unrelated is the point. The delta facet is in neither map, so the wrapper's
/// already-assigned guards do not fire, and the environment replay is no help
/// either: it applies `site=factory_a`, typed-rejects `site=factory_b` — which
/// correctly still yields the session — and then accepts `container=c1` over a
/// deployment the caller never coherently described.
#[test]
fn select_refuses_a_state_whose_tag_and_choice_disagree_about_one_facet() {
    let compiled = compile_fixture("q50t-merge", SITE_CONTAINER_FIXTURE);
    let state = canonical_selection_state(
        compiled.handle.model_hash.clone(),
        "all",
        map(&[("site", "factory_a")]),
        map(&[("site", "factory_b")]),
    )
    .expect("canonical state over the contradictory maps");

    assert_select_is_the_compilers_refusal(
        apply_request(&compiled.handle, &state, "container", "c1"),
        "a facet carrying different values in context_tags and choices",
    );
}

/// A hash that is not the canonical hash of the state it seals. This is the
/// binding that makes a `SelectionState` unforgeable in transit; without it the
/// other five are only as trustworthy as whoever wrote the JSON.
#[test]
fn select_refuses_a_forged_selection_state_hash() {
    let compiled = compile_fixture("q50t-hash", SITE_CONTAINER_FIXTURE);
    let mut state = selection(&compiled.handle, &[("site", "factory_b")], &[]);
    state.selection_state_hash = "0".repeat(64);

    assert_select_is_the_compilers_refusal(
        apply_request(&compiled.handle, &state, "container", "c2"),
        "a forged selection_state_hash",
    );
}

/// A state sealed against a DIFFERENT model than the handle names. Nothing else
/// catches it: the session is built from `model_handle.ccm_ref`, so the wrapper
/// never reads `selection_state.model_hash` at all.
#[test]
fn select_refuses_a_state_sealed_against_another_model() {
    let compiled = compile_fixture("q50t-model", SITE_CONTAINER_FIXTURE);
    let state = resealed(
        selection(&compiled.handle, &[("site", "factory_b")], &[]),
        |state| state.model_hash = "sha256:0000000000000000".to_string(),
    );

    assert_select_is_the_compilers_refusal(
        apply_request(&compiled.handle, &state, "container", "c2"),
        "a selection_state.model_hash bound to another model",
    );
}

/// A state whose own scope disagrees with the scope the request asks about —
/// selections carried over from one deployment scope into another.
#[test]
fn select_refuses_a_state_scoped_to_another_scope() {
    let compiled = compile_fixture("q50t-scope", SITE_CONTAINER_FIXTURE);
    let state = resealed(
        selection(&compiled.handle, &[("site", "factory_b")], &[]),
        |state| state.scope = "staging".to_string(),
    );

    assert_select_is_the_compilers_refusal(
        apply_request_scoped(&compiled.handle, &state, "all", "container", "c2"),
        "a selection_state.scope that is not the request scope",
    );
}

/// An empty scope. `initialize_selection_state` refuses to mint one, so the
/// state is built through `canonical_selection_state` directly — which is
/// exactly the reach a caller writing the JSON by hand has.
#[test]
fn select_refuses_an_empty_scope() {
    let compiled = compile_fixture("q50t-empty-scope", SITE_CONTAINER_FIXTURE);
    let state = canonical_selection_state(
        compiled.handle.model_hash.clone(),
        "",
        map(&[("site", "factory_b")]),
        BTreeMap::new(),
    )
    .expect("canonical state over the empty scope");

    assert_select_is_the_compilers_refusal(
        apply_request_scoped(&compiled.handle, &state, "", "container", "c2"),
        "an empty scope",
    );
}

/// The state's OWN `schema_version`, which is a different field from the
/// request's and can be set independently — the request's is the one the
/// wrapper already checked, and checking it proved nothing about this one.
#[test]
fn select_refuses_a_state_declaring_an_unsupported_schema_version() {
    let compiled = compile_fixture("q50t-schema", SITE_CONTAINER_FIXTURE);
    let state = resealed(
        selection(&compiled.handle, &[("site", "factory_b")], &[]),
        |state| state.schema_version = PRODUCT_SCHEMA_VERSION - 1,
    );

    let request = apply_request(&compiled.handle, &state, "container", "c2");
    assert_eq!(
        request.schema_version,
        PRODUCT_SCHEMA_VERSION,
        "premise: the REQUEST's schema_version is the supported one, so only the \
         state's own version is under test"
    );
    assert_select_is_the_compilers_refusal(
        request,
        "a selection_state.schema_version the product does not support",
    );
}

/// The boundary the screen must not cross: a VALID state still takes the
/// solver-accept path and still comes back with the wrapper's own canonical
/// next state. A screen that delegated everything would look safer and would
/// hand `select` back to the engine ADR-0030 took it from.
#[test]
fn a_valid_state_still_takes_the_solver_accept_path() {
    let compiled = compile_fixture("q50t-accept", SITE_CONTAINER_FIXTURE);
    let state = selection(&compiled.handle, &[("site", "factory_b")], &[]);

    let applied = super::apply(apply_request(&compiled.handle, &state, "container", "c2"));

    assert_eq!(
        applied.status,
        OperationStatus::Ok,
        "a coherent state must still be accepted: {:?}",
        applied.diagnostics
    );
    let next = applied.selection_state.expect("accepted apply carries a state");
    assert_eq!(
        next.context_tags,
        map(&[("site", "factory_b")]),
        "the tags are carried through untouched"
    );
    assert_eq!(
        next.choices,
        map(&[("container", "c2")]),
        "the delta is recorded as a choice"
    );
    assert_eq!(
        next.selection_state_hash,
        compute_selection_state_hash(&next).expect("hash over the next state"),
        "the next state is sealed with its own canonical hash"
    );
}

/// The sibling gap, on the read side. `options` delegates on a bad
/// `schema_version` or an empty facet only, so an incoherent state reached the
/// solver here too. Its envelope is the compiler's merged with the solver's
/// `valid_options`, and the compiler's refusal envelope carries an EMPTY list —
/// so the merge published a list of still-valid options computed over the
/// tampered environment on top of an error saying the state is invalid.
///
/// Milder than `select`: the status and diagnostics were already the
/// compiler's, so nothing was ever answered Ok. It is the same screen and the
/// same delegation, and an error envelope must not carry a payload.
#[test]
fn options_over_an_incoherent_state_is_the_compilers_envelope() {
    let compiled = compile_fixture("q50t-options", SITE_CONTAINER_FIXTURE);
    let state = canonical_selection_state(
        compiled.handle.model_hash.clone(),
        "all",
        map(&[("site", "factory_a")]),
        map(&[("site", "factory_b")]),
    )
    .expect("canonical state over the contradictory maps");
    let request = GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: compiled.handle.clone(),
        scope: "all".to_string(),
        selection_state: state,
        facet: "container".to_string(),
        include_pruned_reasons: false,
    };

    let expected = get_selection_options(request.clone());
    assert_eq!(
        expected.status,
        OperationStatus::Error,
        "premise: the compiler already refuses this state: {:?}",
        expected.diagnostics
    );
    assert!(
        expected.valid_options.is_empty(),
        "premise: a refusal envelope offers no options"
    );

    assert_eq!(
        super::options(request),
        expected,
        "options must return the compiler's whole envelope, not one carrying \
         options enumerated over a state the engine refuses"
    );
}

// ---------------------------------------------------------------------------
// configflux-vfh5 — a facet-equality constraint is named in every core it
// makes unsatisfiable, not only in the cores whose clause happens to assert
// both of its facets.
//
// `sorter_container == line_container` (ADR-0057 §D5) is ONE roster entry
// carrying the authored text, and `unsat_attribution` decides whether a core
// clause is accounted for by EVALUATING that text against the assignment the
// clause forbids. The clause the equality actually contributes to a core
// asserts one facet and merely rules ONE value out of the other — so before
// this fix the second facet had two values left, stayed unassigned, and the
// equality evaluated `Unknown`. Unknown is not a violation (ADR-0054 §2), so
// the one rule that made the selection impossible went unnamed.
//
// The three selections below are the three outcomes that produced, and they
// only mean anything read together: the same constraint, over the same model,
// was named in one, replaced by §5.4's anonymous fallback in another, and
// absent from the third. Asserting on the machine core (`constraint_id`s)
// rather than on rendered prose is deliberate — the renderer was never the
// defect.
// ---------------------------------------------------------------------------

/// The issue's model. `site` derives `line_container` through a partial table,
/// `compute_service` accepts only two of the three containers, `sorter_container`
/// is bound by nothing but the equality, and the catalogue holds THREE entries.
///
/// Three is load-bearing. With a two-entry catalogue, ruling one value out
/// leaves exactly one standing and the pre-existing closed-facet entailment
/// (configflux-pt6v) already completes the assignment — the defect is invisible.
/// A third entry is what leaves two values open and drops the constraint.
const FACET_EQUALITY_FIXTURE: &str = r#"{
    "package": "vfh5_facet_equality",
    "version": "1.0.0",
    "facets": {
        "site": {
            "values": ["factory_a", "factory_b"],
            "default": "factory_a",
            "doc": "Which plant this deployment runs at."
        }
    },
    "catalogues": {
        "containers": {
            "doc": "The containers this plant runs on the line.",
            "fields": {
                "width_mm": {"type": "integer", "unit": "mm", "doc": "Internal width"}
            },
            "entries": {
                "c1": {"width_mm": 800},
                "c2": {"width_mm": 600},
                "c3": {"width_mm": 400}
            }
        }
    },
    "bindings": {
        "line_container": {
            "catalogue": "containers",
            "doc": "The container the line draws from, derived from the site.",
            "derive": { "site": { "factory_a": "c1", "factory_b": "c2" } }
        },
        "sorter_container": {
            "catalogue": "containers",
            "default": "c1",
            "doc": "The container the sorter draws from."
        }
    },
    "constraints": {
        "sorter_matches_line": {
            "condition": "sorter_container == line_container",
            "doc": "The sorter and the line must draw from the same container."
        }
    },
    "components": {
        "compute_service": {
            "type": "service",
            "requires": {
                "container": { "binding": "line_container", "accepts": ["c1", "c2"] }
            }
        }
    }
}"#;

/// Apply `choices` in order and explain the FIRST one the solver refuses —
/// the loop `cfx explain` runs (its accumulated state is the satisfiable base,
/// its first rejection is the thing to explain). Reproducing it here rather
/// than driving the binary keeps the test on the rule instead of on the CLI.
///
/// Panics when every choice is accepted: a selection that does not conflict has
/// no core to assert about, so silently passing would hide a broken fixture.
fn explain_first_refusal(
    compiled: &Compiled,
    choices: &[(&str, &str)],
) -> ExplainRejectionResult {
    let mut state = selection(&compiled.handle, &[], &[]);
    for (facet, option) in choices {
        let applied = super::apply(apply_request(&compiled.handle, &state, facet, option));
        match applied.status {
            OperationStatus::Ok => {
                state = applied
                    .selection_state
                    .expect("an accepted choice returns the next state");
            }
            OperationStatus::Error => {
                return explain_for(&compiled.handle, &state, facet, option);
            }
        }
    }
    panic!("every choice was accepted; the fixture models no conflict to explain");
}

/// The `summary` of the one entry a core blames a given constraint id for.
fn summary_for(explained: &ExplainRejectionResult, id: &str) -> String {
    explained
        .rejection
        .unsat_core
        .as_ref()
        .expect("a constraint conflict carries a core")
        .conflicting_constraints
        .iter()
        .find(|clause| clause.constraint_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no core entry named {id}"))
        .summary
        .clone()
}

#[test]
fn a_facet_equality_is_named_when_only_it_forbids_the_selection() {
    // CASE 1, the worse half. Under `site=factory_a` the derive table forces
    // `line_container=c1`, and the equality is then the ONLY rule that forbids
    // `sorter_container=c3` — drop it from the model and the selection is
    // satisfiable. A core that omits it is not a minimal core of this
    // selection: it does not explain the conflict it was asked about.
    let compiled = compile_fixture("vfh5-case1", FACET_EQUALITY_FIXTURE);
    let explained = explain_first_refusal(
        &compiled,
        &[("site", "factory_a"), ("sorter_container", "c3")],
    );

    assert!(
        blamed_constraints(&explained).contains(&"sorter_matches_line".to_string()),
        "the one rule that makes this selection impossible must be in its core: {:?}",
        explained.rejection.unsat_core
    );
    assert_eq!(
        summary_for(&explained, "sorter_matches_line"),
        "sorter_container == line_container",
        "the core quotes the text the author wrote, not a lowered form"
    );
}

#[test]
fn a_facet_equality_is_named_rather_than_reported_anonymous() {
    // CASE 2. The same conflict with `line_container` stated outright. Here the
    // constraint's clause DID reach the core, so §5.4's honest fallback fired
    // on a conflict that a declared constraint — by id, with a doc string —
    // plainly accounts for. The fallback must not be reachable on this core.
    let compiled = compile_fixture("vfh5-case2", FACET_EQUALITY_FIXTURE);
    let explained = explain_first_refusal(
        &compiled,
        &[
            ("site", "factory_a"),
            ("line_container", "c1"),
            ("sorter_container", "c3"),
        ],
    );

    assert_eq!(
        blamed_constraints(&explained),
        vec!["sorter_matches_line".to_string()],
        "the equality accounts for this conflict on its own: {:?}",
        explained.rejection.unsat_core
    );
    let core = explained
        .rejection
        .unsat_core
        .as_ref()
        .expect("a constraint conflict carries a core");
    assert!(
        !core
            .conflicting_constraints
            .iter()
            .any(|clause| clause.summary == MODEL_OVER_CONSTRAINED_SUMMARY),
        "no clause may claim nothing declared accounts for this: {core:?}"
    );
}

#[test]
fn the_already_working_facet_equality_core_is_unchanged() {
    // The contrast case, and the reason it is here: its clause asserts BOTH
    // facets, so it was attributed before this fix and must stay attributed
    // after it. A fix that only moved which clause shape gets named would pass
    // the two cases above and turn this one red.
    let compiled = compile_fixture("vfh5-contrast", FACET_EQUALITY_FIXTURE);
    let explained = explain_first_refusal(
        &compiled,
        &[("line_container", "c2"), ("sorter_container", "c1")],
    );

    assert_eq!(
        blamed_constraints(&explained),
        vec!["sorter_matches_line".to_string()],
        "the previously-working attribution must not move: {:?}",
        explained.rejection.unsat_core
    );
}

#[test]
fn a_constraint_the_selection_does_not_break_is_never_named() {
    // The boundary the fix must NOT cross (ADR-0054 §5.4). Widening attribution
    // to read a clause's negative literals could just as easily start naming a
    // constraint the selection never broke. The pack's one authored constraint
    // is the container equality, and this conflict is not about it: it is
    // between the container the site derives and what the service accepts,
    // which no authored `constraints:` entry mentions at all.
    //
    // Only that half is pinned here. The core's attribution goes to the two
    // LOWERED rules, so §5.4's anonymous fallback does not fire on this core —
    // it is not reachable through the authoring surface at all, and is pinned
    // by the unit suite in `loader_api/unsat_attribution_tests.rs` instead.
    let compiled = compile_fixture("vfh5-anonymous", FACET_EQUALITY_FIXTURE);
    let explained =
        explain_first_refusal(&compiled, &[("site", "factory_b"), ("line_container", "c3")]);

    assert!(
        !blamed_constraints(&explained).contains(&"sorter_matches_line".to_string()),
        "a constraint the selection does not break may never be named: {:?}",
        explained.rejection.unsat_core
    );
}

// ---------------------------------------------------------------------------
// configflux-rzyd — an unsatisfiable selection is never a usage error
// ---------------------------------------------------------------------------
//
// `E_RESOLVE_FACET_UNBOUND` carries a CLAIM, not just a name: ADR-0047 §6 and
// the code's own registry entry say the model "is satisfiable once the facet is
// bound", which is why `cfx` classifies it as a usage error (exit 2) instead of
// the valid-input-but-unsatisfiable family (exit 3, ADR-0042 §3).
//
// A contradiction that empties a binding decided by a `derive` table falsifies
// that claim, and nothing noticed. The chain: the solver's satisfiability gate
// typed-rejects the contradicting choice, `infer_forced_bindings` bails at its
// own rejected-replay guard so `implied_choices` is empty, and the binding has
// no default either — so it is ABSENT from the total assignment the compiler
// evaluates constraints against. The tying constraint then evaluates `Unknown`,
// which is never a violation (ADR-0054 §2), so the contradiction is invisible
// and `resolve_scoped` instead fails on the unmet component requirement.
//
// The user is told to bind the facet or give it a default — advice that cannot
// work, because the derive table WOULD have decided it had the selection been
// satisfiable — while `cfx options` and `cfx explain` both already report the
// selection unsatisfiable. Only `resolve` disagreed, and it disagreed in the
// direction that hides the cause.
//
// The two cases below are the discriminator and they only mean anything read
// together: the SAME missing binding, once where the solver has proved the
// selection unsatisfiable and once where it has proved it satisfiable. A fix
// that collapses them into one class is a regression, not a fix.

/// The genuine unbound case (the negative control): NOTHING decides
/// `spare_container` — no default, no derive, no constraint — and the selection
/// is satisfiable, so the `E_RESOLVE_FACET_UNBOUND` claim holds and the
/// diagnostic must survive untouched.
///
/// Two entries, not one: this fixture must leave the binding genuinely FREE,
/// and a one-entry catalogue would force it (ADR-0054 §5.2 lowers a one-value
/// closed domain to a bare root assertion, so inference would bind it and there
/// would be no unbound facet left to assert about).
const UNBOUND_REQUIREMENT_FIXTURE: &str = r#"{
    "package": "rzyd_unbound_requirement",
    "version": "1.0.0",
    "catalogues": {
        "containers": {
            "doc": "The containers this plant keeps in stock.",
            "fields": {
                "width_mm": {"type": "integer", "unit": "mm", "doc": "Internal width"}
            },
            "entries": {
                "c1": {"width_mm": 800},
                "c2": {"width_mm": 600}
            }
        }
    },
    "bindings": {
        "spare_container": {
            "catalogue": "containers",
            "doc": "The container held as a spare. Nothing decides it."
        }
    },
    "components": {
        "spare_service": {
            "type": "service",
            "requires": { "container": "spare_container" }
        }
    }
}"#;

/// The first diagnostic's code, or a marker when there is no diagnostic at all.
fn first_code(resolved: &ResolveResult) -> &str {
    resolved
        .diagnostics
        .diagnostics
        .first()
        .map(|d| d.code.as_str())
        .unwrap_or("<no diagnostic>")
}

/// The first diagnostic's message, or the empty string.
fn first_message(resolved: &ResolveResult) -> &str {
    resolved
        .diagnostics
        .diagnostics
        .first()
        .map(|d| d.message.as_str())
        .unwrap_or("")
}

#[test]
fn an_unsatisfiable_selection_is_a_conflict_even_when_it_empties_a_derived_binding() {
    // The issue's model and the issue's selection. Under `site=factory_a` the
    // derive table forces `line_container=c1` and the equality forces
    // `sorter_container` to match it, so `sorter_container=c3` is unsatisfiable.
    let compiled = compile_fixture("rzyd-derived", FACET_EQUALITY_FIXTURE);

    // The other surfaces already agree, which is what makes resolve's verdict a
    // DISAGREEMENT rather than a judgement call: with the site fixed, `options`
    // has already pruned `c3` away.
    let site_only = selection(&compiled.handle, &[], &[("site", "factory_a")]);
    assert_eq!(
        options_for(&compiled.handle, &site_only, "sorter_container"),
        vec!["c1".to_string()],
        "the fixture must leave c3 unavailable, or this case models nothing"
    );

    let state = selection(
        &compiled.handle,
        &[],
        &[("site", "factory_a"), ("sorter_container", "c3")],
    );
    let resolved = super::resolve(request(&compiled.handle, &state));

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "an unsatisfiable selection must be rejected"
    );
    assert_eq!(
        first_code(&resolved),
        E_SELECTION_CONFLICT,
        "an unsatisfiable selection belongs to the conflict family (cfx exit 3), \
         never to the usage family: {:?}",
        resolved.diagnostics
    );
    // The second half of the defect, and the half a code-only assertion would
    // miss: the old message named a TRUE fact ("is unbound and has no default")
    // as the CAUSE, sending the reader to add a default that cannot help.
    assert!(
        !first_message(&resolved).contains("has no default"),
        "the rejection must not blame the missing default: {}",
        first_message(&resolved)
    );
    assert!(
        resolved.resolved_output.is_none() && resolved.resolve_hash.is_none(),
        "no snapshot on rejection (ADR-0054 §6)"
    );
}

#[test]
fn a_facet_nothing_decides_is_still_unbound_when_the_selection_is_satisfiable() {
    // The negative control. The solver proves this selection SATISFIABLE —
    // `spare_container` keeps both of its values — so the model really is
    // satisfiable once the facet is bound, which is exactly the claim
    // `E_RESOLVE_FACET_UNBOUND` makes. It must survive as a usage error
    // (cfx exit 2, ADR-0047 §6), message and all.
    let compiled = compile_fixture("rzyd-unbound", UNBOUND_REQUIREMENT_FIXTURE);
    let state = selection(&compiled.handle, &[], &[]);
    let resolved = super::resolve(request(&compiled.handle, &state));

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "a component requiring a binding nothing binds cannot resolve"
    );
    assert_eq!(
        first_code(&resolved),
        E_RESOLVE_FACET_UNBOUND,
        "a genuinely underspecified selection stays a usage error: {:?}",
        resolved.diagnostics
    );
    assert!(
        first_message(&resolved).contains("spare_container")
            && first_message(&resolved).contains("has no default"),
        "the genuine case keeps naming the facet and the missing default: {}",
        first_message(&resolved)
    );
}

// ---------------------------------------------------------------------------
// configflux-eclx (ADR-0030 Amendment 2) — state admissibility (Rule 1), the
// classified replay verdict (Rule 2), and the solver's rejection of a STATE
// rendered wherever the compiler cannot see it (Rule 3)
// ---------------------------------------------------------------------------

/// Three declared CLOSED facets, one constraint tying two of them, and a THIRD
/// facet no rule mentions.
///
/// The third facet is what makes the cases below possible. A delta on it is one
/// the solver accepts whatever the other two say, so the wrapper reaches its own
/// answer without ever asking the compiler about the state it answered over —
/// which is the defect this block pins. A delta on a constrained facet would be
/// adjudicated on its own merits and the state would never be reached.
const THREE_FACET_FIXTURE: &str = r#"{
    "package": "eclx_three_facet",
    "version": "1.0.0",
    "facets": {
        "site":      { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "container": { "values": ["c1", "c2"], "default": "c1" },
        "tier":      { "values": ["gold", "silver"], "default": "silver" }
    },
    "constraints": {
        "factory_b_uses_c2": {
            "condition": "site != 'factory_b' || container == 'c2'",
            "doc": "Factory B only has c2 containers on the line."
        }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// A CLOSED facet the constraints force once another is chosen, beside a facet
/// only a component CONDITION mentions (`region`, undeclared).
///
/// This is S1's smoke shape in miniature (configflux-cy3k): the `.ccm` carries
/// one `region.eu` symbol, so a `region=us` tag is a value the model does not
/// model rather than one it forbids. Rule 2 skips it exactly as it skips a
/// facet the `.ccm` omits entirely, and inference then still gets to run.
const PARTIAL_FACET_FIXTURE: &str = r#"{
    "package": "eclx_partial_facet",
    "version": "1.0.0",
    "facets": {
        "site":      { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "container": { "values": ["c1", "c2"], "default": "c1" }
    },
    "constraints": {
        "factory_b_uses_c2": {
            "condition": "site != 'factory_b' || container == 'c2'",
            "doc": "Factory B only has c2 containers on the line."
        }
    },
    "components": {
        "base": { "type": "service" },
        "eu_only": { "type": "service", "condition": "region == 'eu'" }
    }
}"#;

/// configflux-im7s: a derive-only binding NOTHING requires and no component
/// condition reads, tied by a constraint to a facet the user selects.
///
/// The difference from `FACET_EQUALITY_FIXTURE` is one line — no component
/// `requires` the derived binding — and it is the whole case. With nothing
/// requiring it, the compiler never reaches `E_RESOLVE_FACET_UNBOUND`: the
/// binding is simply unbound, the tying constraint evaluates `Unknown`
/// (ADR-0054 §2 — never a violation), and the compiler composes a full,
/// successful snapshot for a deployment the solver has just proved impossible.
const DERIVED_ONLY_FIXTURE: &str = r#"{
    "package": "eclx_derived_only",
    "version": "1.0.0",
    "facets": {
        "site": {
            "values": ["factory_a", "factory_b"],
            "default": "factory_a",
            "doc": "Which plant this deployment runs at."
        }
    },
    "catalogues": {
        "containers": {
            "doc": "The containers this plant runs on the line.",
            "fields": {
                "width_mm": {"type": "integer", "unit": "mm", "doc": "Internal width"}
            },
            "entries": {
                "c1": {"width_mm": 800},
                "c2": {"width_mm": 600},
                "c3": {"width_mm": 400}
            }
        }
    },
    "bindings": {
        "line_container": {
            "catalogue": "containers",
            "doc": "The container the line draws from, derived from the site.",
            "derive": { "site": { "factory_a": "c1", "factory_b": "c2" } }
        },
        "sorter_container": {
            "catalogue": "containers",
            "default": "c1",
            "doc": "The container the sorter draws from."
        }
    },
    "constraints": {
        "sorter_matches_line": {
            "condition": "sorter_container == line_container",
            "doc": "The sorter and the line must draw from the same container."
        }
    },
    "components": {
        "compute_service": { "type": "service" }
    }
}"#;

/// The skew pair, part one: the model the SOURCES describe.
const SKEW_SOURCE_FIXTURE: &str = r#"{
    "package": "eclx_skew",
    "version": "1.0.0",
    "facets": {
        "container": { "values": ["c1", "c2", "c3"], "default": "c1" },
        "tier":      { "values": ["gold", "silver"], "default": "silver" }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// The skew pair, part two: the same model with ONE declared value missing, so
/// its `.ccm` carries no `container.c3` symbol.
///
/// Pairing two real compilations is how a symbol-table skew is produced without
/// hand-authoring a BDD: the handle keeps the full model's index and chunks and
/// borrows the narrow model's artifact, which is exactly the state a stale
/// `.ccm` beside recompiled sources leaves behind.
const SKEW_CCM_FIXTURE: &str = r#"{
    "package": "eclx_skew",
    "version": "1.0.0",
    "facets": {
        "container": { "values": ["c1", "c2"], "default": "c1" },
        "tier":      { "values": ["gold", "silver"], "default": "silver" }
    },
    "components": {
        "svc": { "type": "service" }
    }
}"#;

/// A `SelectionState` assembled directly from `context_tags` and `choices`,
/// canonically hashed, WITHOUT going through `apply_selection`.
///
/// Every case below needs a state the canonical builders refuse to produce —
/// that is the point: the SDK seam deserializes whatever JSON it is handed, so
/// the states worth testing are precisely the ones no well-behaved client
/// builds.
fn authored_state(
    handle: &ModelHandle,
    context_tags: &[(&str, &str)],
    choices: &[(&str, &str)],
) -> SelectionState {
    canonical_selection_state(
        handle.model_hash.clone(),
        "all",
        map(context_tags),
        map(choices),
    )
    .expect("canonical state over the authored maps")
}

/// The first diagnostic's code on a `select` envelope.
fn apply_code(applied: &ApplySelectionResult) -> &str {
    applied
        .diagnostics
        .diagnostics
        .first()
        .map(|d| d.code.as_str())
        .unwrap_or("<no diagnostic>")
}

/// The first diagnostic's code on an `options` envelope.
fn options_code(listed: &GetSelectionOptionsResult) -> &str {
    listed
        .diagnostics
        .diagnostics
        .first()
        .map(|d| d.code.as_str())
        .unwrap_or("<no diagnostic>")
}

/// An `options` request for one facet over `state`.
fn options_request(
    handle: &ModelHandle,
    state: &SelectionState,
    facet: &str,
) -> GetSelectionOptionsRequest {
    GetSelectionOptionsRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        facet: facet.to_string(),
        include_pruned_reasons: false,
    }
}

/// An `explain` request for one probed `(facet, option)` over `state`.
fn explain_request(
    handle: &ModelHandle,
    state: &SelectionState,
    facet: &str,
    option: &str,
) -> ExplainRejectionRequest {
    ExplainRejectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        rejected_option: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    }
}

/// The contract Rule 1 states on all four surfaces at once: an inadmissible
/// state is refused with `code`, and the refusal is the COMPILER'S whole
/// envelope — not one the wrapper composed for itself (configflux-q50t's
/// convention, which is what keeps the bytes from drifting apart later).
fn assert_every_surface_refuses(handle: &ModelHandle, state: &SelectionState, code: &str) {
    let apply_req = apply_request(handle, state, "site", "factory_b");
    let expected_apply = apply_selection(apply_req.clone());
    let applied = super::apply(apply_req);
    assert_eq!(applied.status, OperationStatus::Error, "select must refuse");
    assert_eq!(apply_code(&applied), code, "select: {:?}", applied.diagnostics);
    assert_eq!(
        applied, expected_apply,
        "select must return the compiler's whole envelope for an inadmissible state"
    );

    let options_req = options_request(handle, state, "site");
    let expected_options = get_selection_options(options_req.clone());
    let listed = super::options(options_req);
    assert_eq!(listed.status, OperationStatus::Error, "options must refuse");
    assert_eq!(options_code(&listed), code, "options: {:?}", listed.diagnostics);
    assert_eq!(
        listed, expected_options,
        "options must return the compiler's whole envelope for an inadmissible state"
    );

    let resolve_req = request(handle, state);
    let expected_resolve = resolve_from_selection(resolve_req.clone());
    let resolved = super::resolve(resolve_req);
    assert_eq!(resolved.status, OperationStatus::Error, "resolve must refuse");
    assert_eq!(first_code(&resolved), code, "resolve: {:?}", resolved.diagnostics);
    assert_eq!(
        resolved, expected_resolve,
        "resolve must return the compiler's whole envelope for an inadmissible state"
    );

    let explain_req = explain_request(handle, state, "site", "factory_b");
    let expected_explain = explain_rejection(explain_req.clone());
    let explained = super::explain(explain_req);
    assert_eq!(
        explained.status,
        OperationStatus::Error,
        "explain must refuse: the operation cannot run over a state the model rejects"
    );
    assert_eq!(explained.rejection.code, code, "explain: {:?}", explained.rejection);
    assert_eq!(
        explained, expected_explain,
        "explain must return the compiler's whole envelope for an inadmissible state"
    );
}

/// T1/T1b — a choice on a declared CLOSED facet carrying a value outside its
/// domain. Every binding configflux-q50t added passes: the hash is canonical,
/// the model and scope match, tags and choices agree. Only the MODEL can refuse
/// it, and before this task nothing asked the model.
#[test]
fn a_choice_outside_its_facets_domain_is_refused_on_every_surface() {
    let compiled = compile_fixture("eclx-domain", THREE_FACET_FIXTURE);
    let state = authored_state(&compiled.handle, &[], &[("container", "c9")]);

    assert_every_surface_refuses(&compiled.handle, &state, E_SELECTION_INVALID_OPTION);
}

/// T2 — a choice on a facet the model does not know at all. The `.ccm` has no
/// symbol for it, so the replay SKIPS it and the wrapper composed an answer
/// over a deployment naming a facet that does not exist.
#[test]
fn a_choice_on_an_unknown_facet_is_refused_on_every_surface() {
    let compiled = compile_fixture("eclx-unknown", THREE_FACET_FIXTURE);
    let state = authored_state(&compiled.handle, &[], &[("nosuch", "x")]);

    assert_every_surface_refuses(&compiled.handle, &state, E_SELECTION_UNKNOWN_FACET);
}

/// T3 — a context tag on a declared CLOSED facet with an undeclared value. Tags
/// are screened, but only where the model actually declares the domain.
#[test]
fn a_context_tag_outside_a_closed_facets_declared_values_is_refused() {
    let compiled = compile_fixture("eclx-tag-domain", THREE_FACET_FIXTURE);
    let state = authored_state(&compiled.handle, &[("container", "c9")], &[]);

    assert_every_surface_refuses(&compiled.handle, &state, E_SELECTION_INVALID_OPTION);
}

/// T3b — the other half of the tag rule, and the one the corpus depends on: a
/// tag on a facet only a component CONDITION mentions, carrying a value the
/// condition does not name. S1's smoke golden resolves with `region=us` against
/// a model whose only mention of `region` is `region == 'eu'`.
///
/// Screening it would refuse a shipped scenario, and Rule 2 must not reject the
/// replay over it either: `region.us` is a value the `.ccm` does not model, not
/// one it forbids. Every surface answers normally.
#[test]
fn a_tag_on_a_condition_only_facet_is_never_screened_or_rejected() {
    let compiled = compile_fixture("eclx-condition-only", PARTIAL_FACET_FIXTURE);
    let state = authored_state(&compiled.handle, &[("region", "us")], &[]);

    let applied = super::apply(apply_request(&compiled.handle, &state, "site", "factory_a"));
    assert_eq!(
        applied.status,
        OperationStatus::Ok,
        "select must still answer over an unmodelled tag value: {:?}",
        applied.diagnostics
    );

    let listed = super::options(options_request(&compiled.handle, &state, "site"));
    assert_eq!(
        listed.status,
        OperationStatus::Ok,
        "options must still answer: {:?}",
        listed.diagnostics
    );

    let explained = super::explain(explain_request(&compiled.handle, &state, "site", "factory_a"));
    assert_eq!(
        explained.status,
        OperationStatus::Ok,
        "explain must still run: {:?}",
        explained.diagnostics
    );

    let resolved = super::resolve(request(&compiled.handle, &state));
    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "resolve must still compose: {:?}",
        resolved.diagnostics
    );
}

/// T4 (configflux-oime) — an incoherent state under an unreachable `.ccm`
/// reports the STATE. `resolve` used to reach `load_solver_ccm` first and blame
/// the missing artifact, so a caller fixing what the diagnostic named still had
/// an invalid state; `apply` and `options` already chose the other precedence.
#[test]
fn resolve_reports_an_incoherent_state_ahead_of_an_unreachable_ccm() {
    let compiled = compile_fixture("eclx-oime", THREE_FACET_FIXTURE);
    let mut handle = compiled.handle.clone();
    handle.ccm_ref = String::new();
    // A hash that is not the canonical hash of the state it seals — the
    // integrity binding, forged directly rather than through `resealed`, which
    // exists to RE-seal.
    let state = SelectionState {
        selection_state_hash: "0".repeat(64),
        ..selection(&compiled.handle, &[], &[])
    };

    let resolved = super::resolve(request(&handle, &state));
    assert_eq!(resolved.status, OperationStatus::Error);
    assert_eq!(
        first_code(&resolved),
        E_SELECTION_STATE_INVALID,
        "the state is the thing that is wrong, and the caller can fix it: {:?}",
        resolved.diagnostics
    );
}

/// Rule 3(a)'s diagnostic, written out here as LITERAL text.
///
/// It is spelled twice on purpose. A test that borrowed the production
/// constructor would agree with any rewording of it — a dropped hint, a renamed
/// code, a message that no longer names the assignment — and the wording is
/// precisely what the rule fixes: it is what `cfx` prints, what
/// `docs/diagnostics.md` documents, and what an operator reads when a
/// deployment is refused. Asserting a substring or the code alone would leave
/// all of that free to drift, which is how four surfaces came to describe one
/// fact four ways (ADR-0054).
fn expected_conflict_diagnostic(facet: &str, option: &str) -> Diagnostic {
    Diagnostic {
        code: "E_SELECTION_CONFLICT".to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!(
            "Selection is unsatisfiable: no assignment of the remaining facets \
             satisfies the model once '{facet}' = '{option}' is applied"
        ),
        source_id: None,
        entity_path: None,
        hint: Some(
            "Run explain with the same selection to see the minimal set of choices that conflict"
                .to_string(),
        ),
    }
}

/// T5 — the headline. Two in-domain choices one constraint forbids TOGETHER,
/// with the delta on a third facet. Nothing in the state is individually wrong,
/// so Rule 1 admits it; the solver proves the pair impossible.
#[test]
fn select_refuses_a_state_the_solver_proves_unsatisfiable() {
    let compiled = compile_fixture("eclx-unsat-select", THREE_FACET_FIXTURE);
    let state = authored_state(
        &compiled.handle,
        &[],
        &[("site", "factory_b"), ("container", "c1")],
    );

    let req = apply_request(&compiled.handle, &state, "tier", "gold");
    let legacy = apply_selection(req.clone());
    let applied = super::apply(req);

    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "select must not compose an Ok over a state the solver refuses: {:?}",
        applied.diagnostics
    );
    assert_eq!(
        apply_code(&applied),
        E_SELECTION_CONFLICT,
        "a refused state is a selection conflict: {:?}",
        applied.diagnostics
    );
    // Where the compiler can see the contradiction for itself its bytes win, so
    // the two engines answer with ONE envelope. Where it cannot, the wrapper
    // renders the conflict and names the refused assignment.
    if legacy.status == OperationStatus::Error {
        assert_eq!(
            applied, legacy,
            "the compiler saw the contradiction; select must be its envelope"
        );
    } else {
        assert_eq!(
            applied.diagnostics.diagnostics,
            vec![expected_conflict_diagnostic("site", "factory_b")],
            "the wrapper's rendering must be Rule 3(a)'s diagnostic verbatim"
        );
    }
}

/// T5a — Rule 3(b)'s OTHER arm, the one T5 above cannot reach.
///
/// T5's contradiction is between two DECLARED facets, so the compiler sees it
/// too and its bytes win: `select` returns the legacy envelope and the arm that
/// RENDERS the conflict never runs. That arm is the substance of Rule 3(b) —
/// it is what stops `select` composing an `Ok` over a deployment the solver has
/// already refused — so it needs a case where the compiler genuinely cannot
/// see the contradiction.
///
/// configflux-im7s's shape supplies one. `sorter_container == line_container`
/// ties the choice to a derive-only binding nothing requires, so at selection
/// time the compiler evaluates it `Unknown` (ADR-0054 §2 — never a violation)
/// and accepts. The total BDD does not: no site leaves `line_container` at
/// `c3`, so the STATE is unsatisfiable before any delta is considered.
///
/// The delta is on `site` and the refusal names `sorter_container` — that
/// asymmetry is the point, and it is what separates this arm from the DELTA arm
/// T7 covers. The state is the thing that is wrong; changing the delta would
/// leave the caller holding the same impossible deployment.
#[test]
fn select_renders_the_state_conflict_the_compiler_cannot_see() {
    let compiled = compile_fixture("eclx-unsat-select-rendered", DERIVED_ONLY_FIXTURE);
    // Built through the compiler's own `apply_selection`, so this is a state a
    // well-behaved client really holds — nothing here is hand-authored.
    let state = selection(&compiled.handle, &[], &[("sorter_container", "c3")]);

    let req = apply_request(&compiled.handle, &state, "site", "factory_a");
    let legacy = apply_selection(req.clone());
    let applied = super::apply(req);

    assert_eq!(
        legacy.status,
        OperationStatus::Ok,
        "premise: the compiler must ACCEPT this request, or the rendering arm \
         under test is never reached: {:?}",
        legacy.diagnostics
    );
    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "the solver refused the STATE, so no delta over it may be accepted: {:?}",
        applied.diagnostics
    );
    assert_eq!(
        applied.diagnostics.diagnostics,
        vec![expected_conflict_diagnostic("sorter_container", "c3")],
        "select must render Rule 3(a)'s diagnostic verbatim"
    );
    assert!(
        !applied.diagnostics.diagnostics[0].message.contains("factory_a"),
        "the STATE's refused assignment is the one to report, not the delta's: {:?}",
        applied.diagnostics
    );
    assert_eq!(
        (applied.error_count, applied.warning_count),
        (1, 0),
        "one error, no warnings"
    );
    assert!(
        applied.selection_state.is_none(),
        "a refused select advances no state"
    );
    assert_eq!(
        (applied.model_hash, applied.scope),
        (legacy.model_hash, legacy.scope),
        "the rendered envelope keeps the identity the compiler's carries"
    );
}

/// T5b — the same state through `options`. An error envelope carries no
/// payload, so the option list is empty rather than enumerated over a
/// deployment the engine has just called impossible.
#[test]
fn options_refuses_a_state_the_solver_proves_unsatisfiable() {
    let compiled = compile_fixture("eclx-unsat-options", THREE_FACET_FIXTURE);
    let state = authored_state(
        &compiled.handle,
        &[],
        &[("site", "factory_b"), ("container", "c1")],
    );

    let listed = super::options(options_request(&compiled.handle, &state, "tier"));

    assert_eq!(
        listed.status,
        OperationStatus::Error,
        "options must not enumerate over a refused state: {:?}",
        listed.diagnostics
    );
    assert_eq!(
        options_code(&listed),
        E_SELECTION_CONFLICT,
        "options: {:?}",
        listed.diagnostics
    );
    assert!(
        listed.valid_options.is_empty(),
        "an error envelope must carry no payload: {:?}",
        listed.valid_options
    );
}

/// T5c — the same state through `explain`. A rejection is normally the SUCCESS
/// path (ADR-0031 D2), but this is not the probe's rejection: the operation
/// cannot run at all, which is the exit-2 class.
#[test]
fn explain_cannot_run_over_a_state_the_solver_proves_unsatisfiable() {
    let compiled = compile_fixture("eclx-unsat-explain", THREE_FACET_FIXTURE);
    let state = authored_state(
        &compiled.handle,
        &[],
        &[("site", "factory_b"), ("container", "c1")],
    );

    let explained = super::explain(explain_request(&compiled.handle, &state, "tier", "gold"));

    assert_eq!(
        explained.status,
        OperationStatus::Error,
        "explain over an unsatisfiable state is a command error: {:?}",
        explained.diagnostics
    );
    // The whole reason, not its code: `blocking_choices` and the absent core
    // are as much of Rule 3(a) as the message is, and `explain` is the only
    // surface that carries them.
    let expected = expected_conflict_diagnostic("site", "factory_b");
    assert_eq!(
        explained.rejection,
        RejectionReason {
            code: E_SELECTION_CONFLICT.to_string(),
            message: expected.message.clone(),
            blocking_choices: BTreeMap::new(),
            hint: expected.hint.clone(),
            unsat_core: None,
        },
        "explain must render Rule 3(a) verbatim, with no blocking choices and no core"
    );
    assert_eq!(
        explained.diagnostics.diagnostics,
        vec![expected],
        "the reason and the diagnostics report must not disagree"
    );
}

/// T6 (configflux-im7s) — `resolve` composed a full snapshot for an impossible
/// deployment. The contradiction runs through a derive-only binding NOTHING
/// requires, so the compiler never even reaches the unbound-facet complaint
/// configflux-rzyd rewrote: it evaluates the tying constraint `Unknown`, sees
/// no violation, and composes `Ok` — over a selection the solver has already
/// proved unsatisfiable and which `options` and `explain` both refuse.
#[test]
fn resolve_refuses_a_selection_the_solver_proves_unsatisfiable_with_no_unbound_complaint() {
    let compiled = compile_fixture("eclx-im7s", DERIVED_ONLY_FIXTURE);

    // The premise, so the case cannot pass for an unrelated reason: with the
    // site fixed, `c3` is already unavailable to the sorter.
    let site_only = selection(&compiled.handle, &[], &[("site", "factory_a")]);
    assert_eq!(
        options_for(&compiled.handle, &site_only, "sorter_container"),
        vec!["c1".to_string()],
        "the fixture must leave c3 unavailable, or this case models nothing"
    );

    let state = selection(
        &compiled.handle,
        &[],
        &[("site", "factory_a"), ("sorter_container", "c3")],
    );
    let resolved = super::resolve(request(&compiled.handle, &state));

    assert_eq!(
        resolved.status,
        OperationStatus::Error,
        "no snapshot may be composed for a deployment the solver refuses: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        first_code(&resolved),
        E_SELECTION_CONFLICT,
        "the refusal belongs to the conflict family: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.diagnostics.diagnostics,
        vec![expected_conflict_diagnostic("sorter_container", "c3")],
        "resolve must render Rule 3(a)'s diagnostic verbatim, naming the \
         assignment the solver refused"
    );
    assert!(
        resolved.resolved_output.is_none()
            && resolved.resolve_hash.is_none()
            && resolved.resolved_output_hash.is_none(),
        "a refused resolve delivers no payload and no payload identity"
    );
}

/// T7 — ADR-0030 D3's delta arm, narrowed. The solver refuses a delta that
/// empties a derive-only binding and the compiler accepts it, because after
/// ADR-0057 the compiler's check is three-valued and `Unknown` is never a
/// violation. That is a legitimate model exercising a legitimate construct, not
/// a correctness incident, so it is reported as the conflict it is rather than
/// as an internal fault telling the user to recompile.
///
/// Replaces the case that asserted the old `E_SELECTION_ENGINE_DIVERGENCE`
/// arm — same fixture, same request, the decided expectation.
#[test]
fn select_reports_a_delta_the_solver_refuses_as_a_conflict_not_a_divergence() {
    let compiled = compile_fixture("eclx-delta", FACET_EQUALITY_FIXTURE);
    let state = selection(&compiled.handle, &[], &[("site", "factory_a")]);

    let req = apply_request(&compiled.handle, &state, "sorter_container", "c3");
    let legacy = apply_selection(req.clone());
    let applied = super::apply(req);

    assert_eq!(
        legacy.status,
        OperationStatus::Ok,
        "premise: the compiler must ACCEPT this delta, or the arm under test \
         is never reached: {:?}",
        legacy.diagnostics
    );
    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "the solver refuses this delta, so select must: {:?}",
        applied.diagnostics
    );
    assert_eq!(
        apply_code(&applied),
        E_SELECTION_CONFLICT,
        "a working model must not be reported as engine divergence: {:?}",
        applied.diagnostics
    );
    assert_eq!(
        applied.diagnostics.diagnostics,
        vec![expected_conflict_diagnostic("sorter_container", "c3")],
        "the delta arm must render Rule 3(a)'s diagnostic verbatim, naming the DELTA"
    );
}

/// T8 (configflux-cy3k's symptom) — one partially modelled facet used to
/// disable inference for the whole model. The replay met `region=us`, reported
/// a flat rejection, and `infer_forced_bindings` abandoned every entailment the
/// constraints held — so a resolve over a perfectly ordinary deployment lost
/// the binding the model itself decides.
#[test]
fn a_partially_modelled_facet_no_longer_disables_inference() {
    let compiled = compile_fixture("eclx-cy3k", PARTIAL_FACET_FIXTURE);
    let state = authored_state(&compiled.handle, &[("region", "us")], &[("site", "factory_b")]);

    let resolved = super::resolve(request(&compiled.handle, &state));

    assert_eq!(
        resolved.status,
        OperationStatus::Ok,
        "the deployment is ordinary and must resolve: {:?}",
        resolved.diagnostics
    );
    assert_eq!(
        resolved.implied_choices.get("container").map(String::as_str),
        Some("c2"),
        "the constraint decides `container`, and an unmodelled tag elsewhere \
         must not cost that: {:?}",
        resolved.implied_choices
    );
}

/// T9 — artifact skew: the `.ccm` does not model a value the sources DECLARE.
/// Rule 1 admits it (the model declares it), the replay cannot apply it, and
/// the two facts together mean the artifact is stale rather than the request
/// wrong. That is the one thing the divergence code is still for, and the
/// message says which value is missing rather than "recompile something".
#[test]
fn a_declared_value_the_ccm_does_not_model_is_reported_as_skew() {
    let sources = compile_fixture("eclx-skew-sources", SKEW_SOURCE_FIXTURE);
    let narrow = compile_fixture("eclx-skew-ccm", SKEW_CCM_FIXTURE);

    let mut skewed = sources.handle.clone();
    skewed.ccm_ref = narrow.handle.ccm_ref.clone();

    // The replay classifies it directly: `c3` is a declared value of a CLOSED
    // facet, so its absence from the symbol table is skew and not a deployment
    // the model merely does not model.
    let state = authored_state(&sources.handle, &[], &[("container", "c3")]);
    let closed = compiler::loader_api::closed_facet_domains(&sources.handle)
        .expect("closed facet roster over the source model");
    let mut session = fixture_session(&skewed);
    assert!(
        matches!(
            super::apply_environment(&mut session, &state, &closed),
            super::EnvReplay::Skew { .. }
        ),
        "a declared value the .ccm omits is skew, not a rejection"
    );

    // The delta is on the OTHER facet: a delta on `container` would be
    // adjudicated by the already-chosen guard before any session was built, and
    // the skew would never be reached.
    let applied = super::apply(apply_request(&skewed, &state, "tier", "gold"));
    assert_eq!(applied.status, OperationStatus::Error);
    assert_eq!(
        apply_code(&applied),
        E_SELECTION_ENGINE_DIVERGENCE,
        "skew is the internal-fault family: {:?}",
        applied.diagnostics
    );
    assert!(
        applied.diagnostics.diagnostics[0]
            .message
            .contains("does not model declared value 'c3' of closed facet 'container'"),
        "the message must name the missing value: {:?}",
        applied.diagnostics
    );
}

/// T10 — the accept path, unmoved. Chained selects over an admissible state
/// each return `Ok` with a fresh canonical hash, which is what every legitimate
/// client does and what all three rules must leave alone.
#[test]
fn chained_selects_over_an_admissible_state_still_answer_ok() {
    let compiled = compile_fixture("eclx-accept", THREE_FACET_FIXTURE);
    let mut state = selection(&compiled.handle, &[], &[]);
    let mut seen = vec![state.selection_state_hash.clone()];

    for (facet, option) in [("site", "factory_b"), ("container", "c2"), ("tier", "gold")] {
        let applied = super::apply(apply_request(&compiled.handle, &state, facet, option));
        assert_eq!(
            applied.status,
            OperationStatus::Ok,
            "select {facet}={option} must still be accepted: {:?}",
            applied.diagnostics
        );
        state = applied.selection_state.expect("next state");
        assert!(
            !seen.contains(&state.selection_state_hash),
            "each accepted select must re-canonicalize to a fresh hash"
        );
        seen.push(state.selection_state_hash.clone());
    }

    assert_eq!(
        state.choices,
        map(&[("site", "factory_b"), ("container", "c2"), ("tier", "gold")]),
        "the chained selects must accumulate"
    );
}

/// One `select` refusal, asserted the way every other Rule 1 case is: the
/// status, the code, and the WHOLE envelope against the compiler's own, so a
/// wrapper that started composing its own bytes here would be caught.
fn assert_select_refuses(
    handle: &ModelHandle,
    state: &SelectionState,
    facet: &str,
    option: &str,
    code: &str,
    case: &str,
) {
    let apply_req = apply_request(handle, state, facet, option);
    let expected = apply_selection(apply_req.clone());
    let applied = super::apply(apply_req);

    assert_eq!(
        applied.status,
        OperationStatus::Error,
        "{case}: select must refuse an inadmissible state, delta {facet}={option}"
    );
    assert_eq!(apply_code(&applied), code, "{case}: {:?}", applied.diagnostics);
    assert_eq!(
        applied, expected,
        "{case}: select must return the compiler's whole envelope"
    );
}

/// The screen is asked before the DELTA is adjudicated, not after.
///
/// `apply_selection` used to answer the delta from the state alone — a delta
/// repeating a choice the state already holds is idempotent, so it returned the
/// state back with `status: ok` — and it did so BEFORE it loaded the model, so
/// Rule 1's screen never ran. Nothing in `session_compose` could compensate:
/// `apply`'s pre-screen disjunction short-circuits on `choices.contains_key`
/// and delegates, and the delegate was the function with the hole.
///
/// The offending entry does not have to be the delta's own facet, and it does
/// not have to be a choice — the last two cases carry an admissible delta over
/// a state made inadmissible somewhere else entirely.
#[test]
fn an_idempotent_delta_over_an_inadmissible_state_is_still_refused() {
    let compiled = compile_fixture("eclx-idempotent", THREE_FACET_FIXTURE);

    // The delta's own facet is the unknown one.
    let ghost = authored_state(&compiled.handle, &[], &[("nosuch", "x")]);
    assert_select_refuses(
        &compiled.handle,
        &ghost,
        "nosuch",
        "x",
        E_SELECTION_UNKNOWN_FACET,
        "unknown facet, echoed by the delta",
    );

    // The delta's own facet carries a value outside its declared domain.
    let out_of_domain = authored_state(&compiled.handle, &[], &[("container", "c9")]);
    assert_select_refuses(
        &compiled.handle,
        &out_of_domain,
        "container",
        "c9",
        E_SELECTION_INVALID_OPTION,
        "out-of-domain choice, echoed by the delta",
    );

    // The delta is admissible and idempotent; a DIFFERENT choice is not.
    let elsewhere = authored_state(
        &compiled.handle,
        &[],
        &[("container", "c9"), ("tier", "gold")],
    );
    assert_select_refuses(
        &compiled.handle,
        &elsewhere,
        "tier",
        "gold",
        E_SELECTION_INVALID_OPTION,
        "the offending entry is not the delta's facet",
    );

    // The same, with the offending entry in `context_tags` rather than
    // `choices` — the tag half of the screen sits behind the same early return.
    let tagged = authored_state(
        &compiled.handle,
        &[("tier", "enterprise")],
        &[("site", "factory_a")],
    );
    assert_select_refuses(
        &compiled.handle,
        &tagged,
        "site",
        "factory_a",
        E_SELECTION_INVALID_OPTION,
        "the offending entry is a context tag",
    );
}

/// The other two arms behind the same early return.
///
/// A delta that contradicts a context tag, and one that re-decides an
/// already-chosen facet, are both answered from the state alone — as
/// `E_SELECTION_CONFLICT`, which is an ANSWER about the delta. Rule 1 says the
/// state is screened before any adjudication, so over an inadmissible state
/// these must report the state instead: the conflict is a true statement about
/// a deployment that cannot exist, and fixing the delta it names would leave
/// the caller holding the same unusable state.
#[test]
fn a_delta_the_state_alone_could_answer_still_reports_the_state() {
    let compiled = compile_fixture("eclx-adjudicated", THREE_FACET_FIXTURE);

    // Already chosen, different value.
    let already_chosen = authored_state(
        &compiled.handle,
        &[("tier", "enterprise")],
        &[("site", "factory_a")],
    );
    assert_select_refuses(
        &compiled.handle,
        &already_chosen,
        "site",
        "factory_b",
        E_SELECTION_INVALID_OPTION,
        "already-chosen facet over an inadmissible state",
    );

    // Pinned by an immutable context tag, different value.
    let pinned = authored_state(
        &compiled.handle,
        &[("site", "factory_a"), ("tier", "enterprise")],
        &[],
    );
    assert_select_refuses(
        &compiled.handle,
        &pinned,
        "site",
        "factory_b",
        E_SELECTION_INVALID_OPTION,
        "context-tag-pinned facet over an inadmissible state",
    );
}

// ---------------------------------------------------------------------------
// configflux-tsuf — a refused `options` request reports no options.
//
// `options` asks the compiler for the envelope and overrides `valid_options`
// with the solver's decision. On a REFUSAL the compiler's envelope carries an
// empty list deliberately (ADR-0030 D5 owns those bytes; Amendment 2 states the
// rule: an error envelope must carry no payload), and the override replaced it
// — so the caller held one result saying both "this request failed" and "here
// are the valid options".
//
// The first two cases drive the merge itself, which is where the rule now
// lives, so neither depends on which refusals happen to be reachable today.
// The third proves one is reachable end to end, through both tail sites.
// ---------------------------------------------------------------------------

/// A facet NO declaration mentions, named by one component selector with a `!=`
/// atom: `region`, against a model whose own policy is over declared facets.
///
/// `!=` is the entire point. The emitter's symbol walk collects both operators,
/// so `region.eu` is a real `.ccm` variable the solver enumerates, while the
/// loader widens `facet_domains` on `==` atoms only — so the compiler has no
/// domain for `region` and refuses to enumerate it. Write the selector as
/// `region == 'eu'` instead (S1's shape, `PARTIAL_FACET_FIXTURE`) and the
/// compiler answers normally, which is why that fixture cannot stand in here.
///
/// The selector lands its symbols as tautologies and is never asserted on the
/// BDD root (ADR-0054 §5.1), so the solver holds `eu` VALID — a populated list
/// arriving at the merge beside the compiler's refusal.
const SELECTOR_ONLY_NE_FIXTURE: &str = r#"{
    "package": "tsuf_selector_only_ne",
    "version": "1.0.0",
    "facets": {
        "site":      { "values": ["factory_a", "factory_b"], "default": "factory_a" },
        "container": { "values": ["c1", "c2"], "default": "c1" }
    },
    "constraints": {
        "factory_b_uses_c2": {
            "condition": "site != 'factory_b' || container == 'c2'",
            "doc": "Factory B only has c2 containers on the line."
        }
    },
    "components": {
        "base": { "type": "service" },
        "non_eu": { "type": "service", "condition": "region != 'eu'" }
    }
}"#;

/// An `options` envelope in the shape the compiler's `selection_options_failed`
/// builds: `Error`, one diagnostic, and every payload field empty.
///
/// Written out here rather than borrowed from a production constructor on
/// purpose — a fixture that agrees with the code it checks would agree with a
/// future code that splices a payload into an error too.
fn refusal_envelope() -> GetSelectionOptionsResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: vec![Diagnostic {
            code: E_SELECTION_UNKNOWN_FACET.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "Unknown selection facet 'region'".to_string(),
            source_id: None,
            entity_path: None,
            hint: Some("Use get_selection_options on a known facet from model conditions".to_string()),
        }],
        error_count: 1,
        warning_count: 0,
    };
    GetSelectionOptionsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash: "model-hash".to_string(),
        scope: "all".to_string(),
        facet: "region".to_string(),
        valid_options: Vec::new(),
        default: None,
        declared_open: None,
        pruned_options: None,
        selection_state_hash: "state-hash".to_string(),
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// A successful `options` envelope carrying everything the merge exists to
/// carry: the ADR-0047 §6 `default` and `declared_open` annotations, the legacy
/// `pruned_options` list, and a `valid_options` the solver is about to override.
fn served_envelope() -> GetSelectionOptionsResult {
    GetSelectionOptionsResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: "model-hash".to_string(),
        scope: "all".to_string(),
        facet: "cooling_model".to_string(),
        valid_options: vec!["x200".to_string()],
        default: Some("x200".to_string()),
        declared_open: Some(false),
        pruned_options: Some(vec![PrunedOptionReason {
            option: "a9".to_string(),
            reason: "no satisfiable branch remains".to_string(),
        }]),
        selection_state_hash: "state-hash".to_string(),
        error_count: 0,
        warning_count: 0,
        diagnostics_ref: None,
        diagnostics: DiagnosticsReport {
            schema_version: PRODUCT_SCHEMA_VERSION,
            diagnostics: Vec::new(),
            error_count: 0,
            warning_count: 0,
        },
    }
}

/// A refusal keeps its empty option list, its status and its diagnostics. The
/// whole envelope is compared, so nothing else may be spliced in either.
#[test]
fn an_error_envelope_passes_through_the_merge_untouched() {
    let refused = refusal_envelope();

    let merged = super::merge_valid_options(
        refused.clone(),
        vec!["eu".to_string(), "us".to_string()],
    );

    assert_eq!(
        merged, refused,
        "a refused request has no valid set to report: the compiler's envelope \
         must come back verbatim"
    );
    assert!(
        merged.valid_options.is_empty(),
        "the refusal's empty option list must survive the merge"
    );
}

/// The success path is unchanged, and it is a MERGE rather than a rebuild
/// precisely so the declaration annotations ride along (ADR-0047 §6). Both
/// halves are pinned here: the solver's decision wins on `valid_options`, and
/// every other field is the compiler's.
#[test]
fn the_merge_overrides_the_option_list_and_carries_the_declaration_annotations() {
    let served = served_envelope();
    let solver_decision = vec!["a9".to_string(), "x200".to_string()];

    let merged = super::merge_valid_options(served.clone(), solver_decision.clone());

    assert_eq!(
        merged.valid_options, solver_decision,
        "the solver decides the still-valid set on the success path"
    );
    assert_eq!(
        merged,
        GetSelectionOptionsResult {
            valid_options: solver_decision,
            ..served
        },
        "only valid_options may differ: `default`, `declared_open`, \
         `pruned_options` and the diagnostics are the compiler's"
    );
}

/// The end-to-end case, through the real compiler and the real solver.
///
/// `region` is named by one component selector and declared nowhere. The
/// emitter lands BOTH `==` and `!=` atoms in the `.ccm` symbol table
/// (configflux-9xxq / ADR-0054 §5.1), so the solver models `region` and
/// enumerates it; the loader widens `facet_domains` from `==` atoms only, so the
/// compiler has no domain for it and refuses with `E_SELECTION_UNKNOWN_FACET` —
/// the unconstrained-facet diagnostic it permanently owns (ADR-0030 D5). That
/// is a refusal and a non-empty solver list meeting at the merge, which is the
/// defect's exact shape, and `!=` rather than `==` is the whole fixture.
///
/// Both tail sites are driven: the `include_pruned_reasons` branch merges in its
/// own place, after a `pruned_options` recompute that must not touch a refusal
/// either.
#[test]
fn options_over_a_facet_the_compiler_cannot_enumerate_is_the_compilers_envelope() {
    let compiled = compile_fixture("tsuf-selector-only", SELECTOR_ONLY_NE_FIXTURE);
    let state = selection(&compiled.handle, &[], &[]);

    // Premise: the solver holds an option for `region`, so the merge had
    // something to splice. Without this the test could pass for the wrong
    // reason — an empty solver list looks like a fixed merge.
    let session = fixture_session(&compiled.handle);
    assert_eq!(
        session
            .valid_options("region")
            .expect("the .ccm models region")
            .options,
        vec!["eu".to_string()],
        "premise: the solver enumerates the selector's facet"
    );

    for include_pruned_reasons in [false, true] {
        let request = GetSelectionOptionsRequest {
            include_pruned_reasons,
            ..options_request(&compiled.handle, &state, "region")
        };

        let expected = get_selection_options(request.clone());
        assert_eq!(
            expected.status,
            OperationStatus::Error,
            "premise: the compiler refuses a facet it has no domain for: {:?}",
            expected.diagnostics
        );
        assert_eq!(
            options_code(&expected),
            E_SELECTION_UNKNOWN_FACET,
            "premise: the refusal is the unconstrained-facet diagnostic"
        );

        let listed = super::options(request);
        assert!(
            listed.valid_options.is_empty(),
            "a refused options request must report no options \
             (include_pruned_reasons={include_pruned_reasons}): {:?}",
            listed.valid_options
        );
        assert_eq!(
            listed, expected,
            "the whole envelope is the compiler's \
             (include_pruned_reasons={include_pruned_reasons})"
        );
    }
}
