// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the interface summary (ADR-0057 §D9).
//!
//! The fixture is the three-unit catalogue shape the epic is designed around: a
//! shared `catalogue` unit declaring one table plus two bindings over it, and a
//! `vision` service unit that reads a shared parameter through an override
//! condition. It is the shape ADR-0058's linker will see as three object
//! headers, which is exactly why the summary is asserted on it here.

use super::*;
use crate::schema::{
    Artifact, Binding, Catalogue, CatalogueField, CatalogueFieldType, Component,
    ConditionalBlock, Config, Constraint, Facet, Parameter, Requirement, Value,
};
use std::collections::HashMap;

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
        facet: None,
        overrides: Vec::new(),
    }
}

pub(crate) fn config(package: &str) -> Config {
    Config {
        package: package.to_string(),
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

pub(crate) fn container_catalogue() -> Catalogue {
    let field = |t: CatalogueFieldType, unit: Option<&str>| CatalogueField {
        r#type: t,
        unit: unit.map(str::to_string),
        doc: None,
    };
    let entry = |w: i64, h: i64| {
        let mut map = BTreeMap::new();
        map.insert("width_mm".to_string(), Value::Integer(w));
        map.insert("height_mm".to_string(), Value::Integer(h));
        map
    };
    let mut fields = BTreeMap::new();
    fields.insert(
        "width_mm".to_string(),
        field(CatalogueFieldType::Integer, Some("mm")),
    );
    fields.insert(
        "height_mm".to_string(),
        field(CatalogueFieldType::Integer, Some("mm")),
    );
    let mut entries = BTreeMap::new();
    entries.insert("c1".to_string(), entry(800, 1000));
    entries.insert("c2".to_string(), entry(600, 900));
    entries.insert("c3".to_string(), entry(400, 400));
    Catalogue {
        fields,
        entries,
        doc: Some("The containers this plant uses".to_string()),
    }
}

pub(crate) fn site_facet() -> Facet {
    Facet {
        values: vec!["factory_a".to_string(), "factory_b".to_string()],
        default: Some("factory_a".to_string()),
        open: false,
        doc: None,
    }
}

pub(crate) fn derived_binding() -> Binding {
    let mut table = BTreeMap::new();
    let mut pairs = BTreeMap::new();
    pairs.insert("factory_a".to_string(), "c1".to_string());
    pairs.insert("factory_b".to_string(), "c2".to_string());
    table.insert("site".to_string(), pairs);
    Binding {
        catalogue: "containers".to_string(),
        default: None,
        derive: Some(table),
        doc: None,
    }
}

pub(crate) fn defaulted_binding() -> Binding {
    Binding {
        catalogue: "containers".to_string(),
        default: Some("c3".to_string()),
        derive: None,
        doc: None,
    }
}

/// The shared `catalogue` unit: one table, two bindings over it, and the facet
/// one of them derives from.
pub(crate) fn catalogue_unit() -> Config {
    let mut cfg = config("catalogue");
    cfg.catalogues
        .insert("containers".to_string(), container_catalogue());
    cfg.facets.insert("site".to_string(), site_facet());
    cfg.bindings
        .insert("line_container".to_string(), derived_binding());
    cfg.bindings
        .insert("sorter_container".to_string(), defaulted_binding());
    cfg
}

/// The `vision` service unit: one component whose parameter override selects on
/// a facet, and one artifact.
fn vision_unit() -> Config {
    let mut cfg = config("vision");
    let mut param = empty_param();
    param.inherits = Some("width_mm".to_string());
    param.overrides = vec![ConditionalBlock {
        condition: "site == 'factory_b'".to_string(),
        payload: Box::new(empty_param()),
    }];
    let mut params = HashMap::new();
    params.insert("frame_width".to_string(), param);
    // ADR-0057 §D4: the service declares what it NEEDS. The bare form names the
    // binding and accepts every entry.
    let mut requires = BTreeMap::new();
    requires.insert(
        "container".to_string(),
        Requirement {
            binding: "line_container".to_string(),
            accepts: None,
        },
    );
    cfg.components.insert(
        "vision_service".to_string(),
        Component {
            r#type: Some("service".to_string()),
            condition: None,
            depends_on: vec!["power_bus".to_string()],
            requires,
            params,
        },
    );
    // A second consumer, narrowing the SAME binding to the entries it supports.
    let mut narrowed = BTreeMap::new();
    narrowed.insert(
        "container".to_string(),
        Requirement {
            binding: "sorter_container".to_string(),
            accepts: Some(vec!["c2".to_string(), "c3".to_string()]),
        },
    );
    cfg.components.insert(
        "sorter_service".to_string(),
        Component {
            r#type: Some("service".to_string()),
            condition: None,
            depends_on: Vec::new(),
            requires: narrowed,
            params: HashMap::new(),
        },
    );
    cfg.artifacts.insert(
        "vision_driver".to_string(),
        Artifact {
            name: "vision_driver".to_string(),
            version: None,
            hash: None,
            source: None,
            target: None,
            doc: None,
        },
    );
    cfg
}

#[test]
fn the_catalogue_unit_exports_one_catalogue_and_two_bindings() {
    let summary = summarize(&catalogue_unit(), "repos/catalogue/00_catalogue.json");

    assert_eq!(summary.unit, "catalogue");
    assert_eq!(
        summary.exports.catalogues,
        ["containers"].map(str::to_string).into_iter().collect()
    );
    assert_eq!(
        summary.exports.bindings,
        ["line_container", "sorter_container"]
            .map(str::to_string)
            .into_iter()
            .collect()
    );
    // The entry roster is the binding's value domain, in id-ascending order —
    // the only order the wire format carries (see `schema::Catalogue`).
    assert_eq!(
        summary.catalogue_entries.get("containers").unwrap(),
        &vec!["c1".to_string(), "c2".to_string(), "c3".to_string()]
    );
}

#[test]
fn a_binding_records_its_catalogue_default_and_derive_links() {
    let summary = summarize(&catalogue_unit(), "src");

    let derived = summary.binding_links.get("line_container").unwrap();
    assert_eq!(derived.catalogue, "containers");
    assert_eq!(derived.default, None);
    assert_eq!(derived.derive_source.as_deref(), Some("site"));
    assert_eq!(derived.derive_source_count, 1);
    assert_eq!(
        derived.derive_pairs,
        vec![
            ("factory_a".to_string(), "c1".to_string()),
            ("factory_b".to_string(), "c2".to_string()),
        ]
    );

    let defaulted = summary.binding_links.get("sorter_container").unwrap();
    assert_eq!(defaulted.default.as_deref(), Some("c3"));
    assert_eq!(defaulted.derive_source, None);

    // A catalogue is a link obligation; the derive source rides the facet
    // channel, which is header information rather than an obligation.
    assert!(summary
        .imports
        .catalogues
        .get("containers")
        .unwrap()
        .contains("line_container"));
    assert!(summary
        .imports
        .facets
        .get("site")
        .unwrap()
        .contains("bindings.line_container"));
}

#[test]
fn a_service_unit_imports_the_facets_its_overrides_select_on() {
    let summary = summarize(&vision_unit(), "repos/vision/10_vision.json");

    assert_eq!(summary.unit, "vision");
    assert!(summary.exports.components.contains("vision_service"));
    assert!(summary.exports.artifacts.contains("vision_driver"));
    // The override condition names `site`, which the catalogue unit declares.
    assert!(summary.imports.facets.contains_key("site"));
    assert!(summary
        .imports
        .definitions
        .get("width_mm")
        .unwrap()
        .contains("components.vision_service.params.frame_width"));
    assert!(summary
        .imports
        .components
        .get("power_bus")
        .unwrap()
        .contains("vision_service"));
}

/// ADR-0057 §D4: a `requires` block is an import of a BINDING, recorded against
/// the authored site so a link diagnostic can name the offending slot rather
/// than only the missing target.
#[test]
fn a_service_unit_imports_the_bindings_its_requirements_name() {
    let summary = summarize(&vision_unit(), "repos/vision/10_vision.json");

    assert_eq!(
        summary.imports.bindings.keys().collect::<Vec<_>>(),
        vec!["line_container", "sorter_container"]
    );
    assert!(summary
        .imports
        .bindings
        .get("line_container")
        .unwrap()
        .contains("components.vision_service.requires.container"));

    // A requirement is NOT a dependency edge (§D4). `power_bus` is there
    // because `vision_service` genuinely depends_on it; neither binding is.
    assert_eq!(
        summary.imports.components.keys().collect::<Vec<_>>(),
        vec!["power_bus"]
    );
}

/// The requirement roster is what `link_verify::validate_requirements` and the
/// `accepts` lowering both read, so its ORDER is a property of the model:
/// component-then-slot ascending, never the `HashMap` walk order.
#[test]
fn requirements_are_recorded_component_then_slot_ascending() {
    let summary = summarize(&vision_unit(), "repos/vision/10_vision.json");

    let seen: Vec<(&str, &str, &str)> = summary
        .requirements
        .iter()
        .map(|r| (r.component.as_str(), r.slot.as_str(), r.binding.as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("sorter_service", "container", "sorter_container"),
            ("vision_service", "container", "line_container"),
        ]
    );
    assert_eq!(
        summary.requirements[0].accepts.as_deref(),
        Some(["c2".to_string(), "c3".to_string()].as_slice()),
        "the accepted entries are carried in AUTHORED order"
    );
    assert_eq!(summary.requirements[1].accepts, None, "the bare form accepts every entry");
}

#[test]
fn constraints_become_clauses_in_id_ascending_order() {
    let mut cfg = catalogue_unit();
    cfg.constraints.insert(
        "zeta".to_string(),
        Constraint {
            condition: "site != 'factory_b'".to_string(),
            doc: None,
        },
    );
    cfg.constraints.insert(
        "alpha".to_string(),
        Constraint {
            condition: "site != 'factory_a'".to_string(),
            doc: None,
        },
    );

    let summary = summarize(&cfg, "src");
    let ids: Vec<&str> = summary.clauses.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "zeta"]);
}

#[test]
fn merge_records_both_sources_of_a_duplicated_binding() {
    // The one-entity-one-chunk invariant, seen from the linker's side: the
    // merge is TOTAL, so it records the collision rather than rejecting it, and
    // `link_verify::validate_link_summary` is what turns it into a diagnostic.
    let mut second = config("integration");
    second
        .bindings
        .insert("line_container".to_string(), defaulted_binding());

    let merged = merge(&[
        summarize(&catalogue_unit(), "repos/catalogue/00_catalogue.json"),
        summarize(&second, "repos/integration/10_integration.json"),
    ]);

    assert_eq!(
        merged.exports.bindings.get("line_container").unwrap(),
        &vec![
            "repos/catalogue/00_catalogue.json".to_string(),
            "repos/integration/10_integration.json".to_string(),
        ]
    );
    assert_eq!(
        merged.units,
        ["catalogue", "integration"]
            .map(str::to_string)
            .into_iter()
            .collect()
    );
}

#[test]
fn declared_domain_answers_for_a_facet_and_for_a_binding() {
    // ADR-0057 §D3: a `derive` source may be either kind, so one lookup has to
    // answer for both or the validator would need to know the difference.
    let merged = merge(&[summarize(&catalogue_unit(), "src")]);

    assert_eq!(
        merged.declared_domain("site").unwrap(),
        ["factory_a".to_string(), "factory_b".to_string()]
    );
    assert_eq!(
        merged.declared_domain("line_container").unwrap(),
        ["c1".to_string(), "c2".to_string(), "c3".to_string()]
    );
    assert!(merged.declared_domain("nothing_declared").is_none());
}

#[test]
fn a_binding_projects_onto_the_closed_facet_it_is() {
    let cfg = catalogue_unit();
    let facets = binding_facets(&cfg.catalogues, &cfg.bindings);

    let line = facets.get("line_container").expect("binding projected");
    assert_eq!(line.values, vec!["c1", "c2", "c3"]);
    assert!(!line.open, "a binding's domain is exhaustive by construction");
    assert_eq!(line.default, None);
    assert_eq!(
        facets.get("sorter_container").unwrap().default.as_deref(),
        Some("c3")
    );
}

#[test]
fn a_binding_whose_catalogue_is_absent_is_skipped_rather_than_guessed() {
    // The projection runs on models that already passed `E_BINDING_INVALID`, so
    // an unresolvable binding must not fabricate an empty domain — an empty
    // closed facet would silently make every selection unsatisfiable.
    let cfg = catalogue_unit();
    let facets = binding_facets(&BTreeMap::new(), &cfg.bindings);
    assert!(facets.is_empty());
}

#[test]
fn summarize_survives_an_unparseable_condition() {
    // Mirrors `collect_ccm_clauses` and `validate_facets`: a selector that does
    // not parse widens no domain, so it imports nothing either — and it must
    // not abort the summary.
    let mut cfg = config("unit");
    cfg.components.insert(
        "c".to_string(),
        Component {
            r#type: None,
            condition: Some("this is not <> a condition".to_string()),
            depends_on: Vec::new(),
            requires: Default::default(),
            params: HashMap::new(),
        },
    );
    let summary = summarize(&cfg, "src");
    assert!(summary.imports.facets.is_empty());
    assert!(summary.exports.components.contains("c"));
}
