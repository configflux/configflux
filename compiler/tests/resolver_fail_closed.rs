// SPDX-License-Identifier: BUSL-1.1

//! configflux-ok46: fail-closed assertion at the resolve-time safety/lifecycle/
//! access sink.
//!
//! `configflux-qofj` closed the TOML route to the silent safety-default sink in
//! `resolver.rs` (safety=QM / lifecycle=Runtime / access=Technician via
//! `unwrap_or`). The JSON ingest path (`add_chunk_json_with_source` /
//! `add_chunk_auto`) shares that sink and is intentionally unguarded because the
//! ADR-0027 Decision 8 invariant is that JSON chunks originate from CUE
//! whole-pack export, which bakes the parent definition's declared
//! safety/lifecycle/access into the emitted chunk.
//!
//! These black-box tests pin Option 1 (the decided fix): a parameter that
//! carries `inherits` and drops a metadata field its parent definition declared
//! must fail closed with a deterministic diagnostic instead of silently
//! defaulting. They are driven entirely through the public `resolve()` API.
//!
//! The over-broad-assertion guards (artifact-slot inheritor; standalone/root
//! param) prove the assertion does NOT fire for legitimate CUE-origin shapes,
//! which is what keeps the scenario / byte-stability / cross-path equivalence
//! corpora green.

use std::collections::HashMap;

use compiler::resolver::{resolve, ResolutionContext};
use compiler::schema::{
    Artifact, Component, Config, Lifecycle, Parameter, Role, SafetyLevel, Value,
};

fn empty_param() -> Parameter {
    Parameter {
        inherits: None,
        r#type: None,
        unit: None,
        doc: None,
        value: None,
        lifecycle: None,
        safety: None,
        access: None,
        limits: None,
        req_id: None,
        overrides: Vec::new(),
    }
}

fn no_tags() -> ResolutionContext {
    ResolutionContext {
        tags: HashMap::new(),
    }
}

fn single_param_config(
    definitions: HashMap<String, Parameter>,
    param_name: &str,
    param: Parameter,
    artifacts: HashMap<String, Artifact>,
) -> Config {
    let mut params = HashMap::new();
    params.insert(param_name.to_string(), param);
    let comp = Component {
        r#type: Some("actuator".to_string()),
        condition: None,
        depends_on: Vec::new(),
        params,
    };
    let mut components = HashMap::new();
    components.insert("motor".to_string(), comp);
    Config {
        package: "test".to_string(),
        version: "1.0".to_string(),
        definitions,
        components,
        artifacts,
    }
}

/// A SIL-rated `float` definition that declares all three metadata fields, so a
/// child inheriting from it is expected (post-CUE) to carry them too.
fn sil_rated_definition() -> HashMap<String, Parameter> {
    let mut def = empty_param();
    def.r#type = Some("float".to_string());
    def.value = Some(Value::Float(2.0));
    def.safety = Some(SafetyLevel::Sil3);
    def.lifecycle = Some(Lifecycle::Startup);
    def.access = Some(Role::Supervisor);
    let mut definitions = HashMap::new();
    definitions.insert("base_speed".to_string(), def);
    definitions
}

/// configflux-y11i: a TWO-LEVEL definition chain that CUE export never emits but
/// is structurally valid in hand-crafted JSON:
///   grandparent definition declares safety=sil3 (+ lifecycle/access);
///   intermediate definition {inherits: grandparent} OMITS all three;
///   (the inheriting child param is supplied by the caller).
/// The one-level guard only inspects the immediate parent (`intermediate`),
/// whose metadata is `None`, so the transitive drop must be caught by walking
/// the definition->definition `inherits` chain up to the grandparent.
fn two_level_sil_chain() -> HashMap<String, Parameter> {
    let mut grandparent = empty_param();
    grandparent.r#type = Some("float".to_string());
    grandparent.value = Some(Value::Float(2.0));
    grandparent.safety = Some(SafetyLevel::Sil3);
    grandparent.lifecycle = Some(Lifecycle::Startup);
    grandparent.access = Some(Role::Supervisor);

    // Intermediate inherits the grandparent but declares NONE of the metadata
    // itself (the gap the resolver must see through).
    let mut intermediate = empty_param();
    intermediate.r#type = Some("float".to_string());
    intermediate.inherits = Some("base_speed".to_string());

    let mut definitions = HashMap::new();
    definitions.insert("base_speed".to_string(), grandparent);
    definitions.insert("mid_speed".to_string(), intermediate);
    definitions
}

fn float_param_with_value(value: f64) -> Parameter {
    let mut param = empty_param();
    param.r#type = Some("float".to_string());
    param.value = Some(Value::Float(value));
    param
}

/// The missing-fields list is rendered inside `[..]`; extract it so assertions
/// can target the dropped fields specifically (the prose body also names
/// safety/lifecycle/access when describing the default it refuses to apply).
fn bracketed_fields(rendered: &str) -> String {
    rendered
        .split_once('[')
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list.to_string())
        .expect("diagnostic should contain a [missing-fields] list")
}

#[test]
fn resolve_fails_closed_when_inherits_present_but_all_metadata_missing() {
    // inherits set + type/value authored, but safety/lifecycle/access NOT baked
    // in. This is the silent-downgrade footgun; it must now error.
    let mut child = float_param_with_value(1.0);
    child.inherits = Some("base_speed".to_string());

    let config = single_param_config(sil_rated_definition(), "speed", child, HashMap::new());
    let err = resolve(config, &no_tags()).unwrap_err();
    let rendered = format!("{err:#}");

    assert!(
        rendered.contains("inherits"),
        "diagnostic should mention inherits: {rendered}"
    );
    assert_eq!(
        bracketed_fields(&rendered),
        "safety, lifecycle, access",
        "all three dropped fields should be listed in a fixed, deterministic order: {rendered}"
    );
    assert!(
        rendered.contains("CUE"),
        "diagnostic should point the author at CUE authoring: {rendered}"
    );
    assert!(
        rendered.contains("speed"),
        "diagnostic should name the offending parameter: {rendered}"
    );
}

#[test]
fn resolve_fails_closed_when_inherits_present_but_only_access_missing() {
    // safety + lifecycle baked in, but access omitted: still a dropped field
    // the parent declares, so it must fail closed naming only `access`.
    let mut child = float_param_with_value(1.0);
    child.inherits = Some("base_speed".to_string());
    child.safety = Some(SafetyLevel::Sil2);
    child.lifecycle = Some(Lifecycle::Construction);

    let config = single_param_config(sil_rated_definition(), "speed", child, HashMap::new());
    let err = resolve(config, &no_tags()).unwrap_err();
    let rendered = format!("{err:#}");

    assert_eq!(
        bracketed_fields(&rendered),
        "access",
        "only the dropped `access` field should be listed: {rendered}"
    );
}

#[test]
fn resolve_inherits_with_all_metadata_baked_in_resolves_cleanly() {
    // The legitimate CUE-origin value-bearing shape: inherits carried verbatim
    // AND safety/lifecycle/access baked in. Must resolve and preserve them.
    let mut child = float_param_with_value(1.0);
    child.inherits = Some("base_speed".to_string());
    child.safety = Some(SafetyLevel::Sil3);
    child.lifecycle = Some(Lifecycle::Startup);
    child.access = Some(Role::Supervisor);

    let config = single_param_config(sil_rated_definition(), "speed", child, HashMap::new());
    let resolved = resolve(config, &no_tags()).unwrap();
    let param = &resolved.components["motor"].params["speed"];

    assert_eq!(param.safety, SafetyLevel::Sil3);
    assert_eq!(param.lifecycle, Lifecycle::Startup);
    assert_eq!(param.access, Role::Supervisor);
}

#[test]
fn resolve_inherits_from_artifact_slot_without_safety_keeps_defaults() {
    // The exact legitimate CUE shape that must NOT trip the assertion (it is why
    // the scenario corpora stay green): an artifact param inheriting from an
    // artifact-slot definition that declares lifecycle+access but no `safety`
    // (e.g. s1's `driver_slot`). The child legitimately omits `safety`; it must
    // resolve to the QM default rather than failing closed, since the parent
    // never declared safety to bake in.
    let mut def = empty_param();
    def.r#type = Some("artifact".to_string());
    def.lifecycle = Some(Lifecycle::Construction);
    def.access = Some(Role::Developer);
    // def.safety intentionally None — artifact slots are not safety-rated.
    let mut definitions = HashMap::new();
    definitions.insert("driver_slot".to_string(), def);

    let mut child = empty_param();
    child.inherits = Some("driver_slot".to_string());
    child.r#type = Some("artifact".to_string());
    child.value = Some(Value::String("some_driver".to_string()));
    child.lifecycle = Some(Lifecycle::Construction);
    child.access = Some(Role::Developer);
    // child.safety None, matching the parent — legitimate.

    let mut artifacts = HashMap::new();
    artifacts.insert(
        "some_driver".to_string(),
        Artifact {
            name: "some_driver".to_string(),
            version: None,
            hash: None,
            source: Some("artifact://some_driver".to_string()),
            target: None,
            doc: None,
        },
    );

    let config = single_param_config(definitions, "control_driver", child, artifacts);
    let resolved = resolve(config, &no_tags()).unwrap();
    let param = &resolved.components["motor"].params["control_driver"];

    assert_eq!(param.safety, SafetyLevel::QM);
    assert_eq!(param.lifecycle, Lifecycle::Construction);
    assert_eq!(param.access, Role::Developer);
}

#[test]
fn resolve_non_inheriting_param_keeps_defaults_when_fields_missing() {
    // Guard against an over-broad assertion: a standalone/root param that omits
    // safety/lifecycle/access and does NOT inherit must STILL resolve to the
    // QM/Runtime/Technician defaults (existing fixtures rely on it).
    let child = float_param_with_value(1.0);

    let config = single_param_config(HashMap::new(), "speed", child, HashMap::new());
    let resolved = resolve(config, &no_tags()).unwrap();
    let param = &resolved.components["motor"].params["speed"];

    assert_eq!(param.safety, SafetyLevel::QM);
    assert_eq!(param.lifecycle, Lifecycle::Runtime);
    assert_eq!(param.access, Role::Technician);
}

// ---------------------------------------------------------------------------
// configflux-y11i: transitive (multi-level) definition-chain hardening.
//
// ok46 closed the one-level case (child param vs IMMEDIATE parent definition).
// These tests pin the transitive case: the child's immediate parent is itself
// an intermediate definition whose own `inherits` points at a grandparent that
// declares the metadata. Walking the definition->definition chain must surface
// the grandparent's declared field so a transitive drop ALSO fails closed —
// while a legitimate one-level CUE shape (no definition-level chain) stays a
// no-op, and a cyclic definition chain terminates safely rather than hanging.
// ---------------------------------------------------------------------------

#[test]
fn resolve_fails_closed_when_transitive_parent_declares_dropped_safety() {
    // child param -> intermediate def -> grandparent def(safety=sil3,...).
    // intermediate.safety is None, so the one-level guard would miss it; the
    // chain walk must catch the transitive drop and fail closed, naming the
    // FULL chain (param path, intermediate, grandparent) deterministically.
    let mut child = float_param_with_value(1.0);
    child.inherits = Some("mid_speed".to_string());

    let config = single_param_config(two_level_sil_chain(), "speed", child, HashMap::new());
    let err = resolve(config, &no_tags()).unwrap_err();
    let rendered = format!("{err:#}");

    assert_eq!(
        bracketed_fields(&rendered),
        "safety, lifecycle, access",
        "all three transitively-declared fields should be listed in fixed order: {rendered}"
    );
    // The diagnostic must name the full chain (param -> intermediate ->
    // grandparent) so the author can see WHERE the rating originates.
    assert!(
        rendered.contains("speed"),
        "diagnostic should name the offending parameter: {rendered}"
    );
    assert!(
        rendered.contains("mid_speed"),
        "diagnostic should name the intermediate definition in the chain: {rendered}"
    );
    assert!(
        rendered.contains("base_speed"),
        "diagnostic should name the grandparent definition that declares the metadata: {rendered}"
    );
    assert!(
        rendered.contains("CUE"),
        "diagnostic should point the author at CUE authoring: {rendered}"
    );
}

#[test]
fn resolve_transitive_chain_is_cycle_safe_and_does_not_hang() {
    // A cyclic definition chain (a -> b -> a) is structurally possible in
    // hand-crafted JSON and is NOT pre-screened on the resolve path
    // (detect_definition_cycle lives in link_verify, not guaranteed to have
    // run). The chain walk must terminate deterministically — no hang, no
    // unbounded recursion, no stack overflow. None of these definitions
    // declares metadata, so the safe outcome is a clean resolve to the
    // defaults; the load-bearing property is that `resolve` RETURNS.
    let mut def_a = empty_param();
    def_a.r#type = Some("float".to_string());
    def_a.inherits = Some("def_b".to_string());
    let mut def_b = empty_param();
    def_b.r#type = Some("float".to_string());
    def_b.inherits = Some("def_a".to_string());
    let mut definitions = HashMap::new();
    definitions.insert("def_a".to_string(), def_a);
    definitions.insert("def_b".to_string(), def_b);

    let mut child = float_param_with_value(1.0);
    child.inherits = Some("def_a".to_string());

    let config = single_param_config(definitions, "speed", child, HashMap::new());
    // Must not hang / overflow. Deterministic safe outcome: clean defaults,
    // since no definition in the cycle declares any metadata field.
    let resolved = resolve(config, &no_tags()).expect("cyclic chain must resolve, not hang");
    let param = &resolved.components["motor"].params["speed"];
    assert_eq!(param.safety, SafetyLevel::QM);
    assert_eq!(param.lifecycle, Lifecycle::Runtime);
    assert_eq!(param.access, Role::Technician);
}

#[test]
fn resolve_self_referential_definition_chain_is_bounded() {
    // Degenerate self-reference (a -> a): same bounded-termination guarantee.
    // The definition declares safety=sil3 and the child drops it, so the safe
    // outcome is fail-closed (a dropped declared field), reached without
    // looping forever on the self-edge.
    let mut def_a = empty_param();
    def_a.r#type = Some("float".to_string());
    def_a.value = Some(Value::Float(2.0));
    def_a.safety = Some(SafetyLevel::Sil3);
    def_a.inherits = Some("def_a".to_string());
    let mut definitions = HashMap::new();
    definitions.insert("def_a".to_string(), def_a);

    let mut child = float_param_with_value(1.0);
    child.inherits = Some("def_a".to_string());

    let config = single_param_config(definitions, "speed", child, HashMap::new());
    let err = resolve(config, &no_tags()).unwrap_err();
    let rendered = format!("{err:#}");
    assert_eq!(
        bracketed_fields(&rendered),
        "safety",
        "self-referential parent declaring safety must still fail closed on the drop: {rendered}"
    );
}

#[test]
fn resolve_one_level_cue_shape_with_no_definition_chain_still_resolves() {
    // Regression guard for the legitimate CUE-exported shape: the immediate
    // parent definition declares all three metadata fields and has NO
    // definition-level `inherits` of its own (exactly what the corpus emits).
    // The chain walk must be a strict no-op here — the child carries the baked
    // fields verbatim and resolves unchanged. This is what keeps all 11 packs,
    // the testdata corpus, and the byte-stability suites green.
    let mut child = float_param_with_value(1.0);
    child.inherits = Some("base_speed".to_string());
    child.safety = Some(SafetyLevel::Sil3);
    child.lifecycle = Some(Lifecycle::Startup);
    child.access = Some(Role::Supervisor);

    let config = single_param_config(sil_rated_definition(), "speed", child, HashMap::new());
    let resolved = resolve(config, &no_tags()).unwrap();
    let param = &resolved.components["motor"].params["speed"];

    assert_eq!(param.safety, SafetyLevel::Sil3);
    assert_eq!(param.lifecycle, Lifecycle::Startup);
    assert_eq!(param.access, Role::Supervisor);
}
