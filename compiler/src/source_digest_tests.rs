// SPDX-License-Identifier: BUSL-1.1

//! Acceptance suite for the source-manifest digest (ADR-0056 §9).
//!
//! Sibling of `model_identity_tests` (§Acceptance A1–A7), split from it because
//! §9 is a **second preimage on a different code path**: `hash_sources` runs
//! before parsing, over raw source text, and so cannot reuse the `chunk_hash`
//! ordering §1–§2 install on the compile path. Rewriting the `IrIndexContent`
//! preimage left this one untouched, which is why the two suites witness the
//! same invariant — **no path string may enter any hash preimage** — through
//! two different commands.
//!
//! Every assertion here reads **serialized output**, not a struct field. That
//! is deliberate: A8 is stated over `compiler inspect`'s JSON, and the rename
//! A10 pins is a wire-format change, so a test that named the Rust field would
//! stop witnessing the thing that can actually break for a consumer.

use crate::product_api::{
    inspect_model, verify_model, InspectModelRequest, InspectQuery, OperationStatus,
    SourceManifestEntry, VerifyModelRequest, PRODUCT_SCHEMA_VERSION,
};
use serde_json::Value as JsonValue;

const DEFS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const COMPONENTS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

/// The same two chunks under repo-relative and under absolute source ids. The
/// compiler receives bare `--source` strings from the CLI and never resolves
/// them, so an inline manifest reproduces both invocations faithfully.
const RELATIVE: [(&str, &str); 2] = [
    ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
    ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
];
const ABSOLUTE: [(&str, &str); 2] = [
    ("/srv/build/configflux/compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
    ("/srv/build/configflux/compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
];

fn manifest(entries: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    entries
        .iter()
        .map(|(source_id, content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        })
        .collect()
}

/// Run `inspect` and return its serialized envelope, asserting the run was
/// diagnostic-free. A8 is scoped to that case: `Diagnostic.source_id` carries
/// paths by design (ADR-0056 §5), so a run that emits diagnostics is expected
/// to differ between spellings and is not what these tests assert.
fn inspect_json(label: &str, entries: &[(&str, &str)], query: InspectQuery) -> String {
    let result = inspect_model(InspectModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(entries),
        query,
    });
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "[{label}] inspect failed: {:?}",
        result.diagnostics.diagnostics
    );
    assert_eq!(
        result.diagnostics.diagnostics.len(),
        0,
        "[{label}] inspect emitted diagnostics; A8 is scoped to a diagnostic-free run"
    );
    serde_json::to_string(&result).expect("serialize inspection result")
}

// --- A8: inspect output is invariant under path spelling ---------------------

/// ADR-0056 A8. Byte-identical content under two different path spellings must
/// produce byte-identical `inspect` output, `source_digest` included. This is
/// the falsifier for §9: it fails before the change and passes after.
#[test]
fn inspect_output_is_invariant_under_source_path_spelling() {
    let relative = inspect_json("relative", &RELATIVE, InspectQuery::Summary);
    let absolute = inspect_json("absolute", &ABSOLUTE, InspectQuery::Summary);

    assert_eq!(
        relative, absolute,
        "inspect output differs between repo-relative and absolute source ids; \
         a path string is reaching the source-manifest digest"
    );
}

/// The same guarantee under a query that returns an item, so the invariant is
/// not accidentally scoped to the summary envelope alone.
#[test]
fn inspect_item_output_is_invariant_under_source_path_spelling() {
    let query = || InspectQuery::Parameter {
        component_id: "thermal_control".to_string(),
        param_key: "control_driver".to_string(),
    };

    assert_eq!(
        inspect_json("relative", &RELATIVE, query()),
        inspect_json("absolute", &ABSOLUTE, query()),
        "inspect parameter output differs between path spellings"
    );
}

// --- A9: verify carries the same guarantee ----------------------------------

/// ADR-0056 A9. `VerifyReport.model_hash` keeps its name (§9.2 — `compile_model`
/// reuses the struct and assigns the real CMP identity to it), which leaves it
/// the only field still carrying the guarantee under that name. Warranted
/// despite A8 precisely because the name did not move.
#[test]
fn verify_model_hash_is_invariant_under_source_path_spelling() {
    let verify = |entries: &[(&str, &str)]| {
        let report = verify_model(VerifyModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest(entries),
        });
        assert_eq!(report.status, OperationStatus::Ok, "verify failed");
        report.model_hash
    };

    assert_eq!(
        verify(&RELATIVE),
        verify(&ABSOLUTE),
        "verify model_hash differs between repo-relative and absolute source ids"
    );
}

/// The source-manifest digest still discriminates: editing content moves it.
/// Without this, A8 and A9 would be satisfied by a constant.
#[test]
fn verify_model_hash_changes_when_content_changes() {
    let edited = COMPONENTS.replacen("thermal_control", "thermal_controls", 1);
    assert_ne!(edited, COMPONENTS, "edit did not change the chunk");

    let baseline = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(&RELATIVE),
    });
    let changed = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(&[(RELATIVE[0].0, DEFS), (RELATIVE[1].0, &edited)]),
    });

    assert_ne!(
        baseline.model_hash, changed.model_hash,
        "source-manifest digest did not move when a chunk's content changed"
    );
}

// --- A10: the rename is complete, not half-landed ----------------------------

fn inspect_envelope(query: InspectQuery) -> JsonValue {
    serde_json::from_str(&inspect_json("rename", &RELATIVE, query)).expect("parse envelope")
}

/// Every key in `value`, recursively, so a `model_hash` surviving inside a
/// nested item or the diagnostics report is caught too.
fn collect_keys(value: &JsonValue, out: &mut Vec<String>) {
    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                out.push(key.clone());
                collect_keys(child, out);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_keys(item, out);
            }
        }
        _ => {}
    }
}

/// ADR-0056 A10. No `model_hash` key anywhere in `inspect` output, and the
/// content-canonical digest is present under its new name.
#[test]
fn inspect_output_carries_source_digest_and_no_model_hash() {
    for query in [
        InspectQuery::Summary,
        InspectQuery::Component {
            component_id: "thermal_control".to_string(),
        },
        InspectQuery::ScopedStats {
            scope: "component:thermal_control".to_string(),
        },
    ] {
        let envelope = inspect_envelope(query);
        let mut keys = Vec::new();
        collect_keys(&envelope, &mut keys);

        assert!(
            !keys.iter().any(|k| k == "model_hash"),
            "inspect output still carries a model_hash key: {keys:?}"
        );
        assert!(
            envelope
                .get("source_digest")
                .and_then(JsonValue::as_str)
                .is_some_and(|d| d.len() == 64),
            "inspect output has no 64-char source_digest: {envelope}"
        );
    }
}

/// A10's other half: `compile`'s `model_hash` keeps its name. Asserted on the
/// shared `VerifyReport`, which `compile_model` populates with the real CMP
/// identity on the success path — renaming it there would make compile lie.
#[test]
fn verify_report_keeps_the_model_hash_name() {
    let report = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(&RELATIVE),
    });
    let envelope = serde_json::to_value(&report).expect("serialize verify report");

    assert!(
        envelope.get("model_hash").and_then(JsonValue::as_str).is_some(),
        "verify report lost its model_hash key: {envelope}"
    );
}
