// SPDX-License-Identifier: BUSL-1.1

//! configflux-3j2i: pins the `entry_id` worked example in
//! `docs/runtime-v2-contract.md` section 11.4 to the real serializer.
//!
//! Section 11.4 documents a *normative* byte layout and two normative content
//! addresses. Nothing executed them, so both digests silently rotted across the
//! product-schema rotations: the canonical payload injects
//! `PRODUCT_SCHEMA_VERSION` (see `compute_lineage_entry_content_address`), so
//! every bump invalidates the documented digests while leaving the doc green.
//!
//! The doc is the input here, not a copy of it: the payloads and digests are
//! extracted from `runtime-v2-contract.md` itself, so the example cannot be
//! rotated in code without the doc going red, or in the doc without the code
//! agreeing.
//!
//! `ProvenanceLineageEntryCanonical` is private, so the canonical bytes cannot
//! be compared directly. Two assertions pin them jointly without deriving any
//! hash by hand:
//!
//! 1. `compute_lineage_entry_content_address` reproduces the documented digest.
//! 2. The sha256 of the documented payload *bytes* is that same digest.
//!
//! Any divergence between the implementation's canonical bytes and the
//! documented byte layout makes those two disagree.

use compiler::product_api::PRODUCT_SCHEMA_VERSION;
use compiler::runtime_api::{
    compute_lineage_entry_content_address, OverrideIntent, ProvenanceVersionTriple,
};
use sha2::{Digest, Sha256};

/// The contract document, compiled in so the test binary carries the exact
/// bytes under review. Mirrors the `include_str!("../../VERSION")` idiom in
/// `provenance_sidecar.rs`; `compile_data` in the BUILD target supplies it.
const CONTRACT_DOC: &str = include_str!("../../docs/runtime-v2-contract.md");

const WORKED_EXAMPLE_HEADING: &str = "#### Worked example";

/// A fenced code block lifted from the document.
struct FencedBlock {
    info: String,
    body: String,
}

/// Return the text of section 11.4, from its heading to the next section.
///
/// Fails closed: a renamed or removed heading is a contract change that must
/// not silently disable the pinning.
fn section_11_4() -> &'static str {
    const HEADING: &str = "### 11.4 Content addressing";
    let start = CONTRACT_DOC
        .find(HEADING)
        .unwrap_or_else(|| panic!("runtime-v2-contract.md must still contain '{HEADING}'"));
    let rest = &CONTRACT_DOC[start + HEADING.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let end = rest[..end].find("\n### ").unwrap_or(end);
    &rest[..end]
}

/// Extract the fenced code blocks of the section's worked example, in order.
fn worked_example_blocks() -> Vec<FencedBlock> {
    let section = section_11_4();
    let start = section.find(WORKED_EXAMPLE_HEADING).unwrap_or_else(|| {
        panic!("section 11.4 must still contain '{WORKED_EXAMPLE_HEADING}'")
    });

    let mut blocks = Vec::new();
    let mut current: Option<FencedBlock> = None;
    for line in section[start..].lines() {
        match (line.strip_prefix("```"), current.as_mut()) {
            (Some(info), None) => {
                current = Some(FencedBlock {
                    info: info.trim().to_string(),
                    body: String::new(),
                })
            }
            (Some(_), Some(_)) => blocks.push(current.take().expect("fence is open")),
            (None, Some(block)) => {
                if !block.body.is_empty() {
                    block.body.push('\n');
                }
                block.body.push_str(line);
            }
            (None, None) => {}
        }
    }
    assert!(
        current.is_none(),
        "section 11.4 has an unterminated code fence"
    );
    blocks
}

/// The worked example's four blocks: root payload, root `entry_id`, child
/// payload, child `entry_id`.
fn worked_example() -> (String, String, String, String) {
    let blocks = worked_example_blocks();
    let shapes: Vec<&str> = blocks.iter().map(|block| block.info.as_str()).collect();
    assert_eq!(
        shapes,
        vec!["json", "", "json", ""],
        "section 11.4's worked example must stay four fenced blocks in order \
         (root payload, root entry_id, child payload, child entry_id); got {shapes:?}"
    );
    let mut bodies = blocks.into_iter().map(|block| block.body);
    (
        bodies.next().expect("root payload"),
        bodies.next().expect("root entry_id"),
        bodies.next().expect("child payload"),
        bodies.next().expect("child entry_id"),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The root entry of the documented example.
fn root_state() -> ProvenanceVersionTriple {
    ProvenanceVersionTriple {
        model_version: "m1".to_string(),
        selection_version: "s1".to_string(),
        override_layer: "o1".to_string(),
    }
}

/// The child entry's state: `model_version` pinned to `m2`, other axes held.
fn child_state() -> ProvenanceVersionTriple {
    ProvenanceVersionTriple {
        model_version: "m2".to_string(),
        selection_version: "s1".to_string(),
        override_layer: "o1".to_string(),
    }
}

fn root_entry_id() -> String {
    compute_lineage_entry_content_address(
        &root_state(),
        "operator-a",
        Some("initial state"),
        1_700_000_000_000,
        OverrideIntent::Experimental,
        None,
    )
    .expect("the documented root entry must be addressable")
}

#[test]
fn worked_example_root_entry_id_matches_the_implementation() {
    let (_, documented, _, _) = worked_example();
    assert_eq!(
        documented,
        root_entry_id(),
        "section 11.4's root entry_id must equal the address \
         compute_lineage_entry_content_address derives from the documented contents \
         (the canonical payload pins PRODUCT_SCHEMA_VERSION = {PRODUCT_SCHEMA_VERSION}, \
         so a schema rotation invalidates the documented digest)"
    );
}

#[test]
fn worked_example_child_entry_id_matches_the_implementation() {
    let (_, documented_root, _, documented_child) = worked_example();
    let computed = compute_lineage_entry_content_address(
        &child_state(),
        "operator-a",
        None,
        1_700_000_005_000,
        OverrideIntent::Compensating,
        Some(&documented_root),
    )
    .expect("the documented child entry must be addressable");
    assert_eq!(
        documented_child, computed,
        "section 11.4's child entry_id must equal the address derived from the \
         documented contents parented to the documented root entry_id"
    );
}

#[test]
fn worked_example_canonical_payloads_hash_to_the_documented_entry_ids() {
    let (root_payload, root_id, child_payload, child_id) = worked_example();

    assert!(
        child_payload.contains(&root_id),
        "section 11.4's child canonical payload must carry the root entry_id as its \
         parent_entry_id; rotating the root digest without rotating the child's parent \
         pointer leaves the chain inconsistent"
    );

    assert_eq!(
        sha256_hex(root_payload.as_bytes()),
        root_id,
        "the documented root canonical payload must hash to the documented root entry_id — \
         the doc states these bytes are the normative layout, so they must be the bytes hashed"
    );
    assert_eq!(
        sha256_hex(child_payload.as_bytes()),
        child_id,
        "the documented child canonical payload must hash to the documented child entry_id"
    );
}

#[test]
fn worked_example_pins_the_current_product_schema_version() {
    let (root_payload, _, child_payload, _) = worked_example();
    let expected_prefix = format!("{{\"schema_version\":{PRODUCT_SCHEMA_VERSION},");
    for (label, payload) in [("root", &root_payload), ("child", &child_payload)] {
        assert!(
            payload.starts_with(&expected_prefix),
            "section 11.4's {label} canonical payload must open with the current product \
             schema version ({expected_prefix}); the canonical payload injects \
             PRODUCT_SCHEMA_VERSION, so it rotates with the product schema"
        );
    }

    // The prose states the pinned value in words as well as in the payloads.
    const ANCHOR: &str = "set to the current product schema";
    let section = section_11_4();
    let anchor = section
        .find(ANCHOR)
        .unwrap_or_else(|| panic!("section 11.4 must still state '{ANCHOR}'"));
    const OPEN: &str = "(value `";
    let open = section[anchor..]
        .find(OPEN)
        .expect("the product-schema sentence must state its value as (value `N`)")
        + anchor
        + OPEN.len();
    let close = section[open..]
        .find('`')
        .expect("the stated product-schema value must be closed by a backtick")
        + open;
    assert_eq!(
        &section[open..close],
        PRODUCT_SCHEMA_VERSION.to_string(),
        "section 11.4's prose must state the current PRODUCT_SCHEMA_VERSION"
    );
}
