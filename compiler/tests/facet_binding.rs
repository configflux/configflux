// SPDX-License-Identifier: BUSL-1.1

//! configflux-tcp5 / ADR-0064 D1-D4: a parameter is a facet's runtime handle
//! only when it DECLARES so.
//!
//! Before this, the runtime derived the mapping from the last path segment —
//! `component.<id>.param.<key>` was taken to name facet `<key>` — which is
//! many-to-one, so two paths could land on one facet, disagree, and have the
//! facet dropped from the assignment entirely. That containment is fail-OPEN:
//! on affected model shapes a declared constraint silently is not enforced.
//! The compiler half replaces the guess with a declaration, validates it, and
//! propagates the facet's value into the parameter.
//!
//! Black box: everything below drives the public product and loader APIs —
//! `verify_model`, `compile_model`, `link_model`, `compile_object` and the
//! resolve chain — never the validators or the resolver directly. The four
//! declaration rules are each asserted through BOTH `verify_model` and
//! `compile_model`, because the whole point of putting them in `link_verify`
//! rather than at emit time is that the two agree (configflux-8o20 fact 2),
//! and only a paired assertion can see that.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use compiler::loader_api::{
    apply_selection, initialize_selection_state, open_model, resolve_from_selection,
    ApplySelectionRequest, InitializeSelectionStateRequest, ModelHandle, OpenModelRequest,
    ResolveFromSelectionRequest, ResolveResult, SelectionDelta, SelectionState,
    E_RESOLVE_FAILED,
};
use compiler::object_compile::{compile_object, CompileObjectRequest};
use compiler::product_api::{
    compile_model, inspect_model, link_model, verify_model, CompileModelRequest, CompileResult,
    Diagnostic, InspectModelRequest, InspectQuery, InspectionItem, InspectionResult,
    LinkModelRequest, OperationStatus, SourceManifestEntry, VerifyModelRequest, VerifyReport,
    E_COMPILE_INPUT_INVALID, E_FACET_VALUE_UNDECLARED, PRODUCT_SCHEMA_VERSION,
};

#[path = "temp_dirs.rs"]
mod temp_dirs;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The clean shape every negative below perturbs: a declared facet `tier` with
/// a default arm, and ONE parameter that declares itself its handle. The
/// parameter authors no value — its value is the facet's.
const BOUND_OK: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold", "platinum"], "default": "basic" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": {
                    "type": "string",
                    "facet": "tier",
                    "lifecycle": "runtime",
                    "safety": "q_m",
                    "access": "technician"
                }
            }
        }
    }
}"#;

/// Rule 1: the model declares no `tier`, as a facet or as a binding.
const UNDECLARED_FACET: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string", "facet": "tier" }
            }
        }
    }
}"#;

/// Rule 2: a facet value is a symbol token, so an `integer` handle could never
/// carry one.
const WRONG_TYPE: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "integer", "facet": "tier" }
            }
        }
    }
}"#;

/// Rule 3, top-level half: an authored `value` beside the binding would be a
/// second source of truth for the same fact.
const OWN_VALUE: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string", "facet": "tier", "value": "gold" }
            }
        }
    }
}"#;

/// Rule 3, overrides half (a): a conditional `value`. CUE cannot express this —
/// `#ConditionalBlock` closes `value` off for a bound parameter's variant only
/// through the Rust rule — but `compile --source` ingests JSON directly and
/// never evaluates CUE, so the refusal has to live here too.
const OVERRIDE_SETS_VALUE: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" },
        "site": { "values": ["eu", "us"], "default": "eu" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": {
                    "type": "string",
                    "facet": "tier",
                    "overrides": [{ "condition": "site == 'us'", "value": "gold" }]
                }
            }
        }
    }
}"#;

/// Rule 3, overrides half (b): a conditional REBINDING. `#ConditionalBlock`
/// refuses `facet` in CUE; this is the JSON-direct twin.
const OVERRIDE_SETS_FACET: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" },
        "site": { "values": ["eu", "us"], "default": "eu" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": {
                    "type": "string",
                    "facet": "tier",
                    "overrides": [{ "condition": "site == 'us'", "facet": "site" }]
                }
            }
        }
    }
}"#;

/// The narrowing the challenger's 2026-09-18 review forced: an `overrides`
/// entry that varies only fields a binding leaves alone stays LEGAL. A bound
/// parameter may still tighten its limits, or change its safety class, under a
/// condition — the binding and the value it carries are what must stay pinned.
const OVERRIDE_VARIES_OTHER_FIELDS: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" },
        "site": { "values": ["eu", "us"], "default": "eu" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": {
                    "type": "string",
                    "facet": "tier",
                    "safety": "q_m",
                    "overrides": [
                        { "condition": "site == 'us'", "safety": "sil2", "doc": "stricter in us" }
                    ]
                }
            }
        }
    }
}"#;

/// Rule 4, chunk A: declares the facet and binds it.
const DUPLICATE_CHUNK_A: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" }
    },
    "components": {
        "alpha": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string", "facet": "tier" }
            }
        }
    }
}"#;

/// Rule 4, chunk B: a SECOND handle for the same facet, in another chunk. Each
/// chunk is legal alone; together they re-create the many-to-one mapping this
/// whole change exists to make impossible.
const DUPLICATE_CHUNK_B: &str = r#"{
    "package": "facet_binding",
    "version": "1.0.0",
    "components": {
        "beta": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string", "facet": "tier" }
            }
        }
    }
}"#;

/// The propagation fixture: an OPEN facet so an explicit choice may name a
/// value outside the declared arms, with a default arm so the third resolve has
/// something to fall back to.
const PROPAGATION: &str = r#"{
    "package": "facet_propagation",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold", "platinum"], "default": "basic" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": {
                    "type": "string",
                    "facet": "tier",
                    "lifecycle": "runtime",
                    "safety": "q_m",
                    "access": "technician"
                }
            }
        }
    }
}"#;

/// An OPEN facet with NO default: nothing selects it, nothing defaults it, so
/// the handle has nothing to carry.
const NO_EFFECTIVE_VALUE: &str = r#"{
    "package": "facet_no_value",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "open": true }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string", "facet": "tier" }
            }
        }
    }
}"#;

/// The control for the refusal above: a parameter with no binding and no
/// authored value. It is the fault a bound facet with no effective value IS —
/// the handle has nothing to carry — so the two must be refused identically.
const UNVALUED_UNBOUND: &str = r#"{
    "package": "facet_no_value",
    "version": "1.0.0",
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string" }
            }
        }
    }
}"#;

/// The byte-identity control: the same model shape with no binding anywhere.
const BINDING_FREE: &str = r#"{
    "package": "facet_binding_free",
    "version": "1.0.0",
    "facets": {
        "tier": { "values": ["basic", "gold"], "default": "basic" }
    },
    "components": {
        "svc": {
            "type": "service",
            "params": {
                "tier_handle": { "type": "string", "value": "basic" }
            }
        }
    }
}"#;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn tempdir_for(label: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-facet-binding", label)
}

fn sources(chunks: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    chunks
        .iter()
        .map(|(source_id, content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        })
        .collect()
}

fn verify(chunks: &[(&str, &str)]) -> VerifyReport {
    verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: sources(chunks),
    })
}

fn compile(label: &str, chunks: &[(&str, &str)]) -> (CompileResult, PathBuf) {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: sources(chunks),
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    (result, output_dir)
}

fn sole_diagnostic(report: &VerifyReport, case: &str) -> Diagnostic {
    assert_eq!(
        report.status,
        OperationStatus::Error,
        "{case}: the model must be refused"
    );
    let diags = &report.diagnostics.diagnostics;
    assert_eq!(diags.len(), 1, "{case}: expected one diagnostic, got {diags:?}");
    diags[0].clone()
}

/// The load-bearing assertion of this file: `verify` and `compile` refuse the
/// same model, with the same code and the same remedy, and the remedy is the
/// four-rule one.
///
/// One helper rather than two tests, because the claim is that the two AGREE —
/// a pair of separate assertions can both pass while the answers differ.
fn refused_by_both(label: &str, chunks: &[(&str, &str)], expected_code: &str) -> Diagnostic {
    let verified = sole_diagnostic(&verify(chunks), &format!("{label}/verify"));

    let (compiled, output_dir) = compile(label, chunks);
    let from_compile = sole_diagnostic(&compiled.verify_report, &format!("{label}/compile"));
    assert!(
        compiled.compiled_model_package_ref.is_none(),
        "{label}: a refused compile must emit no package"
    );
    fs::remove_dir_all(&output_dir).ok();

    assert_eq!(
        verified.code, expected_code,
        "{label}: unexpected code for message '{}'",
        verified.message
    );
    assert_eq!(
        (from_compile.code.as_str(), from_compile.message.as_str()),
        (verified.code.as_str(), verified.message.as_str()),
        "{label}: verify and compile must refuse identically"
    );
    assert_eq!(
        from_compile.hint, verified.hint,
        "{label}: verify and compile must carry the same remedy"
    );

    let hint = verified
        .hint
        .clone()
        .unwrap_or_else(|| panic!("{label}: a binding refusal must carry the binding remedy"));
    assert!(
        hint.contains("four rules hold together")
            && hint.contains("at most one parameter in the whole model may bind a given facet"),
        "{label}: the remedy must state the whole rule set, got: {hint}"
    );
    verified
}

fn accepted_by_both(label: &str, chunks: &[(&str, &str)]) -> PathBuf {
    let report = verify(chunks);
    assert_eq!(
        report.status,
        OperationStatus::Ok,
        "{label}: verify must accept: {:?}",
        report.diagnostics.diagnostics
    );
    let (compiled, output_dir) = compile(label, chunks);
    assert_eq!(
        compiled.status,
        OperationStatus::Ok,
        "{label}: compile must accept: {:?}",
        compiled.verify_report
    );
    assert!(
        compiled.compiled_model_package_ref.is_some(),
        "{label}: a successful compile must emit a package"
    );
    output_dir
}

struct Compiled {
    output_dir: PathBuf,
    handle: ModelHandle,
}

fn open(label: &str, source: &str) -> Compiled {
    let output_dir = tempdir_for(label);
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: sources(&[("00_model.json", source)]),
        output_dir: Some(output_dir.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "{label}: compile must succeed: {:?}",
        result.verify_report
    );
    let opened = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: result
            .compiled_model_package_ref
            .clone()
            .expect("cmp manifest ref"),
    });
    assert_eq!(opened.status, OperationStatus::Ok, "{label}: open_model ok");
    Compiled {
        output_dir,
        handle: opened.model_handle.expect("model handle"),
    }
}

fn empty_state(handle: &ModelHandle) -> SelectionState {
    let init = initialize_selection_state(InitializeSelectionStateRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        context_tags: BTreeMap::new(),
    });
    assert_eq!(
        init.status,
        OperationStatus::Ok,
        "init selection state: {:?}",
        init.diagnostics
    );
    init.selection_state.expect("selection_state")
}

fn with_choice(handle: &ModelHandle, facet: &str, option: &str) -> SelectionState {
    let applied = apply_selection(ApplySelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: empty_state(handle),
        selection_delta: SelectionDelta {
            facet: facet.to_string(),
            option: option.to_string(),
        },
    });
    assert_eq!(
        applied.status,
        OperationStatus::Ok,
        "apply_selection {facet}={option}: {:?}",
        applied.diagnostics
    );
    applied.selection_state.expect("selection_state")
}

fn resolve(
    handle: &ModelHandle,
    state: &SelectionState,
    implied: BTreeMap<String, String>,
) -> ResolveResult {
    resolve_from_selection(ResolveFromSelectionRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_handle: handle.clone(),
        scope: "all".to_string(),
        selection_state: state.clone(),
        implied_choices: implied,
    })
}

/// The resolved parameter object at `all/svc/tier_handle`.
fn resolved_handle(result: &ResolveResult) -> &serde_json::Value {
    let output = result
        .resolved_output
        .as_ref()
        .unwrap_or_else(|| panic!("resolve produced no output: {:?}", result.diagnostics));
    &output["all"]["components"]["svc"]["params"]["tier_handle"]
}

fn implied(facet: &str, option: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert(facet.to_string(), option.to_string());
    map
}

// ---------------------------------------------------------------------------
// T1 — the four declaration rules (ADR-0064 D2, acceptance (a))
// ---------------------------------------------------------------------------

#[test]
fn a_binding_to_an_undeclared_facet_is_refused_by_verify_and_compile() {
    let diagnostic = refused_by_both(
        "undeclared-facet",
        &[("00_model.json", UNDECLARED_FACET)],
        E_FACET_VALUE_UNDECLARED,
    );
    assert!(
        diagnostic.message.contains("component.svc.param.tier_handle")
            && diagnostic.message.contains("'tier'"),
        "the message must name the parameter path and the facet: {}",
        diagnostic.message
    );
}

#[test]
fn a_non_string_bound_parameter_is_refused_by_verify_and_compile() {
    let diagnostic = refused_by_both(
        "wrong-type",
        &[("00_model.json", WRONG_TYPE)],
        E_COMPILE_INPUT_INVALID,
    );
    assert!(
        diagnostic.message.contains("integer") && diagnostic.message.contains("'string'"),
        "the message must name the declared type and the required one: {}",
        diagnostic.message
    );
}

#[test]
fn a_bound_parameter_that_authors_its_own_value_is_refused_by_verify_and_compile() {
    let diagnostic = refused_by_both(
        "own-value",
        &[("00_model.json", OWN_VALUE)],
        E_COMPILE_INPUT_INVALID,
    );
    assert!(
        diagnostic.message.contains("`value`"),
        "the message must name the field that may not be authored: {}",
        diagnostic.message
    );
}

#[test]
fn an_overrides_entry_setting_value_or_facet_is_refused_by_verify_and_compile() {
    // The JSON-direct path is the one that needs this: CUE's `#ConditionalBlock`
    // closes `facet` off, but `compile --source` never evaluates CUE.
    let by_value = refused_by_both(
        "override-value",
        &[("00_model.json", OVERRIDE_SETS_VALUE)],
        E_COMPILE_INPUT_INVALID,
    );
    assert!(
        by_value.message.contains("`overrides`") && by_value.message.contains("`value`"),
        "the message must name the override and the field: {}",
        by_value.message
    );

    let by_facet = refused_by_both(
        "override-facet",
        &[("00_model.json", OVERRIDE_SETS_FACET)],
        E_COMPILE_INPUT_INVALID,
    );
    assert!(
        by_facet.message.contains("`overrides`") && by_facet.message.contains("`facet`"),
        "the message must name the override and the field: {}",
        by_facet.message
    );
}

#[test]
fn an_overrides_entry_varying_other_fields_keeps_the_binding_legal() {
    // The challenger's narrowing, pinned: refusing `overrides` wholesale would
    // remove a real modelling capability (conditional limits/safety on a bound
    // parameter) that the "second source of truth" rationale never asked for.
    let output_dir = accepted_by_both(
        "override-other-fields",
        &[("00_model.json", OVERRIDE_VARIES_OTHER_FIELDS)],
    );
    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn two_parameters_binding_one_facet_across_chunks_name_both_paths() {
    let diagnostic = refused_by_both(
        "duplicate-binder",
        &[
            ("00_alpha.json", DUPLICATE_CHUNK_A),
            ("10_beta.json", DUPLICATE_CHUNK_B),
        ],
        E_COMPILE_INPUT_INVALID,
    );
    assert!(
        diagnostic.message.contains("component.alpha.param.tier_handle")
            && diagnostic.message.contains("component.beta.param.tier_handle"),
        "the message must name BOTH binders: {}",
        diagnostic.message
    );
}

#[test]
fn a_clean_binding_compiles() {
    // The positive control every negative above is one edit away from: guards
    // against a fix that over-corrects into refusing the ordinary case.
    let output_dir = accepted_by_both("clean-binding", &[("00_model.json", BOUND_OK)]);
    fs::remove_dir_all(&output_dir).ok();
}

// ---------------------------------------------------------------------------
// T1 (cross-unit) — rule 4 is a LINK obligation, not a unit-local one
// ---------------------------------------------------------------------------

#[test]
fn a_second_unit_binding_one_facet_is_refused_at_link_not_at_compile_object() {
    let dir = tempdir_for("cross-unit-binder");

    // Two units, so two package names: a unit is named by its `package`, and
    // reusing one would be caught as a duplicate unit before any rule of this
    // change ran.
    let alpha_src = DUPLICATE_CHUNK_A.replace("\"facet_binding\"", "\"alpha_unit\"");
    let beta_src = DUPLICATE_CHUNK_B.replace("\"facet_binding\"", "\"beta_unit\"");

    // Unit A declares the facet and binds it. Nothing about it is unit-local.
    let alpha = object(&dir, "alpha", &[("00_alpha.json", alpha_src.as_str())]);
    // Unit B binds the SAME facet, compiled against A's interface so the
    // declaredness rule (which does run unit-locally) is satisfied. This is the
    // assertion that matters: `compile-object` ACCEPTS it, because one object
    // sees one unit and cannot know about A's binder.
    let beta = object_against(
        &dir,
        "beta",
        &[("10_beta.json", beta_src.as_str())],
        &[alpha.clone()],
    );

    let out = dir.join("out");
    let result = link_model(LinkModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        object_dirs: vec![alpha, beta],
        output_dir: out.to_string_lossy().into_owned(),
        cluster_size: None,
        budget: None,
        stamp_time: false,
        lock_path: None,
        lock_allow_extra: false,
        write_lock_path: None,
        lock_sources: Default::default(),
        force_lock: false,
    });

    assert_eq!(
        result.status,
        OperationStatus::Error,
        "the link must catch the second binder"
    );
    let diagnostics = &result.verify_report.diagnostics.diagnostics;
    assert_eq!(diagnostics.len(), 1, "expected one diagnostic: {diagnostics:?}");
    assert_eq!(diagnostics[0].code, E_COMPILE_INPUT_INVALID);
    assert!(
        diagnostics[0].message.contains("component.alpha.param.tier_handle")
            && diagnostics[0].message.contains("component.beta.param.tier_handle"),
        "the link message must name both binders: {}",
        diagnostics[0].message
    );
    assert!(
        !out.join("cmp.manifest.json").exists(),
        "a refused link must write no package"
    );

    fs::remove_dir_all(&dir).ok();
}

fn object(dir: &Path, name: &str, chunks: &[(&str, &str)]) -> String {
    object_against(dir, name, chunks, &[])
}

fn object_against(
    dir: &Path,
    name: &str,
    chunks: &[(&str, &str)],
    interfaces: &[String],
) -> String {
    let out = dir.join(format!("{name}.cfo")).to_string_lossy().into_owned();
    let headers = interfaces
        .iter()
        .map(|path| {
            compiler::object::ObjectHeader::read_from_dir(Path::new(path)).expect("interface reads")
        })
        .collect();
    compile_object(CompileObjectRequest {
        sources: sources(chunks),
        interfaces: headers,
        output_dir: out.clone(),
        stamp_time: false,
    })
    .unwrap_or_else(|diagnostic| panic!("compile-object '{name}' failed: {diagnostic:?}"));
    out
}

// ---------------------------------------------------------------------------
// T2 — propagation and its precedence (ADR-0064 D3, acceptance (b))
// ---------------------------------------------------------------------------

#[test]
fn a_bound_parameter_takes_the_explicit_choice_then_the_implied_one_then_the_default() {
    let compiled = open("propagation", PROPAGATION);

    // 1. Explicit choice. Passed ALONGSIDE a conflicting implied choice, so the
    //    assertion sees the precedence and not merely the value.
    let explicit = resolve(
        &compiled.handle,
        &with_choice(&compiled.handle, "tier", "platinum"),
        implied("tier", "gold"),
    );
    assert_eq!(
        explicit.status,
        OperationStatus::Ok,
        "explicit-choice resolve: {:?}",
        explicit.diagnostics
    );
    assert_eq!(
        resolved_handle(&explicit)["value"],
        serde_json::json!("platinum"),
        "an explicit choice must outrank an implied one"
    );
    assert_eq!(
        resolved_handle(&explicit)["type"],
        serde_json::json!("string"),
        "a bound parameter resolves as a string"
    );
    assert_eq!(
        resolved_handle(&explicit)["facet"],
        serde_json::json!("tier"),
        "the resolved parameter carries the binding to the runtime"
    );

    // 2. Implied choice only — above the declared default, below anything the
    //    user stated.
    let implied_only = resolve(
        &compiled.handle,
        &empty_state(&compiled.handle),
        implied("tier", "gold"),
    );
    assert_eq!(
        implied_only.status,
        OperationStatus::Ok,
        "implied-choice resolve: {:?}",
        implied_only.diagnostics
    );
    assert_eq!(
        resolved_handle(&implied_only)["value"],
        serde_json::json!("gold"),
        "an implied choice must outrank the declared default"
    );

    // 3. Nothing stated at all: the declared default arm.
    let defaulted = resolve(
        &compiled.handle,
        &empty_state(&compiled.handle),
        BTreeMap::new(),
    );
    assert_eq!(
        defaulted.status,
        OperationStatus::Ok,
        "default resolve: {:?}",
        defaulted.diagnostics
    );
    assert_eq!(
        resolved_handle(&defaulted)["value"],
        serde_json::json!("basic"),
        "with nothing selected the handle carries the declared default"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

#[test]
fn a_bound_facet_with_no_effective_value_refuses_the_resolve() {
    // An open facet with no default and no selection leaves the handle with
    // nothing to carry, which is the fault an unvalued parameter already has.
    // Asserted as a PAIR — the same code, the same message — because the claim
    // is that the binding reuses that refusal rather than inventing a second
    // vocabulary for the same "this parameter has no value".
    let bound = open("no-effective-value", NO_EFFECTIVE_VALUE);
    let refused = resolve(&bound.handle, &empty_state(&bound.handle), BTreeMap::new());
    let unbound = open("unvalued-unbound", UNVALUED_UNBOUND);
    let control = resolve(
        &unbound.handle,
        &empty_state(&unbound.handle),
        BTreeMap::new(),
    );

    for (label, result) in [("bound", &refused), ("control", &control)] {
        assert_eq!(
            result.status,
            OperationStatus::Error,
            "{label}: a parameter with no value must fail the resolve"
        );
        let diagnostic = result
            .diagnostics
            .diagnostics
            .first()
            .expect("a diagnostic is present");
        assert_eq!(
            diagnostic.code, E_RESOLVE_FAILED,
            "{label}: unexpected code for '{}'",
            diagnostic.message
        );
        assert!(
            result.resolved_output.is_none(),
            "{label}: a refused resolve emits no output"
        );
    }
    assert_eq!(
        refused.diagnostics.diagnostics[0].message,
        control.diagnostics.diagnostics[0].message,
        "a bound facet with no effective value must be refused exactly as an unvalued \
         parameter is"
    );

    fs::remove_dir_all(&bound.output_dir).ok();
    fs::remove_dir_all(&unbound.output_dir).ok();
}

// ---------------------------------------------------------------------------
// T3 — an unbound parameter is byte-invisible (ADR-0064 D4, acceptance (c))
// ---------------------------------------------------------------------------

#[test]
fn a_binding_free_model_serializes_no_facet_key() {
    // `skip_serializing_if` is what keeps every golden, every `s1`-`s5`
    // byte-stability baseline and every `resolve_hash` of a model without
    // bindings untouched by this change. The corpus-wide pin is
    // `scenario_byte_stability_test`; this is the direct assertion that the key
    // is absent rather than present-and-null.
    let compiled = open("binding-free", BINDING_FREE);
    let result = resolve(
        &compiled.handle,
        &empty_state(&compiled.handle),
        BTreeMap::new(),
    );
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "binding-free resolve: {:?}",
        result.diagnostics
    );

    let handle = resolved_handle(&result);
    assert!(
        handle.get("facet").is_none(),
        "an unbound resolved parameter must carry no facet key: {handle}"
    );
    let serialized = serde_json::to_string(&result).expect("serialize resolve result");
    assert!(
        !serialized.contains("\"facet\""),
        "a binding-free resolve envelope must carry no facet key anywhere"
    );

    fs::remove_dir_all(&compiled.output_dir).ok();
}

// ---------------------------------------------------------------------------
// The compile-time read (configflux-p9ug)
// ---------------------------------------------------------------------------

/// `inspect parameter` over the chunks alone — no compile, no environment.
/// This is the read an author has while the model is still being written, and
/// the one the CLI's `inspect parameter <component> <key>` performs.
fn inspect_parameter(component_id: &str, param_key: &str, chunk: &str) -> InspectionResult {
    inspect_model(InspectModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: sources(&[("chunks/00_model.json", chunk)]),
        query: InspectQuery::Parameter {
            component_id: component_id.to_string(),
            param_key: param_key.to_string(),
        },
    })
}

/// D4 gave the RUNTIME read the binding, and the compile-time read was outside
/// that change. A handle authors no `value` — rule 3 refuses one — so without
/// the facet name the inspect item shows a parameter with no value and no
/// account of where its value will come from. The facet name IS that account.
#[test]
fn inspect_parameter_names_the_declared_facet() {
    let result = inspect_parameter("svc", "tier_handle", BOUND_OK);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "bound inspect: {:?}",
        result.diagnostics
    );

    match &result.item {
        Some(InspectionItem::Parameter { facet, value, .. }) => {
            assert_eq!(
                facet.as_deref(),
                Some("tier"),
                "the item must name the facet this parameter is the handle for"
            );
            assert!(
                value.is_none(),
                "a handle authors no value, which is why the facet has to be on the item"
            );
        }
        other => panic!("expected a parameter item, got {other:?}"),
    }

    let envelope = serde_json::to_value(&result).expect("serialize inspection result");
    let item = envelope
        .get("item")
        .unwrap_or_else(|| panic!("no item in {envelope}"));
    assert_eq!(
        item.get("facet").and_then(|value| value.as_str()),
        Some("tier"),
        "the serialized item must carry the binding: {item}"
    );
}

/// The byte-identity half: an unbound parameter is exactly what it was before
/// the field existed. Every committed inspect golden is of an unbound
/// parameter, so this is the property that leaves them untouched.
#[test]
fn inspect_parameter_omits_the_facet_key_for_an_unbound_parameter() {
    let result = inspect_parameter("svc", "tier_handle", BINDING_FREE);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "unbound inspect: {:?}",
        result.diagnostics
    );

    match &result.item {
        Some(InspectionItem::Parameter { facet, .. }) => assert!(
            facet.is_none(),
            "a parameter that declares no binding has no facet, got {facet:?}"
        ),
        other => panic!("expected a parameter item, got {other:?}"),
    }

    let serialized = serde_json::to_string(&result).expect("serialize inspection result");
    assert!(
        !serialized.contains("\"facet\""),
        "an unbound inspect envelope must carry no facet key anywhere: {serialized}"
    );
}
