// SPDX-License-Identifier: BUSL-1.1

//! Component requirement validation (ADR-0057 §D4).
//!
//! Every negative in the CUE harness's `requires` block has a twin here, for
//! the same reason the catalogue tests do: CUE rejects a malformed SHAPE, Rust
//! rejects a requirement that does not RESOLVE. A `requires` block naming a
//! binding no chunk declares is perfectly shaped and completely wrong, and this
//! is where that is caught.

use super::*;
use crate::interface_summary::summarize;
use crate::schema::{
    Binding, Catalogue, CatalogueField, CatalogueFieldType, Component, Config, Requirement, Value,
};
use std::collections::HashMap;

fn config() -> Config {
    Config {
        package: "unit".to_string(),
        version: "1.0.0".to_string(),
        definitions: HashMap::new(),
        components: HashMap::new(),
        artifacts: HashMap::new(),
        facets: HashMap::new(),
        constraints: HashMap::new(),
        catalogues: HashMap::new(),
        bindings: HashMap::new(),
    }
}

/// A three-entry catalogue — the shape §D4's intersection rule needs to have
/// anything to say.
fn containers() -> Catalogue {
    let mut fields = BTreeMap::new();
    fields.insert(
        "width_mm".to_string(),
        CatalogueField {
            r#type: CatalogueFieldType::Integer,
            unit: Some("mm".to_string()),
            doc: None,
        },
    );
    let mut entries = BTreeMap::new();
    for (id, width) in [("c1", 800), ("c2", 600), ("c3", 400)] {
        let mut entry = BTreeMap::new();
        entry.insert("width_mm".to_string(), Value::Integer(width));
        entries.insert(id.to_string(), entry);
    }
    Catalogue {
        fields,
        entries,
        doc: None,
    }
}

/// A model with the catalogue and the binding already in place, so each test
/// below varies only the requirement under examination.
fn bound() -> Config {
    let mut cfg = config();
    cfg.catalogues.insert("containers".to_string(), containers());
    cfg.bindings.insert(
        "line_container".to_string(),
        Binding {
            catalogue: "containers".to_string(),
            default: Some("c1".to_string()),
            derive: None,
            doc: None,
        },
    );
    cfg
}

fn requiring(id: &str, condition: Option<&str>, slot: &str, requirement: Requirement) -> Component {
    let mut requires = BTreeMap::new();
    requires.insert(slot.to_string(), requirement);
    let _ = id;
    Component {
        r#type: Some("service".to_string()),
        condition: condition.map(str::to_string),
        depends_on: Vec::new(),
        requires,
        params: HashMap::new(),
    }
}

fn with_component(mut cfg: Config, id: &str, component: Component) -> Config {
    cfg.components.insert(id.to_string(), component);
    cfg
}

fn accepts(binding: &str, entries: &[&str]) -> Requirement {
    Requirement {
        binding: binding.to_string(),
        accepts: Some(entries.iter().map(|e| e.to_string()).collect()),
    }
}

fn bare(binding: &str) -> Requirement {
    Requirement {
        binding: binding.to_string(),
        accepts: None,
    }
}

fn link(config: &Config) -> Result<()> {
    validate_link_summary(&[summarize(config, "chunk.json")])
}

// --- positives ---------------------------------------------------------------

#[test]
fn a_bare_requirement_over_a_declared_binding_validates() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            bare("line_container"),
        ),
    );
    assert!(link(&cfg).is_ok());
}

#[test]
fn an_accepts_subset_of_the_catalogue_validates() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &["c1", "c2"]),
        ),
    );
    assert!(link(&cfg).is_ok());
}

/// Overlapping — not identical — lists leave the intersection non-empty, which
/// is exactly what §D4 asks for.
#[test]
fn two_requirements_whose_accepts_overlap_validate() {
    let mut cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &["c1", "c2"]),
        ),
    );
    cfg = with_component(
        cfg,
        "vision_service",
        requiring(
            "vision_service",
            None,
            "container",
            accepts("line_container", &["c2", "c3"]),
        ),
    );
    assert!(link(&cfg).is_ok());
}

/// A requirement is NOT a dependency edge (§D4), so it must not have to name a
/// component and must not be reachable by the dependency-cycle guard.
#[test]
fn a_requirement_creates_no_component_dependency() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            bare("line_container"),
        ),
    );
    let summary = summarize(&cfg, "chunk.json");
    assert!(
        summary.imports.components.is_empty(),
        "a requirement must not appear as a component import: {:?}",
        summary.imports.components
    );
    assert!(validate_component_dependencies(&cfg.components).is_ok());
}

// --- E_REQUIRES_INVALID ------------------------------------------------------

#[test]
fn a_requirement_naming_an_unknown_binding_is_rejected() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring("compute_service", None, "container", bare("nowhere")),
    );
    let err = format!("{}", link(&cfg).unwrap_err());
    assert_eq!(
        err,
        "component 'compute_service' requires slot 'container' bound to unknown binding 'nowhere'",
        "err: {err}"
    );
}

#[test]
fn an_empty_accepts_list_is_rejected() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &[]),
        ),
    );
    let err = format!("{}", link(&cfg).unwrap_err());
    assert!(
        err.contains("requires slot 'container' with an empty `accepts` list"),
        "err: {err}"
    );
}

#[test]
fn a_duplicate_entry_in_accepts_is_rejected() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &["c1", "c1"]),
        ),
    );
    let err = format!("{}", link(&cfg).unwrap_err());
    assert!(
        err.contains("accepting entry 'c1' more than once"),
        "err: {err}"
    );
}

#[test]
fn an_accepts_entry_outside_the_catalogue_is_rejected() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &["c1", "c9"]),
        ),
    );
    let err = format!("{}", link(&cfg).unwrap_err());
    assert!(
        err.contains("accepting entry 'c9', which is not an entry of catalogue 'containers'"),
        "err: {err}"
    );
    assert!(err.contains("[c1, c2, c3]"), "the message must name the roster: {err}");
}

/// The `accepts` lowering wraps the conjunct in the component's condition, so
/// an unparseable condition would produce an unparseable conjunct. Refused
/// here, where the message can name the component.
#[test]
fn accepts_on_a_component_whose_condition_does_not_parse_is_rejected() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            Some("this is not <> a condition"),
            "container",
            accepts("line_container", &["c1"]),
        ),
    );
    let err = format!("{}", link(&cfg).unwrap_err());
    assert!(err.contains("does not parse"), "err: {err}");
}

/// ...but an unparseable condition on a component with no `accepts` stays
/// legal: it widens no facet and forms no clause, exactly as before.
#[test]
fn an_unparseable_condition_without_accepts_is_still_accepted() {
    let cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            Some("this is not <> a condition"),
            "container",
            bare("line_container"),
        ),
    );
    assert!(link(&cfg).is_ok());
}

// --- E_BINDING_NO_ACCEPTABLE_ENTRY -------------------------------------------

#[test]
fn two_requirements_whose_accepts_intersect_to_nothing_are_rejected() {
    let mut cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &["c1"]),
        ),
    );
    cfg = with_component(
        cfg,
        "vision_service",
        requiring(
            "vision_service",
            None,
            "container",
            accepts("line_container", &["c2", "c3"]),
        ),
    );
    let err = format!("{}", link(&cfg).unwrap_err());
    assert!(
        err.starts_with("binding 'line_container' has no entry every requirement accepts"),
        "err: {err}"
    );
    // §D4: the message names the binding AND every list, because the author
    // cannot tell which one to widen without seeing them all.
    assert!(
        err.contains("compute_service.container accepts [c1]"),
        "err: {err}"
    );
    assert!(
        err.contains("vision_service.container accepts [c2, c3]"),
        "err: {err}"
    );
}

/// A requirement WITHOUT `accepts` accepts everything, so it is the identity of
/// the intersection and cannot empty it.
#[test]
fn a_bare_requirement_never_empties_the_intersection() {
    let mut cfg = with_component(
        bound(),
        "compute_service",
        requiring(
            "compute_service",
            None,
            "container",
            accepts("line_container", &["c3"]),
        ),
    );
    cfg = with_component(
        cfg,
        "vision_service",
        requiring(
            "vision_service",
            None,
            "container",
            bare("line_container"),
        ),
    );
    assert!(link(&cfg).is_ok());
}
