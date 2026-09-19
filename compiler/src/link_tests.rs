// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the header-only half of the link (ADR-0058 §D4 stage 1).
//!
//! These assert what is hard to see from the outside: that the merged summary
//! is built in the canonical order regardless of the order the objects arrive
//! in, and that each stage-1 rule names both sides of the fault it found. The
//! end-to-end behaviour of `link` — the byte-identity oracle included — is
//! asserted black-box in `compiler/tests/link_oracle.rs` and
//! `compiler/tests/link_diagnostics.rs`.

use super::*;
use crate::interface_summary::summarize;
use crate::ir::chunk_hash_from_config;
use crate::object::InterfaceRef;
use crate::schema::Config;

const CATALOGUE_UNIT: &str = r#"{
  "package": "cat_unit",
  "version": "1.0.0",
  "definitions": { "width_mm": { "type": "integer", "value": 10 } },
  "facets": { "site": { "values": ["a", "b"] } }
}"#;

const SERVICE_UNIT: &str = r#"{
  "package": "svc_unit",
  "version": "1.0.0",
  "components": {
    "svc": {
      "type": "service",
      "condition": "site == 'a'",
      "params": { "w": { "inherits": "width_mm", "type": "integer", "value": 1 } }
    }
  }
}"#;

/// A one-chunk object for `json`, with the interfaces it was compiled against.
fn header(json: &str, interfaces: Vec<InterfaceRef>) -> ObjectHeader {
    let config: Config = serde_json::from_str(json).expect("fixture parses");
    let hash = chunk_hash_from_config(&config).expect("hash");
    let summary = summarize(&config, "fixture");
    ObjectHeader::from_summaries(&config.package, vec![hash], &[summary], interfaces)
}

#[test]
fn objects_are_merged_by_unit_name_not_by_argument_order() {
    let cat = header(CATALOGUE_UNIT, Vec::new());
    let svc = header(SERVICE_UNIT, Vec::new());

    let forward = link_stage_headers(&[cat.clone(), svc.clone()]).expect("links");
    let reversed = link_stage_headers(&[svc, cat]).expect("links");

    assert_eq!(forward.selectors, reversed.selectors);
    assert_eq!(forward.clauses, reversed.clauses);
    assert_eq!(forward.facet_domains, reversed.facet_domains);
    assert_eq!(
        forward.units.iter().cloned().collect::<Vec<_>>(),
        vec!["cat_unit".to_string(), "svc_unit".to_string()]
    );
}

#[test]
fn two_objects_for_one_unit_are_refused_naming_the_unit() {
    let one = header(CATALOGUE_UNIT, Vec::new());
    // A second object for the same unit, differing in content so the two
    // hashes differ and the message can show both.
    let other = header(
        &CATALOGUE_UNIT.replace("\"value\": 10", "\"value\": 11"),
        Vec::new(),
    );

    let error = link_stage_headers(&[one, other]).expect_err("refused");
    assert_eq!(error.code, E_LINK_DUPLICATE_UNIT);
    assert!(
        error.message.contains("cat_unit"),
        "message must name the unit: {}",
        error.message
    );
}

#[test]
fn an_id_exported_by_two_units_names_both() {
    let cat = header(CATALOGUE_UNIT, Vec::new());
    let clash = header(
        &CATALOGUE_UNIT.replace("\"package\": \"cat_unit\"", "\"package\": \"other_unit\""),
        Vec::new(),
    );

    let error = link_stage_headers(&[cat, clash]).expect_err("refused");
    assert_eq!(error.code, E_LINK_DUPLICATE_ID);
    assert!(
        error.message.contains("cat_unit") && error.message.contains("other_unit"),
        "message must name both units: {}",
        error.message
    );
}

#[test]
fn an_import_nothing_declares_names_the_unit_and_the_id() {
    let error = link_stage_headers(&[header(SERVICE_UNIT, Vec::new())]).expect_err("refused");
    assert_eq!(error.code, E_LINK_UNRESOLVED_IMPORT);
    assert!(
        error.message.contains("svc_unit")
            && error.message.contains("width_mm")
            && error.message.contains("no linked object declares it"),
        "message must name the unit, the id and the verdict: {}",
        error.message
    );
}

#[test]
fn an_interface_hash_that_moved_is_refused_naming_both_hashes() {
    let compiled_against = header(CATALOGUE_UNIT, Vec::new());
    let linked = header(
        &CATALOGUE_UNIT.replace("\"value\": 10", "\"value\": 11"),
        Vec::new(),
    );
    let service = header(SERVICE_UNIT, vec![compiled_against.as_interface_ref()]);

    let error = link_stage_headers(&[linked.clone(), service]).expect_err("refused");
    assert_eq!(error.code, E_LINK_INTERFACE_MISMATCH);
    assert!(
        error.message.contains(&compiled_against.object_hash)
            && error.message.contains(&linked.object_hash),
        "message must name the hash compiled against and the one linked: {}",
        error.message
    );
}

#[test]
fn the_header_lowering_agrees_with_the_authored_one_conjunct_for_conjunct() {
    // ADR-0057 §D4's `derive` and `accepts` conjuncts are built twice — once
    // from the authored `Binding`/`Component` (the loader's path) and once from
    // the `BindingLink`/`RequirementLink` a header carries (the linker's). They
    // share every helper that produces a byte, so the only thing that could
    // drift is the WALK, and this is what would catch it. The fixture is the
    // shipped three-unit requirements pack, which exercises both halves: a
    // binding with a `derive` table and a component with an `accepts` list.
    let mut compiler = crate::Compiler::new();
    for (source_id, content) in [
        ("00_catalogue.json", include_str!("../scenarios/s_requires_accepts/00_catalogue.json")),
        ("10_bindings.json", include_str!("../scenarios/s_requires_accepts/10_bindings.json")),
        ("20_components.json", include_str!("../scenarios/s_requires_accepts/20_components.json")),
    ] {
        compiler.add_chunk_auto(source_id, content).expect("ingest");
    }
    compiler.link_and_verify().expect("model links");

    let summaries = compiler.interface_summaries();
    let merged = link_stage_headers(&in_memory_headers(compiler.source_chunks(), &summaries))
        .expect("links");
    let repo = compiler.get_repo();

    let from_authored = crate::lowering::lowered_root_conjuncts(&repo.bindings, &repo.components);
    let from_headers = crate::lowering::lowered_root_conjuncts_from_summary(
        &merged.binding_links,
        &merged.requirements,
    );
    assert!(
        !from_authored.is_empty(),
        "the fixture must lower something, or this proves nothing"
    );
    assert_eq!(from_authored, from_headers);
}

#[test]
fn an_interface_that_was_not_linked_is_left_to_the_import_rule() {
    // The service names an interface object that is absent entirely. That is
    // NOT a hash mismatch — there is no second hash to compare — so the fault
    // reported is the unresolved import, which names the id that is missing
    // rather than only the object.
    let absent = header(CATALOGUE_UNIT, Vec::new());
    let service = header(SERVICE_UNIT, vec![absent.as_interface_ref()]);

    let error = link_stage_headers(&[service]).expect_err("refused");
    assert_eq!(error.code, E_LINK_UNRESOLVED_IMPORT);
}
