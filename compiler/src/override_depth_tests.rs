// SPDX-License-Identifier: BUSL-1.1

//! configflux-dw9i: the authored `overrides` chain stops at [`MAX_CHAIN_DEPTH`].
//!
//! `ConditionalBlock::payload` is a `Box<Parameter>` carrying its own
//! `overrides`, so an override chain is a self-referential shape walked by
//! plain recursion in three places — `link_verify::collect_parameter_conditions`
//! (reached from `validate_facets`), `link_verify::validate_parameter_floats`
//! (reached from `validate_parameter_values`) and
//! `resolver::apply_overrides_recursive` (reached from `resolve`). None of them
//! carried the ceiling the two sibling `inherits` / `depends_on` DFSes have had
//! since configflux-xowl.5.
//!
//! These tests enter through the three `pub(crate)`/`pub` validators rather
//! than the private walks, because the ceiling is a property of what the
//! compiler accepts, not of how a particular function iterates. They build the
//! chain in memory: no authored chunk can reach depth 1_000, since both parsers
//! refuse far shallower input (see `tests/override_chain_depth.rs` for the
//! measured numbers), so in-memory construction is the only way to exercise the
//! guard at all.
//!
//! Each ceiling has a matching at-limit test. A guard with no boundary proof is
//! indistinguishable from one that rejects a depth the product is supposed to
//! accept.

use std::collections::HashMap;

use crate::link_verify::{validate_facets, validate_parameter_values, MAX_CHAIN_DEPTH};
use crate::resolver::{resolve, ResolutionContext};
use crate::schema::{Component, ConditionalBlock, Config, Facet, Parameter, Value};

/// One condition, reused at every level: the depth guard has to fire on the
/// shape of the chain, never on what any individual condition says.
const CONDITION: &str = "region == 'eu'";

/// The substring the two sibling ceilings already report. A depth refusal
/// carries no diagnostic code of its own, which is how all three report the
/// `E_COMPILE_INPUT_INVALID` the product mappers give an uncoded refusal
/// (configflux-py7w).
const CEILING_MESSAGE: &str = "exceeds the maximum supported depth";

fn bare_parameter() -> Parameter {
    Parameter {
        inherits: None,
        r#type: Some("float".to_string()),
        unit: None,
        doc: None,
        value: None,
        lifecycle: None,
        safety: None,
        access: None,
        limits: None,
        req_id: None,
        facet: None,
        overrides: Vec::new(),
    }
}

/// A parameter whose override chain nests `depth` levels deep.
///
/// Built bottom-up with a loop, not recursion: a recursive builder would
/// overflow before the code under test ever ran. Only the innermost payload
/// carries a value, so `resolve` has exactly one value to find and every level
/// above it contributes a merge.
fn chain(depth: usize) -> Parameter {
    let mut param = bare_parameter();
    param.value = Some(Value::Float(1.0));
    for _ in 0..depth {
        let mut outer = bare_parameter();
        outer.overrides = vec![ConditionalBlock {
            condition: CONDITION.to_string(),
            payload: Box::new(param),
        }];
        param = outer;
    }
    param
}

fn definitions_with_chain(depth: usize) -> HashMap<String, Parameter> {
    let mut definitions = HashMap::new();
    definitions.insert("deep".to_string(), chain(depth));
    definitions
}

/// A closed `region` facet declaring the value every level's condition names,
/// so the at-limit cases reach the end of `validate_facets` on their merits
/// instead of tripping the undeclared-value rule.
fn region_facet() -> HashMap<String, Facet> {
    let mut facets = HashMap::new();
    facets.insert(
        "region".to_string(),
        Facet {
            values: vec!["eu".to_string(), "us".to_string()],
            default: Some("eu".to_string()),
            open: false,
            doc: None,
        },
    );
    facets
}

/// An otherwise-empty model carrying one component whose single parameter has
/// the chain. `serde_json` supplies the seven `#[serde(default)]` namespaces so
/// this stays a statement about the parameter and nothing else.
fn config_with_chain(depth: usize) -> Config {
    let mut config: Config =
        serde_json::from_str(r#"{"package": "p1", "version": "1.0"}"#).expect("empty config");
    let mut params = HashMap::new();
    params.insert("speed".to_string(), chain(depth));
    config.components.insert(
        "motor".to_string(),
        Component {
            r#type: Some("actuator".to_string()),
            condition: None,
            depends_on: Vec::new(),
            requires: Default::default(),
            params,
        },
    );
    config.facets = region_facet();
    config
}

fn eu_context() -> ResolutionContext {
    let mut tags = HashMap::new();
    tags.insert("region".to_string(), "eu".to_string());
    ResolutionContext { tags }
}

#[test]
fn parameter_value_validation_refuses_an_override_chain_past_the_ceiling() {
    let definitions = definitions_with_chain(MAX_CHAIN_DEPTH + 1);
    let err = validate_parameter_values(&definitions, &HashMap::new())
        .expect_err("an override chain one level past the ceiling must be refused");
    let message = format!("{err}");
    assert!(message.contains(CEILING_MESSAGE), "err: {message}");
    assert!(message.contains("definitions.deep"), "err: {message}");
}

#[test]
fn parameter_value_validation_accepts_an_override_chain_at_the_ceiling() {
    let definitions = definitions_with_chain(MAX_CHAIN_DEPTH);
    validate_parameter_values(&definitions, &HashMap::new())
        .expect("an override chain at the ceiling is legitimate and must be accepted");
}

#[test]
fn facet_validation_refuses_an_override_chain_past_the_ceiling() {
    let definitions = definitions_with_chain(MAX_CHAIN_DEPTH + 1);
    let err = validate_facets(&region_facet(), &HashMap::new(), &definitions)
        .expect_err("an override chain one level past the ceiling must be refused");
    let message = format!("{err}");
    assert!(message.contains(CEILING_MESSAGE), "err: {message}");
    assert!(message.contains("deep"), "err: {message}");
}

#[test]
fn facet_validation_accepts_an_override_chain_at_the_ceiling() {
    let definitions = definitions_with_chain(MAX_CHAIN_DEPTH);
    validate_facets(&region_facet(), &HashMap::new(), &definitions)
        .expect("an override chain at the ceiling is legitimate and must be accepted");
}

#[test]
fn resolve_refuses_an_override_chain_past_the_ceiling() {
    let err = resolve(config_with_chain(MAX_CHAIN_DEPTH + 1), &eu_context())
        .expect_err("an override chain one level past the ceiling must be refused");
    let message = format!("{err:#}");
    assert!(message.contains(CEILING_MESSAGE), "err: {message}");
}

#[test]
fn resolve_accepts_an_override_chain_at_the_ceiling() {
    let resolved = resolve(config_with_chain(MAX_CHAIN_DEPTH), &eu_context())
        .expect("an override chain at the ceiling is legitimate and must resolve");
    let speed = &resolved.components["motor"].params["speed"];
    assert_eq!(speed.value, Value::Float(1.0));
}
