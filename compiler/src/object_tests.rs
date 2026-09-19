// SPDX-License-Identifier: BUSL-1.1

//! Unit tests for the object header's merge and identity rules
//! (configflux-p0jz.1 / ADR-0058 §D2 + §A1).
//!
//! These assert the properties that are hard to see from the outside: which
//! chunk order the merge commits to, that the hash preimage excludes the hash,
//! and that no path can reach either. The end-to-end behaviour of
//! `compile-object` is asserted black-box in
//! `compiler/tests/compile_object.rs`.

use super::*;
use crate::interface_summary::summarize;
use crate::ir::chunk_hash_from_config;
use crate::schema::Config;

fn config(json: &str) -> Config {
    serde_json::from_str(json).expect("fixture parses")
}

/// A unit of two chunks, returned in `chunk_hash` ascending order together with
/// those hashes — the order [`ObjectHeader::from_summaries`] requires.
fn ordered_unit(chunks: &[(&str, &str)]) -> (Vec<String>, Vec<crate::interface_summary::InterfaceSummary>) {
    let mut rows: Vec<(String, crate::interface_summary::InterfaceSummary)> = chunks
        .iter()
        .map(|(source_id, json)| {
            let parsed = config(json);
            let hash = chunk_hash_from_config(&parsed).expect("hash");
            (hash, summarize(&parsed, source_id))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows.into_iter().unzip()
}

const DECLARING: &str = r#"{
  "package": "u",
  "version": "1.0.0",
  "facets": { "site": { "values": ["a", "b"] } },
  "constraints": { "beta_rule": { "condition": "site == 'a'" } }
}"#;

const USING: &str = r#"{
  "package": "u",
  "version": "1.0.0",
  "components": {
    "svc": {
      "type": "service",
      "depends_on": ["absent_service"],
      "params": {
        "p": { "inherits": "absent_definition", "type": "integer", "value": 1 }
      }
    }
  },
  "constraints": { "alpha_rule": { "condition": "site == 'b'" } }
}"#;

#[test]
fn merge_visits_chunks_in_chunk_hash_order_not_argument_order() {
    let (hashes, summaries) = ordered_unit(&[("first", DECLARING), ("second", USING)]);
    let header = ObjectHeader::from_summaries("u", hashes.clone(), &summaries, Vec::new());

    assert_eq!(header.chunk_hashes, hashes);
    assert!(
        header.chunk_hashes.windows(2).all(|pair| pair[0] < pair[1]),
        "chunk hashes ascend"
    );

    // Clauses are chunk order then in-chunk order, NOT globally id-sorted:
    // ADR-0058 §A2 fixes that order so the linker's constraint model is a
    // function of content rather than of how the sources were listed.
    let clause_ids: Vec<&str> = header.clauses.iter().map(|c| c.id.as_str()).collect();
    let declaring_first = hashes[0]
        == chunk_hash_from_config(&config(DECLARING)).expect("hash");
    let expected = if declaring_first {
        ["beta_rule", "alpha_rule"]
    } else {
        ["alpha_rule", "beta_rule"]
    };
    assert_eq!(clause_ids, expected);
}

#[test]
fn imports_drop_what_the_unit_declares_and_keep_what_it_does_not() {
    let (hashes, summaries) = ordered_unit(&[("first", DECLARING), ("second", USING)]);
    let header = ObjectHeader::from_summaries("u", hashes, &summaries, Vec::new());

    assert!(header.exports.facets.contains("site"));
    assert!(
        !header.imports.facets.contains("site"),
        "the unit declares `site`, so naming it is not an import"
    );
    assert!(header.imports.components.contains("absent_service"));
    assert!(header.imports.definitions.contains("absent_definition"));
}

#[test]
fn the_hash_preimage_excludes_the_hash_and_covers_the_interfaces() {
    let (hashes, summaries) = ordered_unit(&[("first", DECLARING)]);
    let bare = ObjectHeader::from_summaries("u", hashes.clone(), &summaries, Vec::new());
    let against = ObjectHeader::from_summaries(
        "u",
        hashes,
        &summaries,
        vec![InterfaceRef {
            unit: "other".to_string(),
            object_hash: "00".repeat(32),
        }],
    );

    assert_eq!(bare.compute_object_hash(), bare.object_hash);
    assert_ne!(
        bare.object_hash, against.object_hash,
        "what a unit was compiled against is part of its identity"
    );

    // Mutating the stored hash must not change what the header hashes TO,
    // which is the whole point of excluding it from the preimage.
    let mut tampered = bare.clone();
    tampered.object_hash = "ff".repeat(32);
    assert_eq!(tampered.compute_object_hash(), bare.object_hash);
}

#[test]
fn interfaces_are_sorted_and_deduplicated() {
    let (hashes, summaries) = ordered_unit(&[("first", DECLARING)]);
    let one = InterfaceRef {
        unit: "aaa".to_string(),
        object_hash: "11".repeat(32),
    };
    let two = InterfaceRef {
        unit: "zzz".to_string(),
        object_hash: "22".repeat(32),
    };
    let header = ObjectHeader::from_summaries(
        "u",
        hashes,
        &summaries,
        vec![two.clone(), one.clone(), two.clone()],
    );
    assert_eq!(header.interfaces, vec![one, two]);
}
