// SPDX-License-Identifier: BUSL-1.1

//! Catalogue and binding validation (ADR-0057 §D2/§D3).
//!
//! Every negative in the CUE harness's catalogue/binding block has a twin here,
//! because CUE and Rust deliberately own different halves of the contract: CUE
//! rejects a malformed SHAPE, Rust rejects a table whose entries do not match
//! its own declared fields and a binding that does not resolve. A fixture that
//! passes `cue vet` and is still wrong is exactly what these tests pin.

use super::*;
use crate::interface_summary::summarize;
use crate::schema::{Binding, Catalogue, CatalogueField, CatalogueFieldType, Config, Value};
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

fn field(t: CatalogueFieldType) -> CatalogueField {
    CatalogueField {
        r#type: t,
        unit: None,
        doc: None,
    }
}

/// A catalogue exercising all four declared field types.
fn typed_catalogue() -> Catalogue {
    let mut fields = BTreeMap::new();
    fields.insert("count".to_string(), field(CatalogueFieldType::Integer));
    fields.insert("ratio".to_string(), field(CatalogueFieldType::Float));
    fields.insert("heated".to_string(), field(CatalogueFieldType::Boolean));
    fields.insert("label".to_string(), field(CatalogueFieldType::String));

    let mut entry = BTreeMap::new();
    entry.insert("count".to_string(), Value::Integer(4));
    entry.insert("ratio".to_string(), Value::Float(1.5));
    entry.insert("heated".to_string(), Value::Boolean(true));
    entry.insert("label".to_string(), Value::String("euro".to_string()));

    let mut entries = BTreeMap::new();
    entries.insert("c1".to_string(), entry);
    Catalogue {
        fields,
        entries,
        doc: None,
    }
}

fn catalogues(pairs: Vec<(&str, Catalogue)>) -> HashMap<String, Catalogue> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Validate one chunk's link surface through the real entry point.
fn link(config: &Config) -> Result<()> {
    validate_link_summary(&[summarize(config, "chunk.json")])
}

fn binding(catalogue: &str) -> Binding {
    Binding {
        catalogue: catalogue.to_string(),
        default: None,
        derive: None,
        doc: None,
    }
}

fn derive_table(source: &str, pairs: &[(&str, &str)]) -> BTreeMap<String, BTreeMap<String, String>> {
    let inner: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut table = BTreeMap::new();
    table.insert(source.to_string(), inner);
    table
}

// --- catalogue shape ---------------------------------------------------------

#[test]
fn a_well_formed_catalogue_validates() {
    assert!(validate_catalogues(&catalogues(vec![("containers", typed_catalogue())])).is_ok());
}

#[test]
fn a_catalogue_with_no_fields_is_rejected() {
    let mut catalogue = typed_catalogue();
    catalogue.fields = BTreeMap::new();
    let err = validate_catalogues(&catalogues(vec![("containers", catalogue)])).unwrap_err();
    assert!(
        format!("{err}").contains("Catalogue 'containers' declares no fields"),
        "err: {err}"
    );
}

#[test]
fn a_catalogue_with_no_entries_is_rejected() {
    let mut catalogue = typed_catalogue();
    catalogue.entries = BTreeMap::new();
    let err = validate_catalogues(&catalogues(vec![("containers", catalogue)])).unwrap_err();
    assert!(
        format!("{err}").contains("declares no entries"),
        "err: {err}"
    );
}

#[test]
fn an_entry_missing_a_declared_field_is_rejected() {
    let mut catalogue = typed_catalogue();
    catalogue
        .entries
        .get_mut("c1")
        .unwrap()
        .remove("ratio");
    let err = validate_catalogues(&catalogues(vec![("containers", catalogue)])).unwrap_err();
    let message = format!("{err}");
    assert!(message.contains("entry 'c1'"), "err: {message}");
    assert!(message.contains("missing field 'ratio'"), "err: {message}");
}

#[test]
fn an_entry_carrying_an_undeclared_field_is_rejected() {
    let mut catalogue = typed_catalogue();
    catalogue
        .entries
        .get_mut("c1")
        .unwrap()
        .insert("depth_mm".to_string(), Value::Integer(1));
    let err = validate_catalogues(&catalogues(vec![("containers", catalogue)])).unwrap_err();
    assert!(
        format!("{err}").contains("undeclared field 'depth_mm'"),
        "err: {err}"
    );
}

#[test]
fn a_value_of_the_wrong_declared_type_is_rejected() {
    for (field_id, wrong) in [
        ("count", Value::String("four".to_string())),
        ("ratio", Value::Boolean(true)),
        ("heated", Value::Integer(1)),
        ("label", Value::Integer(7)),
    ] {
        let mut catalogue = typed_catalogue();
        catalogue
            .entries
            .get_mut("c1")
            .unwrap()
            .insert(field_id.to_string(), wrong);
        let err = validate_catalogues(&catalogues(vec![("containers", catalogue)])).unwrap_err();
        assert!(
            format!("{err}").contains(&format!("field '{field_id}' is not of the declared type")),
            "field {field_id} was accepted with the wrong type: {err}"
        );
    }
}

#[test]
fn an_integer_satisfies_a_float_field() {
    // A JSON `1000` for a millimetre dimension is the same number as `1000.0`,
    // and serde's untagged `Value` resolves it to `Integer` before validation
    // ever sees it. Rejecting it would make a legal authored table unusable.
    let mut catalogue = typed_catalogue();
    catalogue
        .entries
        .get_mut("c1")
        .unwrap()
        .insert("ratio".to_string(), Value::Integer(2));
    assert!(validate_catalogues(&catalogues(vec![("containers", catalogue)])).is_ok());
}

// --- binding links -----------------------------------------------------------

#[test]
fn a_binding_over_a_declared_catalogue_validates() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    cfg.bindings
        .insert("line_container".to_string(), binding("containers"));
    assert!(link(&cfg).is_ok());
}

#[test]
fn a_binding_naming_an_undeclared_catalogue_is_rejected() {
    let mut cfg = config();
    cfg.bindings
        .insert("line_container".to_string(), binding("containers"));
    let err = link(&cfg).unwrap_err();
    let message = format!("{err}");
    assert!(message.starts_with("Binding 'line_container'"), "err: {message}");
    assert!(
        message.contains("catalogue 'containers', which is not declared"),
        "err: {message}"
    );
}

#[test]
fn a_default_outside_the_catalogue_is_rejected() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    let mut b = binding("containers");
    b.default = Some("c9".to_string());
    cfg.bindings.insert("line_container".to_string(), b);
    let err = link(&cfg).unwrap_err();
    assert!(
        format!("{err}").contains("default 'c9' is not an entry of catalogue 'containers' [c1]"),
        "err: {err}"
    );
}

#[test]
fn default_and_derive_together_are_rejected() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    cfg.facets.insert(
        "site".to_string(),
        crate::schema::Facet {
            values: vec!["factory_a".to_string()],
            default: None,
            open: false,
            doc: None,
        },
    );
    let mut b = binding("containers");
    b.default = Some("c1".to_string());
    b.derive = Some(derive_table("site", &[("factory_a", "c1")]));
    cfg.bindings.insert("line_container".to_string(), b);
    let err = link(&cfg).unwrap_err();
    assert!(
        format!("{err}").contains("mutually exclusive"),
        "err: {err}"
    );
}

#[test]
fn a_derive_source_that_is_not_declared_is_rejected() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    let mut b = binding("containers");
    b.derive = Some(derive_table("site", &[("factory_a", "c1")]));
    cfg.bindings.insert("line_container".to_string(), b);
    let err = link(&cfg).unwrap_err();
    assert!(
        format!("{err}").contains("derives from 'site', which is not a declared facet or binding"),
        "err: {err}"
    );
}

#[test]
fn a_derive_key_outside_the_source_domain_is_rejected() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    cfg.facets.insert(
        "site".to_string(),
        crate::schema::Facet {
            values: vec!["factory_a".to_string()],
            default: None,
            open: false,
            doc: None,
        },
    );
    let mut b = binding("containers");
    b.derive = Some(derive_table("site", &[("factory_z", "c1")]));
    cfg.bindings.insert("line_container".to_string(), b);
    let err = link(&cfg).unwrap_err();
    assert!(
        format!("{err}")
            .contains("derive key 'factory_z' is not a declared value of 'site' [factory_a]"),
        "err: {err}"
    );
}

#[test]
fn a_derive_entry_outside_the_catalogue_is_rejected() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    cfg.facets.insert(
        "site".to_string(),
        crate::schema::Facet {
            values: vec!["factory_a".to_string()],
            default: None,
            open: false,
            doc: None,
        },
    );
    let mut b = binding("containers");
    b.derive = Some(derive_table("site", &[("factory_a", "c9")]));
    cfg.bindings.insert("line_container".to_string(), b);
    let err = link(&cfg).unwrap_err();
    assert!(
        format!("{err}").contains("derive entry 'c9' is not an entry of catalogue 'containers'"),
        "err: {err}"
    );
}

#[test]
fn a_partial_derive_table_is_accepted() {
    // ADR-0057 §D3: an uncovered source value implies nothing. A partial table
    // is the normal shape when only some sites pin a container.
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    cfg.facets.insert(
        "site".to_string(),
        crate::schema::Facet {
            values: vec!["factory_a".to_string(), "factory_b".to_string()],
            default: None,
            open: false,
            doc: None,
        },
    );
    let mut b = binding("containers");
    b.derive = Some(derive_table("site", &[("factory_a", "c1")]));
    cfg.bindings.insert("line_container".to_string(), b);
    assert!(link(&cfg).is_ok());
}

#[test]
fn a_derive_table_with_two_sources_is_rejected() {
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    let mut table = derive_table("site", &[("factory_a", "c1")]);
    table.insert("region".to_string(), BTreeMap::new());
    let mut b = binding("containers");
    b.derive = Some(table);
    cfg.bindings.insert("line_container".to_string(), b);
    let err = link(&cfg).unwrap_err();
    assert!(
        format!("{err}").contains("derives from 2 sources; exactly one source is supported"),
        "err: {err}"
    );
}

#[test]
fn a_binding_may_derive_from_another_binding() {
    // ADR-0057 §D3: "the source must be a declared facet or binding". A binding
    // is a facet, so this has to work or the sentence is false.
    let mut cfg = config();
    cfg.catalogues
        .insert("containers".to_string(), typed_catalogue());
    cfg.bindings
        .insert("line_container".to_string(), binding("containers"));
    let mut derived = binding("containers");
    derived.derive = Some(derive_table("line_container", &[("c1", "c1")]));
    cfg.bindings.insert("sorter_container".to_string(), derived);
    assert!(link(&cfg).is_ok());
}

// --- one declaring chunk -----------------------------------------------------

#[test]
fn a_catalogue_declared_by_two_chunks_is_rejected() {
    let mut a = config();
    a.catalogues
        .insert("containers".to_string(), typed_catalogue());
    let mut b = config();
    b.catalogues
        .insert("containers".to_string(), typed_catalogue());

    let err = validate_link_summary(&[summarize(&a, "one.json"), summarize(&b, "two.json")])
        .unwrap_err();
    let message = format!("{err}");
    assert!(message.starts_with("Catalogue 'containers'"), "err: {message}");
    assert!(
        message.contains("declared in more than one chunk: 'one.json' and 'two.json'"),
        "err: {message}"
    );
}

#[test]
fn a_binding_declared_by_two_chunks_is_rejected() {
    let mut a = config();
    a.catalogues
        .insert("containers".to_string(), typed_catalogue());
    a.bindings
        .insert("line_container".to_string(), binding("containers"));
    let mut b = config();
    b.bindings
        .insert("line_container".to_string(), binding("containers"));

    let err = validate_link_summary(&[summarize(&a, "one.json"), summarize(&b, "two.json")])
        .unwrap_err();
    assert!(
        format!("{err}").contains("Binding 'line_container' is declared in more than one chunk"),
        "err: {err}"
    );
}

#[test]
fn a_binding_named_like_a_facet_is_rejected_as_one_id_space() {
    let mut a = config();
    a.facets.insert(
        "line_container".to_string(),
        crate::schema::Facet {
            values: vec!["x".to_string()],
            default: None,
            open: false,
            doc: None,
        },
    );
    let mut b = config();
    b.catalogues
        .insert("containers".to_string(), typed_catalogue());
    b.bindings
        .insert("line_container".to_string(), binding("containers"));

    let err = validate_link_summary(&[summarize(&a, "one.json"), summarize(&b, "two.json")])
        .unwrap_err();
    let message = format!("{err}");
    assert!(message.starts_with("Binding 'line_container'"), "err: {message}");
    assert!(
        message.contains("shares the facet id space"),
        "err: {message}"
    );
    // Routed to the facet duplicate code, not the binding-invalid one.
    assert!(
        message.contains("declared in more than one chunk"),
        "err: {message}"
    );
}
