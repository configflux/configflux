// SPDX-License-Identifier: BUSL-1.1

//! Acceptance suite for content-canonical model identity (ADR-0056).
//!
//! The invariant these tests exist to witness: **no path string may enter any
//! hash preimage**. `model_hash` must be a function of model content alone, so
//! that two checkouts of one commit — or a CI runner and a laptop — agree about
//! the identity of the same model.
//!
//! ADR-0056 §Acceptance names the criteria; each test below is labelled with
//! the one it encodes. They compile through the real product API rather than
//! asserting over `IrIndexContent` directly, because the defect ADR-0056
//! records was invisible at the type level and visible only in observed
//! output: a comment claimed an ordering guarantee the code did not provide,
//! and nothing compiled the same content twice to find out.
//!
//! `CompileModelRequest` takes `source_id` and `inline_content` directly, so
//! the path-spelling cases are exercised with real path-shaped source ids and
//! no real filesystem — the compiler receives bare `--source` strings from the
//! CLI (`main.rs`) and never resolves them, so an inline manifest reproduces
//! the CLI's input faithfully.

use crate::ir::CMP_DEFAULT_MANIFEST_FILENAME;
use crate::loader_api::{open_model, OpenModelRequest, E_LOADER_INDEX_INVALID};
use crate::product_api::{
    compile_model, CompileModelRequest, CompileResult, OperationStatus, SourceManifestEntry,
    PRODUCT_SCHEMA_VERSION,
};
use crate::scenario_test_support::{unique_temp_dir, TempDirGuard};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

const DEFS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
const COMPONENTS: &str = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

/// An entity-free chunk: no definition, component, artifact, facet or
/// constraint. Two copies under different source ids are the only input that
/// can reach the duplicate-`chunk_hash` check — anything carrying an entity is
/// rejected one step earlier by `merge_partial`'s duplicate-id rules
/// (ADR-0056 §3, reason 3).
const ENTITY_FREE: &str = r#"{"package": "s1_water_pump", "version": "1.0.0"}"#;

fn manifest(entries: &[(&str, &str)]) -> Vec<SourceManifestEntry> {
    entries
        .iter()
        .map(|(source_id, content)| SourceManifestEntry {
            source_id: (*source_id).to_string(),
            inline_content: (*content).to_string(),
        })
        .collect()
}

fn compile(label: &str, entries: &[(&str, &str)]) -> (CompileResult, TempDirGuard) {
    let out_dir = unique_temp_dir("configflux-model-identity", label).expect("temp dir");
    let result = compile_model(CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest: manifest(entries),
        output_dir: Some(out_dir.path.to_string_lossy().into_owned()),
        cluster_size: None,
        budget: None,
        stamp_time: false,
    });
    (result, out_dir)
}

/// Compile and return the `model_hash`, asserting the compile succeeded.
fn model_hash(label: &str, entries: &[(&str, &str)]) -> String {
    let (result, _guard) = compile(label, entries);
    assert_eq!(
        result.status,
        OperationStatus::Ok,
        "[{label}] compile failed: {:?}",
        result.verify_report.diagnostics.diagnostics
    );
    result.model_hash
}

// --- A1: identity is invariant under path spelling ---------------------------

/// ADR-0056 A1. The same bytes under repo-relative, absolute, and
/// renamed-parent-directory source ids are one model and must have one
/// identity.
///
/// This is the criterion the configflux-d4y0 measurements falsified: the two
/// S1 smoke chunks gave `c03a52d6…` by repo-relative path and `9b421f34…` by
/// absolute path, for byte-identical content.
#[test]
fn model_hash_is_invariant_under_source_path_spelling() {
    let repo_relative = [
        ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
        ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];
    let absolute = [
        ("/home/builder/checkouts/configflux/compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
        ("/home/builder/checkouts/configflux/compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];
    let renamed_parent = [
        ("/home/builder/checkouts/configflux-review/compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
        ("/home/builder/checkouts/configflux-review/compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];

    let relative_hash = model_hash("relative", &repo_relative);
    let absolute_hash = model_hash("absolute", &absolute);
    let renamed_hash = model_hash("renamed", &renamed_parent);

    assert_eq!(
        relative_hash, absolute_hash,
        "model_hash changed between a repo-relative and an absolute spelling of \
         the same content; a source path has reached the identity preimage"
    );
    assert_eq!(
        relative_hash, renamed_hash,
        "model_hash changed when the parent directory was renamed; a source path \
         has reached the identity preimage"
    );
}

/// ADR-0056 §2. The preimage's chunk ordering must not be derivable from
/// `source_id` either.
///
/// This is the sharper half of A1 and the only test that isolates the ordering
/// decision from the field-removal decision. Both compiles carry the same two
/// chunks, but their source ids sort in opposite relative order, so an
/// implementation that removed `source_id` from the preimage while still
/// SORTING by it would emit the same two hashes in two different orders and
/// fail here — with the field-removal half already correct.
#[test]
fn model_hash_is_invariant_under_source_id_sort_order() {
    let ascending = [("a_defs.json", DEFS), ("b_components.json", COMPONENTS)];
    let descending = [("z_defs.json", DEFS), ("y_components.json", COMPONENTS)];

    assert_eq!(
        model_hash("ascending", &ascending),
        model_hash("descending", &descending),
        "model_hash changed when the source ids' sort order was inverted over \
         identical content; the preimage's chunk order still depends on source_id"
    );
}

// --- A2: identity is invariant under argument order --------------------------

/// ADR-0056 A2. Reordering `--source` arguments over one set of files must not
/// change the model's identity.
///
/// NOTE: this criterion already held before ADR-0056 landed. ADR-0056 §Context
/// states the preimage order is "`--source` argument order" because no sort
/// exists; a sort by `(source_id, chunk_hash)` did in fact exist in
/// `build_ir_index`, and since `source_id` is unique per chunk it already
/// normalized argument order away. The ADR's ordering DECISION still changes
/// behaviour — see the sort-order test above, which is where that change is
/// witnessed. This test is kept because A2 is a named criterion and the
/// guarantee must not silently regress.
#[test]
fn model_hash_is_invariant_under_source_argument_order() {
    let declared = [
        ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
        ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];
    let reversed = [
        ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
        ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
    ];

    assert_eq!(
        model_hash("declared", &declared),
        model_hash("reversed", &reversed),
        "model_hash changed when the --source arguments were reordered over the \
         same set of files"
    );
}

// --- A3: identity still discriminates ----------------------------------------

/// ADR-0056 A3. Excluding paths must not cost the hash its discriminating
/// power: a change to model CONTENT must still change `model_hash`.
#[test]
fn model_hash_changes_when_content_changes() {
    let baseline = [
        ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
        ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];

    // One edit inside the definitions chunk: the documented unit of a
    // parameter. The source ids are held identical to the baseline, so the
    // content change is the only variable.
    let edited_defs = DEFS.replace("\"unit\": \"lpm\"", "\"unit\": \"gpm\"");
    assert_ne!(edited_defs, DEFS, "fixture edit did not apply");
    let edited = [
        ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", edited_defs.as_str()),
        ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];

    assert_ne!(
        model_hash("baseline", &baseline),
        model_hash("edited", &edited),
        "model_hash did not change when model content changed; identity has \
         stopped discriminating between distinct models"
    );
}

// --- A6: duplicate chunk content is rejected at ingest -----------------------

/// ADR-0056 A6. Two chunks with one `chunk_hash` are rejected at ingest with a
/// diagnostic naming BOTH source ids.
///
/// The package cannot represent the duplicate — chunk storage is
/// content-addressed, so both chunks write one file and the loser's
/// `source_id` is lost. Rejecting at ingest is what lets the diagnostic name
/// the two sources; the index-level check one stage later
/// (`verify_index_integrity`) knows only the hash.
#[test]
fn duplicate_chunk_content_is_rejected_naming_both_sources() {
    let (result, _guard) = compile(
        "duplicate",
        &[
            ("configs/alpha.json", ENTITY_FREE),
            ("configs/beta.json", ENTITY_FREE),
        ],
    );

    assert_eq!(
        result.status,
        OperationStatus::Error,
        "two chunks with identical content compiled successfully; the duplicate \
         would collapse to one file on disk and lose a source_id"
    );

    let diagnostics = &result.verify_report.diagnostics.diagnostics;
    let message = diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        message.contains("configs/alpha.json") && message.contains("configs/beta.json"),
        "duplicate-chunk diagnostic must name both source ids, got: {message}"
    );
}

// --- A7: a pre-ADR-0056 package is rejected at the manifest gate -------------

fn read_json(path: &Path) -> JsonValue {
    serde_json::from_slice(&std::fs::read(path).expect("read json")).expect("parse json")
}

fn write_json(path: &Path, value: &JsonValue) {
    std::fs::write(path, serde_json::to_vec_pretty(value).expect("serialize json"))
        .expect("write json");
}

/// Emit a real CMP, then age it into the shape a pre-ADR-0056 toolchain would
/// have written: the index's chunk vector in the order the old
/// `source_id`-keyed sort produced, which under the new canonicalization no
/// longer reproduces the stored `config_hash`.
///
/// Reversing the vector is a faithful stand-in and needs no hand-computed
/// digest: the point of the fixture is that the index recompute WOULD reject
/// this package, so which of the two gates fires first is observable.
fn aged_package(label: &str, canonicalization_version: Option<u32>) -> (PathBuf, TempDirGuard) {
    let entries = [
        ("compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json", DEFS),
        ("compiler/scenarios/s1_water_pump/smoke/cue/10_components.json", COMPONENTS),
    ];
    let (result, guard) = compile(label, &entries);
    assert_eq!(result.status, OperationStatus::Ok, "[{label}] compile failed");

    let manifest_path = guard.path.join(CMP_DEFAULT_MANIFEST_FILENAME);
    let index_path = guard.path.join("index.cfir.json");

    let mut index = read_json(&index_path);
    let chunks = index["chunks"].as_array_mut().expect("index chunks array");
    assert_eq!(chunks.len(), 2, "fixture needs two chunks to reorder");
    chunks.reverse();
    write_json(&index_path, &index);

    if let Some(version) = canonicalization_version {
        let mut manifest = read_json(&manifest_path);
        manifest["canonicalization_version"] = JsonValue::from(version);
        write_json(&manifest_path, &manifest);
    }

    (manifest_path, guard)
}

/// ADR-0056 A7. A package canonicalized under rule v1 is rejected at the
/// MANIFEST gate, stating the true reason.
///
/// Without the `CMP_CANONICALIZATION_VERSION` bump such a package falls
/// through to the index recompute and is rejected as `E_LOADER_INDEX_INVALID`
/// with the hint "Do not mutate emitted index files; recompile instead" — a
/// tampering accusation against a package that is internally consistent and
/// was simply built by the previous toolchain.
#[test]
fn pre_canonicalization_v2_package_is_rejected_at_the_manifest_gate() {
    let (manifest_path, _guard) = aged_package("aged-v1", Some(1));

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    let diagnostics = &result.diagnostics.diagnostics;
    let message = diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        message.contains("canonicalization_version"),
        "a v1 package must be rejected for its canonicalization rule, got: {message}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == E_LOADER_INDEX_INVALID),
        "a v1 package must not be accused of index tampering, got: {diagnostics:?}"
    );
}

/// The control for the test above: hold `canonicalization_version` at the
/// current value and the SAME aged index reaches the recompute and is rejected
/// as a tampered index. This is what the manifest gate is preventing, and
/// without it this test would pass whether or not the gate ordering is right.
#[test]
fn aged_index_without_a_version_marker_is_rejected_as_index_invalid() {
    let (manifest_path, _guard) = aged_package("aged-current", None);

    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: manifest_path.to_string_lossy().into_owned(),
    });

    assert_eq!(result.status, OperationStatus::Error);
    assert!(
        result
            .diagnostics
            .diagnostics
            .iter()
            .any(|d| d.code == E_LOADER_INDEX_INVALID),
        "an index reordered under the CURRENT canonicalization rule must fail the \
         recompute, got: {:?}",
        result.diagnostics.diagnostics
    );
}
